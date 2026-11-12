//! The three ways in: USB, BLE, UART. Each turns incoming bytes into
//! commands and outgoing messages into bytes.
//!
//! Every link task feeds the bytes it receives to a
//! [`Decoder`](xiao_proto::frame::Decoder) and posts each command, with
//! the link it came from, to the one [`Inbox`]. It writes every
//! [`Message`] of its own [`Outbox`] as one frame.
//!
//! Depends on: `xiao-proto::frame`.

pub mod ble;
pub mod uart;
pub mod usb;

use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::channel::Channel;
use xiao_proto::frame::{self, Decoder, MAX_FRAME};
use xiao_proto::message::{Command, Link, Message};

// DECISION: `ThreadModeRawMutex`, not `CriticalSectionRawMutex`: every
// task runs on the one thread-mode executor, and this lock never masks
// interrupts, so it cannot delay the MPSL's radio interrupts (fw-06).
/// Commands from every link to the main task, tagged with their link and
/// the link's [`Session`].
pub type Inbox = Channel<ThreadModeRawMutex, (Link, Session, Command), 4>;

/// Which connection of a link a command came from. A link task bumps it
/// each time the link goes down and passes the new value to `on_down`,
/// so main can tell a closed session's queued commands from the new
/// session's. The UART link, never down, stays at 0.
pub type Session = u32;

/// Messages for one link: replies from the main task and readings from
/// the sensor tasks. Sensor tasks use `try_send` (full = dropped, the
/// counter shows the gap); one `Outbox` per link.
pub type Outbox = Channel<ThreadModeRawMutex, Message, 8>;

/// Feeds `bytes` from `link`'s `session` to `decoder` and posts every
/// whole command to `inbox`, waiting when it is full.
// DECISION: a frame that does not decode is dropped without a word: the
// API times out on its request, and the decoder resyncs at the next zero
// (PROTO-3).
pub async fn receive(
    link: Link,
    session: Session,
    decoder: &mut Decoder,
    bytes: &[u8],
    inbox: &Inbox,
) {
    for &byte in bytes {
        if let Some(Ok(command)) = decoder.feed::<Command>(byte) {
            inbox.send((link, session, command)).await;
        }
    }
}

/// Encodes `message` as one frame into `buf`. `None` when it does not
/// fit, which PROTO-2 rules out for every `Message`.
// DECISION: an unencodable message is skipped rather than panicking:
// with `panic-halt` a panic would stop every link and sensor.
pub fn encode<'b>(message: &Message, buf: &'b mut [u8; MAX_FRAME]) -> Option<&'b [u8]> {
    frame::encode(message, buf).ok()
}
