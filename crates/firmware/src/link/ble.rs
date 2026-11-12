//! The BLE link: a serial-like service a computer or phone connects to.
//!
//! The board advertises as [`BLE_NAME`] and offers one GATT service with
//! the Nordic UART Service layout, which `xiao-api` (`link/ble.rs`)
//! looks for:
//!
//! - `6e400002-…` (write): the API writes commands, with response, in
//!   chunks of the ATT MTU less 3. The chunks are fed in order to one
//!   [`Decoder`](xiao_proto::frame::Decoder), so a frame may span writes.
//! - `6e400003-…` (notify): every [`Message`](xiao_proto::message::Message)
//!   of the link's `Outbox` goes out as one frame, split into
//!   notifications of at most the ATT MTU less 3. The API reads them as
//!   one byte stream.
//!
//! The stack is `trouble-host` on Nordic's `SoftDevice` Controller
//! (`nrf-sdc`) and its MPSL (`nrf-mpsl`). The factory `SoftDevice` in flash
//! below the application is never enabled.
//!
//! One central at a time. When it disconnects, `on_down(Link::Ble, session)` runs,
//! so main releases the sensors the BLE link held, what was still queued
//! for it is thrown away, and the board advertises again.
//!
//! Depends on: `link`.

use embassy_futures::select::{Either, select};
use embassy_nrf::Peri;
use embassy_nrf::peripherals::RNG;
use embassy_nrf::rng::Rng;
use embassy_time::{Duration, Timer};
use nrf_mpsl::MultiprotocolServiceLayer;
use nrf_sdc::{self as sdc, SoftdeviceController};
use trouble_host::prelude::*;
use xiao_proto::frame::{Decoder, MAX_FRAME};
use xiao_proto::message::{BLE_NAME, Link};

use super::{Inbox, Outbox, Session, encode, receive};
use crate::gate::Gate;

/// One central at a time: a second link over BLE is not a use case.
const CONNECTIONS: usize = 1;

/// L2CAP channels: the signalling channel and the ATT channel.
const L2CAP_CHANNELS: usize = 2;

/// The link-layer packet size the controller buffers: the default
/// packet pool's MTU, so one ATT packet of 247 bytes (244 bytes of data
/// plus 3 of header) fits one link-layer packet with data length
/// extension.
const LL_PACKET: u16 = 251;

/// Link-layer packets the controller buffers each way.
// DECISION: 3 TX packets, enough to fill a connection event with an
// audio frame (~200 B) and a reply; 2 RX packets, commands are rare.
const LL_TX_COUNT: u8 = 3;
const LL_RX_COUNT: u8 = 2;

/// Most data in one ATT write or notification: the default packet
/// pool's 251 bytes, less 4 of L2CAP header and 3 of ATT header.
const ATT_PAYLOAD: usize = 244;

/// The controller's memory.
// DECISION: 8 KiB, well above what one peripheral link with these
// buffers and one advertising set need (~4 KiB by the `sdc.h` formulas).
// The SDC checks it at `build` and fails if it is too small; too big
// only wastes RAM, of which there are 256 KiB.
const SDC_MEM: usize = 8192;

/// How long to wait before trying again when advertising fails.
const RETRY: Duration = Duration::from_secs(1);

/// The Nordic UART Service as advertised: its 128-bit UUID, little-endian.
const NUS_UUID: [u8; 16] = 0x6e40_0001_b5a3_f393_e0a9_e50e_24dc_ca9e_u128.to_le_bytes();

/// The GATT server: the GAP service and the Nordic UART Service.
#[gatt_server(connections_max = CONNECTIONS)]
struct Server {
    nus: NordicUart,
}

/// Nordic UART Service: commands in, messages out.
#[gatt_service(uuid = "6e400001-b5a3-f393-e0a9-e50e24dcca9e")]
struct NordicUart {
    /// Written by the API: bytes of command frames.
    #[characteristic(uuid = "6e400002-b5a3-f393-e0a9-e50e24dcca9e", write)]
    rx: heapless::Vec<u8, ATT_PAYLOAD>,
    /// Notified to the API: bytes of message frames.
    #[characteristic(uuid = "6e400003-b5a3-f393-e0a9-e50e24dcca9e", notify)]
    tx: heapless::Vec<u8, ATT_PAYLOAD>,
}

/// What the BLE link runs on.
pub struct Radio {
    /// The MPSL, started by `main`, which also holds the crystal with it.
    pub mpsl: &'static MultiprotocolServiceLayer<'static>,
    /// The PPI channels the controller takes.
    pub sdc: sdc::Peripherals<'static>,
    /// The controller's random numbers.
    pub rng: Peri<'static, RNG>,
}

/// Runs the BLE link forever: starts the controller on `radio`, advertises
/// as [`BLE_NAME`], posts the commands written to it to `inbox` and
/// notifies `outbox` to the central. Calls `on_down(Link::Ble, session)`
/// each time the central disconnects, with the [`Session`] the commands
/// of the next central carry.
///
/// If the controller cannot start, the BLE link stays off and this
/// future waits forever: the other links keep working.
pub async fn run(
    radio: Radio,
    inbox: &Inbox,
    outbox: &Outbox,
    mut on_down: impl FnMut(Link, Session),
) -> ! {
    // DECISION: the blocking RNG driver. The controller pulls random
    // numbers synchronously through `RngCore`, so the async driver's
    // interrupt would never be used, and `board::Irqs` needs no RNG line.
    let mut rng = Rng::new_blocking(radio.rng);
    let mut mem = sdc::Mem::<SDC_MEM>::new();
    let Ok(controller) = controller(radio.sdc, &mut rng, radio.mpsl, &mut mem) else {
        never().await
    };
    let Ok(server) = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: BLE_NAME,
        appearance: &appearance::sensor::GENERIC_SENSOR,
    })) else {
        never().await
    };

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS, L2CAP_CHANNELS> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(static_address())
        .build();
    let mut runner = stack.runner();
    let mut peripheral = stack.peripheral();

    let mut number: Session = 0;
    let serve = async {
        loop {
            let Some(conn) = connect(&mut peripheral, &server).await else {
                Timer::after(RETRY).await;
                continue;
            };
            session(&server, &conn, number, inbox, outbox).await;
            drop(conn);
            number = number.wrapping_add(1);
            on_down(Link::Ble, number);
            // DECISION: as on USB, what was queued for the dead link is
            // stale (its sensors are being released); a new central
            // starts clean.
            outbox.clear();
        }
    };

    // The host runner only returns on a controller error. The BLE link
    // is then dead for good: free what it held and stay off.
    if let Either::Second(never) = select(runner.run(), serve).await {
        return never;
    }
    on_down(Link::Ble, number.wrapping_add(1));
    outbox.clear();
    never().await
}

/// Waits forever.
async fn never() -> ! {
    loop {
        Timer::after_secs(3600).await;
    }
}

/// Starts the `SoftDevice` Controller as one BLE peripheral.
fn controller<'d, const N: usize>(
    p: sdc::Peripherals<'d>,
    rng: &'d mut Rng<'static, embassy_nrf::mode::Blocking>,
    mpsl: &'d MultiprotocolServiceLayer<'_>,
    mem: &'d mut sdc::Mem<N>,
) -> Result<SoftdeviceController<'d>, sdc::Error> {
    // DECISION: data length extension and the 2M PHY are offered so a
    // central that supports them can carry one 247-byte ATT packet per
    // link-layer packet at twice the rate; the central decides.
    sdc::Builder::new()?
        .support_adv()
        .support_peripheral()
        .support_dle_peripheral()
        .support_le_2m_phy()
        .support_phy_update_peripheral()
        .peripheral_count(1)?
        .buffer_cfg(LL_PACKET, LL_PACKET, LL_TX_COUNT, LL_RX_COUNT)?
        .build(p, rng, mpsl, mem)
}

/// The board's random static address, from the chip's factory device
/// address.
// DECISION: FICR DEVICEADDR with the two top bits set (random static),
// what Nordic's own stacks use: unique per chip and the same at every
// boot, so a computer that paired or cached the board keeps knowing it.
// DECISION: read through `embassy_nrf::pac`, which exists because
// nrf-mpsl and nrf-sdc turn on embassy-nrf's `unstable-pac`; no feature
// of ours. FICR is read-only factory data, so the read is safe.
fn static_address() -> Address {
    let ficr = embassy_nrf::pac::FICR;
    let low = ficr.deviceaddr(0).read().to_le_bytes();
    let high = ficr.deviceaddr(1).read().to_le_bytes();
    Address::random([low[0], low[1], low[2], low[3], high[0], high[1] | 0xC0])
}

/// Advertises until a central connects, and serves the GATT server on
/// that connection. `None` when advertising failed.
async fn connect<'s>(
    peripheral: &mut Peripheral<'s, SoftdeviceController<'_>, DefaultPacketPool>,
    server: &'s Server<'_>,
) -> Option<GattConnection<'s, 's, DefaultPacketPool>> {
    // The name goes in the advertising packet itself, so even a passive
    // scan finds it; the service UUID goes in the scan response.
    let mut adv_data = [0; 31];
    let adv_len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteLocalName(BLE_NAME.as_bytes()),
        ],
        &mut adv_data,
    )
    .ok()?;
    let mut scan_data = [0; 31];
    let scan_len = AdStructure::encode_slice(
        &[AdStructure::CompleteServiceUuids128(&[NUS_UUID])],
        &mut scan_data,
    )
    .ok()?;
    let advertiser = peripheral
        .advertise(
            &AdvertisementParameters::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data: adv_data.get(..adv_len)?,
                scan_data: scan_data.get(..scan_len)?,
            },
        )
        .await
        .ok()?;
    let conn = advertiser.accept().await.ok()?;
    conn.with_attribute_server(server).ok()
}

/// One connection, until the central disconnects.
async fn session(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    number: Session,
    inbox: &Inbox,
    outbox: &Outbox,
) {
    // Ends when the central disconnects; the writer never ends by itself.
    match select(read(server, conn, number, inbox), write(server, conn, outbox)).await {
        Either::First(()) => {}
        Either::Second(never) => never,
    }
}

/// Feeds every write to the command characteristic to a fresh decoder
/// and lets the server answer everything else, until the central
/// disconnects.
async fn read(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    session: Session,
    inbox: &Inbox,
) {
    let mut decoder = Decoder::new();
    loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { .. } => return,
            GattConnectionEvent::Gatt { event: GattEvent::Write(write) }
                if write.handle() == server.nus.rx.handle =>
            {
                let mut bytes = [0; ATT_PAYLOAD];
                let n = write.with_data(|_, data| {
                    let n = data.len().min(bytes.len());
                    bytes[..n].copy_from_slice(&data[..n]);
                    n
                });
                // DECISION: the write is answered only after its commands
                // are in the inbox. The API waits for each answer before
                // the next chunk, so a full inbox slows the API down
                // instead of losing commands.
                let bytes = bytes.get(..n).unwrap_or(&[]);
                receive(Link::Ble, session, &mut decoder, bytes, inbox).await;
                // DECISION: answered without storing the value: the
                // characteristic is a pipe, nobody reads it back.
                if let Ok(reply) = write.accept_unprocessed() {
                    reply.send().await;
                }
            }
            // Everything else (the CCCD write that subscribes, MTU
            // exchange, discovery) is answered by the server when the
            // event is dropped.
            _ => {}
        }
    }
}

/// Notifies every message of `outbox` as one frame. Never returns; the
/// session ends it when the central disconnects.
async fn write(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    outbox: &Outbox,
) -> ! {
    let mut buf = [0; MAX_FRAME];
    let mut gate = Gate::new();
    loop {
        let message = outbox.receive().await;
        // A reading the closed session's sensor queued after the clear.
        if !gate.pass(&message) {
            continue;
        }
        let Some(bytes) = encode(&message, &mut buf) else { continue };
        // An ATT notification carries the MTU less 3 bytes of header.
        let chunk = usize::from(conn.raw().att_mtu().saturating_sub(3)).max(20);
        for part in bytes.chunks(chunk) {
            if !notify(server, conn, part).await {
                // DECISION: a frame cut short is closed with a lone zero,
                // so the API's decoder drops only this frame and not the
                // next one glued to it (PROTO-3).
                let _ = notify(server, conn, &[0]).await;
                break;
            }
        }
    }
}

/// Notifies `part` on the message characteristic. `false` when it could
/// not be sent.
async fn notify(server: &Server<'_>, conn: &GattConnection<'_, '_, DefaultPacketPool>, part: &[u8]) -> bool {
    loop {
        match server.nus.tx.notify_raw(conn, part, false).await {
            Ok(()) => return true,
            // DECISION: every packet of the pool is queued for the radio:
            // wait for one to free up instead of dropping the frame. A
            // reading that waits here backs up the `Outbox`, where the
            // sensor task drops readings and its counter shows the gap.
            Err(Error::OutOfMemory) => Timer::after_millis(1).await,
            Err(_) => return false,
        }
    }
}
