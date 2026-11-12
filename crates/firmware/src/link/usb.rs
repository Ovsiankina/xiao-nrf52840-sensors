//! The USB link: a serial port over the board's USB plug.
//!
//! One CDC-ACM interface. The host finds the board by Seeed's vendor id
//! 0x2886 (`xiao-api`, `link/usb.rs`) and opens it as a serial port; the
//! baud the host sets is ignored.
//!
//! The link is down when the bus is reset, suspended or unplugged (the
//! endpoints turn disabled) and when the host closes the port (DTR
//! falls). Either way `on_down(Link::Usb, session)` runs, so main releases the
//! sensors the USB link held, and the messages still queued for it are
//! thrown away.
//!
//! Depends on: `link`, `board`.

use embassy_futures::select::{Either, Either3, select, select3};
use embassy_nrf::Peri;
use embassy_nrf::peripherals::USBD;
use embassy_nrf::usb::Driver;
use embassy_nrf::usb::vbus_detect::HardwareVbusDetect;
use embassy_usb::class::cdc_acm::{CdcAcmClass, ControlChanged, Receiver, Sender, State};
use embassy_usb::driver::{Driver as UsbDriver, EndpointError};
use embassy_usb::{Builder, Config};
use xiao_proto::frame::{Decoder, MAX_FRAME};
use xiao_proto::message::Link;

use super::{Inbox, Outbox, Session, encode, receive};
use crate::board::Irqs;
use crate::gate::Gate;

/// Seeed Studio's USB vendor id; `xiao-api` looks for it.
const VENDOR_ID: u16 = 0x2886;

// DECISION: 0x8045, the product id Seeed's own application firmware
// reports for the XIAO nRF52840 Sense (its UF2 bootloader is 0x0045, in
// the research file). Not checked against a board; the API matches the
// vendor id only, so a wrong guess changes nothing it relies on.
/// The product id.
const PRODUCT_ID: u16 = 0x8045;

/// Bulk packet size: the most a full-speed bulk endpoint carries.
const MAX_PACKET: u16 = 64;

/// Runs the USB link forever: enumerates as a CDC-ACM serial port
/// (vendor id 0x2886, product "XIAO-Sense"), posts the commands it
/// receives to `inbox` and writes `outbox` to the host. Calls
/// `on_down(Link::Usb, session)` each time the link goes down, with the
/// [`Session`] the commands of the next connection carry.
// DECISION: VBUS is watched by the POWER peripheral
// (`HardwareVbusDetect`, bound on `CLOCK_POWER` in `board::Irqs`), so
// unplugging is seen at once. With the MPSL (fw-06) the same line also
// gets the MPSL clock handler; both handlers run on each interrupt.
pub async fn run(
    usb: Peri<'static, USBD>,
    inbox: &Inbox,
    outbox: &Outbox,
    mut on_down: impl FnMut(Link, Session),
) -> ! {
    let driver = Driver::new(usb, Irqs, HardwareVbusDetect::new(Irqs));

    let mut config = Config::new(VENDOR_ID, PRODUCT_ID);
    config.manufacturer = Some("Seeed Studio");
    config.product = Some("XIAO-Sense");
    // DECISION: 100 mA (the default) covers the chip, both sensors and
    // the radio, with room to spare.
    config.max_power = 100;

    // These live as long as this future, which never ends: the task that
    // awaits `run` owns them in its static storage.
    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut control_buf = [0; 64];
    let mut state = State::new();

    let mut builder = Builder::new(
        driver,
        config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut [], // no Microsoft OS descriptors
        &mut control_buf,
    );
    let class = CdcAcmClass::new(&mut builder, &mut state, MAX_PACKET);
    let mut device = builder.build();
    let (mut tx, mut rx, control) = class.split_with_control();

    let serve = async {
        let mut number: Session = 0;
        loop {
            // Configured by the host: the bulk endpoints are enabled.
            rx.wait_connection().await;
            session(&mut tx, &mut rx, &control, number, inbox, outbox).await;
            number = number.wrapping_add(1);
            on_down(Link::Usb, number);
            // DECISION: what was queued for the dead link is stale (its
            // sensors are being released); a new session starts clean.
            outbox.clear();
        }
    };

    match select(device.run(), serve).await {
        Either::First(never) | Either::Second(never) => never,
    }
}

/// One connection, from configuration until the link goes down.
#[expect(
    clippy::too_many_arguments,
    reason = "the session's three ends and where its bytes go; a struct would only rename them"
)]
async fn session<'d, D: UsbDriver<'d>>(
    tx: &mut Sender<'d, D>,
    rx: &mut Receiver<'d, D>,
    control: &ControlChanged<'d>,
    number: Session,
    inbox: &Inbox,
    outbox: &Outbox,
) {
    // Each of the three ends the session; whichever ends first wins.
    match select3(read(rx, number, inbox), write(tx, outbox), dtr_falls(control)).await {
        Either3::First(()) | Either3::Second(()) | Either3::Third(()) => {}
    }
}

/// Feeds every received byte to a fresh decoder until the endpoint is
/// disabled.
async fn read<'d, D: UsbDriver<'d>>(rx: &mut Receiver<'d, D>, session: Session, inbox: &Inbox) {
    let mut decoder = Decoder::new();
    let mut packet = [0; MAX_PACKET as usize];
    loop {
        match rx.read_packet(&mut packet).await {
            Ok(n) => {
                let bytes = packet.get(..n).unwrap_or(&[]);
                receive(Link::Usb, session, &mut decoder, bytes, inbox).await;
            }
            // Cannot happen: the buffer is one max-size packet.
            Err(EndpointError::BufferOverflow) => {}
            Err(EndpointError::Disabled) => return,
        }
    }
}

/// Writes every message of `outbox` as one frame until the endpoint is
/// disabled.
async fn write<'d, D: UsbDriver<'d>>(tx: &mut Sender<'d, D>, outbox: &Outbox) {
    // DECISION: every session starts with a lone zero byte. If the host
    // closed the port while a packet sat in the IN endpoint, the next
    // opener reads that stale half frame first; the zero ends it, so the
    // decoder throws it away instead of gluing it to the first real
    // frame (most likely the reply the host waits for). The API skips a
    // lone zero (`proto::frame::Decoder`).
    if write_transfer(tx, &[0]).await.is_err() {
        return;
    }
    let mut buf = [0; MAX_FRAME];
    let mut gate = Gate::new();
    loop {
        let message = outbox.receive().await;
        // A reading the closed session's sensor queued after the clear.
        if !gate.pass(&message) {
            continue;
        }
        let Some(bytes) = encode(&message, &mut buf) else { continue };
        if write_transfer(tx, bytes).await.is_err() {
            return;
        }
    }
}

/// Sends `bytes` as one USB transfer: max-size packets, then a short
/// one. When the last packet is exactly max size, a zero-length packet
/// follows, or the host would wait for more data before handing the
/// transfer to the reader.
// DECISION: one transfer per frame, not frames packed into shared
// packets: an audio frame (~200 B every 6 ms, ~34 kB/s) is 4 packets,
// far under the ~1 MB/s a full-speed bulk endpoint carries.
async fn write_transfer<'d, D: UsbDriver<'d>>(
    tx: &mut Sender<'d, D>,
    bytes: &[u8],
) -> Result<(), EndpointError> {
    let max = usize::from(MAX_PACKET);
    for packet in bytes.chunks(max) {
        tx.write_packet(packet).await?;
    }
    if bytes.len().is_multiple_of(max) {
        tx.write_packet(&[]).await?;
    }
    Ok(())
}

/// Returns when the host lowers DTR, the sign it closed the port.
// DECISION: only a fall from high to low counts. DTR low on its own is
// not "down": a host that never raises DTR on open still gets a working
// link, and its sensors are freed when the bus goes down.
async fn dtr_falls(control: &ControlChanged<'_>) {
    let mut was_high = control.dtr();
    loop {
        control.control_changed().await;
        let high = control.dtr();
        if was_high && !high {
            return;
        }
        was_high = high;
    }
}
