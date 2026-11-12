//! The UART link: the board's TX/RX pins, 921600 baud by default (FW-3).
//!
//! A UART has no connection to lose: without a DTR line or a cable
//! detect there is nothing that says the other end went away, so this
//! link never reports itself down. Its sensors are freed by the
//! commands that turn them off.
//!
//! Depends on: `link`, `board`.

use embassy_futures::select::{Either, select};
use embassy_nrf::buffered_uarte::{BufferedUarte, BufferedUarteRx, BufferedUarteTx};
use embassy_nrf::uarte::{self, Baudrate};
use xiao_proto::frame::{Decoder, MAX_FRAME};
use xiao_proto::message::{Link, UART_DEFAULT_BAUD};

use super::{Inbox, Outbox, encode, receive};
use crate::board::{Irqs, UartPins};

/// Runs the UART link forever at `baud` bits per second: posts the
/// commands it receives to `inbox` and writes `outbox` to TX.
///
/// `baud` must be one the UARTE supports (1200 … 1 000 000; the
/// standard rates); any other runs at [`UART_DEFAULT_BAUD`].
// DECISION: `BufferedUarte` (RX into a ring buffer, counted by TIMER1
// over PPI), not `Uarte::read`: at 921600 baud a byte lands every
// ~11 µs, and a plain read re-armed per byte could lose bytes between
// two calls. Costs TIMER1, two PPI channels and a PPI group.
pub async fn run(pins: UartPins, baud: u32, inbox: &Inbox, outbox: &Outbox) -> ! {
    let mut config = uarte::Config::default();
    config.baudrate = baudrate(baud);

    // These live as long as this future, which never ends: the task that
    // awaits `run` owns them in its static storage.
    // DECISION: 64 B in (commands are under 20 B each), 512 B out (two
    // audio frames, ~4.5 ms at 921600 baud).
    let mut rx_buffer = [0; 64];
    let mut tx_buffer = [0; 512];

    let uart = BufferedUarte::new(
        pins.uarte,
        pins.timer,
        pins.ppi_ch1,
        pins.ppi_ch2,
        pins.ppi_group,
        pins.rx,
        pins.tx,
        Irqs,
        config,
        &mut rx_buffer,
        &mut tx_buffer,
    );
    pull_up_rx();
    let (mut rx, mut tx) = uart.split();

    match select(read(&mut rx, inbox), write(&mut tx, outbox)).await {
        Either::First(never) | Either::Second(never) => never,
    }
}

/// Pulls the RX pin (P1.12, D7) up to the idle level of a UART line.
// DECISION: the UARTE driver leaves RX without a pull, and the receiver
// is always on: with nothing wired to D7 (board used over USB or BLE) a
// floating pin would feed noise to the decoder, and a spurious command
// would hold a sensor for the UART link, which never goes down. The
// pull is set through `embassy_nrf::pac` (as `link::ble` reads FICR)
// after the driver configured the pin; only the PULL field changes.
// An adapter driving the line overrides the ~13 kΩ pull-up.
fn pull_up_rx() {
    use embassy_nrf::pac::{P1, gpio::vals::Pull};
    P1.pin_cnf(12).modify(|w| w.set_pull(Pull::Pullup));
}

/// Feeds every received byte to the decoder, forever.
async fn read(rx: &mut BufferedUarteRx<'_>, inbox: &Inbox) -> ! {
    let mut decoder = Decoder::new();
    let mut bytes = [0; 32];
    loop {
        // DECISION: a receive error (overrun, framing) is skipped: the
        // damaged frame fails to decode and the decoder resyncs at the
        // next zero (PROTO-3).
        if let Ok(n) = rx.read(&mut bytes).await {
            // The UART link is never down: one session, 0.
            receive(Link::Uart, 0, &mut decoder, bytes.get(..n).unwrap_or(&[]), inbox).await;
        }
    }
}

/// Writes every message of `outbox` as one frame, forever.
async fn write(tx: &mut BufferedUarteTx<'_>, outbox: &Outbox) -> ! {
    let mut buf = [0; MAX_FRAME];
    loop {
        let message = outbox.receive().await;
        let Some(mut bytes) = encode(&message, &mut buf) else { continue };
        while !bytes.is_empty() {
            match tx.write(bytes).await {
                Ok(n) => bytes = bytes.get(n..).unwrap_or(&[]),
                // DECISION: on a write error the rest of this frame is
                // dropped; the next frame starts after a zero anyway.
                Err(_) => break,
            }
        }
    }
}

/// The UARTE setting for `baud`.
// DECISION: an unsupported rate falls back to the default instead of
// panicking (`panic-halt` would stop every link); main passes a
// supported constant.
fn baudrate(baud: u32) -> Baudrate {
    match baud {
        1_200 => Baudrate::Baud1200,
        2_400 => Baudrate::Baud2400,
        4_800 => Baudrate::Baud4800,
        9_600 => Baudrate::Baud9600,
        14_400 => Baudrate::Baud14400,
        19_200 => Baudrate::Baud19200,
        28_800 => Baudrate::Baud28800,
        31_250 => Baudrate::Baud31250,
        38_400 => Baudrate::Baud38400,
        56_000 => Baudrate::Baud56000,
        57_600 => Baudrate::Baud57600,
        76_800 => Baudrate::Baud76800,
        115_200 => Baudrate::Baud115200,
        230_400 => Baudrate::Baud230400,
        250_000 => Baudrate::Baud250000,
        460_800 => Baudrate::Baud460800,
        1_000_000 => Baudrate::Baud1m,
        _ => {
            const _: () = assert!(UART_DEFAULT_BAUD == 921_600);
            Baudrate::Baud921600
        }
    }
}
