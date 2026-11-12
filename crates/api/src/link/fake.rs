//! A link to no board: the test plays the firmware. For tests downstream
//! of this crate (feature `fake`) and for its own.
//!
//! Depends on: `link`.

use super::{Baud, Connection, Memory, Shared, Transport, Uart, Usb, lock, memory_pair, target};
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use xiao_proto::frame::{self, Decoder, MAX_FRAME};
use xiao_proto::message::{Command, ImuConfig, Message, Refusal, Sensor};

/// How long the board's thread waits for a byte before it looks whether
/// it was unplugged.
const POLL: Duration = Duration::from_millis(20);

/// How long [`Board::unplug`] waits for the API side to see the link end.
const UNPLUG_WAIT: Duration = Duration::from_secs(1);

/// Opens links to a [`Board`] the test plays instead of a real XIAO
/// (feature `fake`).
///
/// ```
/// use xiao_api::link::Fake;
/// use xiao_api::proto::message::{ImuReading, Message};
/// use xiao_api::{Error, Gyro};
///
/// let (usb, board) = Fake::usb();
/// board.auto_answer();
/// let gyro = Gyro::on(&usb)?;
/// board.send(Message::Imu(ImuReading { counter: 0, accel: None, gyro: Some([0, 0, 1000]) }));
/// assert_eq!((&gyro).next().transpose()?.map(|r| r.z.0), Some(8.75));
///
/// board.unplug();
/// assert!(matches!(gyro.try_latest(), Err(Error::Disconnected)));
/// # Ok::<(), Error>(())
/// ```
#[derive(Debug)]
pub struct Fake;

impl Fake {
    /// A [`Usb`] link to a fake board, and the board.
    ///
    /// # Panics
    ///
    /// When the link's threads cannot start, as `std::thread::spawn` does.
    pub fn usb() -> (Usb, Board) {
        let (connection, board) = plug();
        (Usb { connection }, board)
    }

    /// A [`Uart`] link at `baud` to a fake board, and the board. The board
    /// does not check the speed: play a link too slow for a sensor with
    /// [`Board::refuse_next`].
    ///
    /// # Panics
    ///
    /// When the link's threads cannot start, as `std::thread::spawn` does.
    // DECISION: `baud` is taken for the shape of `Uart::port(..).baud(..)` and
    // not checked: FW-4's figures are the firmware's to say, and a test states
    // them with `refuse_next`.
    pub fn uart(baud: Baud) -> (Uart, Board) {
        let _ = baud;
        let (connection, board) = plug();
        (Uart { connection }, board)
    }
}

/// The board end of a [`Fake`] link: the test plays the firmware with it.
///
/// It sees every command the API sends ([`Board::commands`]), answers
/// none until told to ([`Board::auto_answer`], [`Board::refuse_next`]),
/// and sends what the test gives it ([`Board::send`]). An unanswered
/// command times out in the API after 2 s, as with a silent board.
/// Dropping the `Board` unplugs it.
#[derive(Debug)]
pub struct Board {
    plug: Plug,
}

impl Board {
    /// From now on, answers every command as a working board would: `On`
    /// for a sensor turned on, `Off` for one turned off.
    pub fn auto_answer(&self) {
        lock(&self.plug.0.script).auto = true;
    }

    /// Answers the next command with `Refused` for `refusal`, whether
    /// [`Board::auto_answer`] is on or not. Each call refuses one more
    /// command, in order.
    // DECISION: refusals queue, so `refuse_next` twice refuses the next two
    // commands instead of the second call replacing the first.
    pub fn refuse_next(&self, refusal: Refusal) {
        lock(&self.plug.0.script).refusals.push_back(refusal);
    }

    /// Sends `message` to the API, as the firmware would: a reading
    /// ([`Message::Imu`], [`Message::Audio`]) or an answer. Lost once the
    /// board is unplugged.
    #[allow(clippy::needless_pass_by_value, reason = "the approved X03 signature")]
    pub fn send(&self, message: Message) {
        self.plug.0.send(&message);
    }

    /// Every command the API sent so far, oldest first.
    pub fn commands(&self) -> Vec<Command> {
        lock(&self.plug.0.script).seen.clone()
    }

    /// Pulls the plug: every stream on the link yields its one
    /// `Err(Error::Disconnected)` after what was sent before, and every
    /// call on the link fails at once. Returns once the API side has seen
    /// the link end, so the next read already gets that error.
    pub fn unplug(&self) {
        self.plug.0.unplug();
    }
}

/// Unplugs the board when the [`Board`] goes.
// DECISION: the unplug lives in a private field's Drop, not in
// `impl Drop for Board` (a trait impl on a pub type), as `StopOnDrop` does
// for `Connection`.
#[derive(Debug)]
struct Plug(Arc<Wire>);

impl Drop for Plug {
    fn drop(&mut self) {
        self.0.unplug();
    }
}

/// What the [`Board`] and its firmware thread share.
#[derive(Debug)]
struct Wire {
    /// The board's end of the in-memory link; `None` once unplugged.
    end: Mutex<Option<Memory>>,
    script: Mutex<Script>,
    /// The API side's routes, which say when it has seen the link end.
    api: Arc<Shared>,
}

/// How the board answers, and what it has seen.
#[derive(Debug, Default)]
struct Script {
    auto: bool,
    refusals: VecDeque<Refusal>,
    seen: Vec<Command>,
}

impl Wire {
    fn send(&self, message: &Message) {
        let mut buf = [0; MAX_FRAME];
        // DECISION: a message that does not fit one frame is not sent; every
        // message does (PROTO-2).
        let Ok(bytes) = frame::encode(message, &mut buf) else {
            return;
        };
        if let Some(end) = lock(&self.end).as_mut() {
            // An API side already gone reads nothing: the bytes are lost, as
            // on a wire.
            let _ = Transport::write(end, bytes);
        }
    }

    /// Notes `command` and answers it as the script says.
    fn answer(&self, command: Command) {
        let reply = {
            let mut script = lock(&self.script);
            script.seen.push(command);
            match script.refusals.pop_front() {
                Some(reason) => Some(Message::Refused { sensor: target(command).0, reason }),
                None => script.auto.then(|| reply_to(command)),
            }
        };
        if let Some(reply) = reply {
            self.send(&reply);
        }
    }

    fn unplugged(&self) -> bool {
        lock(&self.end).is_none()
    }

    /// Drops the board's end, the API side's only sender: its reader sees
    /// the link end at once. Waits until it has.
    fn unplug(&self) {
        drop(lock(&self.end).take());
        let deadline = Instant::now() + UNPLUG_WAIT;
        while lock(&self.api.routes).dead.is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
    }
}

/// A connection to a new fake board, and that board.
fn plug() -> (Connection, Board) {
    let (api, end) = memory_pair();
    // DECISION: `Fake::usb` and `Fake::uart` return no `Result` (the approved
    // shape), so a thread that cannot start panics, as `thread::spawn` does.
    let connection = Connection::new(Box::new(api)).expect("the fake link starts its reader");
    let rx = Arc::clone(&end.rx);
    let wire = Arc::new(Wire {
        end: Mutex::new(Some(end)),
        script: Mutex::default(),
        api: Arc::clone(&connection.inner.shared),
    });
    let firmware = Arc::clone(&wire);
    thread::Builder::new()
        .name("xiao-fake-board".into())
        .spawn(move || play(&firmware, &rx))
        .expect("the fake board starts its thread");
    (connection, Board { plug: Plug(wire) })
}

/// The firmware thread: reads the commands the API sends and answers them
/// as the script says, until the board is unplugged or the API side gone.
fn play(wire: &Wire, rx: &Mutex<Receiver<u8>>) {
    let mut decoder = Decoder::new();
    while !wire.unplugged() {
        match lock(rx).recv_timeout(POLL) {
            Ok(byte) => {
                if let Some(Ok(command)) = decoder.feed::<Command>(byte) {
                    wire.answer(command);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// What a working board answers to `command`: `On` for a sensor turned
/// on, `Off` for one turned off, an `ImuOn` with both halves off included.
pub(super) fn reply_to(command: Command) -> Message {
    match command {
        Command::MicOn => Message::On(Sensor::Mic),
        Command::MicOff => Message::Off(Sensor::Mic),
        Command::ImuOff | Command::ImuOn(ImuConfig { accel: None, gyro: None }) => {
            Message::Off(Sensor::Imu)
        }
        Command::ImuOn(_) => Message::On(Sensor::Imu),
    }
}

#[cfg(test)]
mod tests {
    use super::Fake;
    use crate::link::{Baud, BytesPerSec};
    use crate::{Dropped, Error, Gyro, Mic};
    use std::time::{Duration, Instant};
    use xiao_proto::message::{
        AudioBlock, Command, GyroConfig, ImuConfig, ImuReading, Link, Message, Refusal,
    };

    fn gyro_reading(counter: u32) -> Message {
        Message::Imu(ImuReading { counter, accel: None, gyro: Some([1000, 0, 0]) })
    }

    #[test]
    fn a_fake_board_answers_on() {
        let (usb, board) = Fake::usb();
        board.auto_answer();
        let gyro = Gyro::on(&usb).unwrap();

        board.send(gyro_reading(0));
        board.send(gyro_reading(2));
        let first = (&gyro).next().unwrap().unwrap();
        assert!((first.x.0 - 8.75).abs() < 1e-4, "{first:?}");
        (&gyro).next().unwrap().unwrap();
        assert_eq!(gyro.dropped(), Dropped(1), "reading 1 was lost");

        drop(gyro);
        let on = ImuConfig { accel: None, gyro: Some(GyroConfig::default()) };
        assert_eq!(board.commands(), [Command::ImuOn(on), Command::ImuOff]);
    }

    #[test]
    fn a_fake_board_can_refuse_busy() {
        let (uart, board) = Fake::uart(Baud(115_200));

        // Without auto_answer the refusal is still the answer: no 2 s wait.
        board.refuse_next(Refusal::Busy { held_by: Link::Ble });
        let start = Instant::now();
        let refused = Mic::on(&uart);
        assert!(matches!(refused, Err(Error::Busy { held_by: Link::Ble })), "{refused:?}");
        assert!(start.elapsed() < Duration::from_secs(1), "waited {:?}", start.elapsed());

        // Each call refuses one more command, in order; then auto answers.
        board.auto_answer();
        board.refuse_next(Refusal::TooSlow { needs: 32_000, link_has: 11_520 });
        board.refuse_next(Refusal::Busy { held_by: Link::Usb });
        match Mic::on(&uart) {
            Err(Error::TooSlow { needs, link_has }) => {
                assert_eq!(needs, BytesPerSec(32_000));
                assert_eq!(link_has, BytesPerSec(11_520));
            }
            other => panic!("expected TooSlow, got {other:?}"),
        }
        let busy = Gyro::on(&uart);
        assert!(matches!(busy, Err(Error::Busy { held_by: Link::Usb })), "{busy:?}");
        drop(Mic::on(&uart).unwrap());

        let gyro = ImuConfig { accel: None, gyro: Some(GyroConfig::default()) };
        let seen = [Command::MicOn, Command::MicOn, Command::ImuOn(gyro)];
        assert_eq!(board.commands(), [&seen[..], &[Command::MicOn, Command::MicOff]].concat());
    }

    #[test]
    fn an_unplugged_board_ends_the_stream() {
        let (usb, board) = Fake::usb();
        board.auto_answer();
        let mic = Mic::on(&usb).unwrap();
        let pcm = (0..96_i16).flat_map(i16::to_le_bytes).collect();
        board.send(Message::Audio(AudioBlock { counter: 0, pcm }));
        board.unplug();

        // What was sent before the plug came out, then the end, at once.
        let start = Instant::now();
        assert_eq!(mic.try_next().unwrap().unwrap().samples()[95], 95);
        assert!(matches!(mic.try_next(), Err(Error::Disconnected)));
        assert_eq!(mic.try_next().unwrap(), None);
        let late = Gyro::on(&usb);
        assert!(matches!(late, Err(Error::Disconnected)), "{late:?}");
        drop(mic);
        assert!(start.elapsed() < Duration::from_millis(100), "waited {:?}", start.elapsed());
        assert_eq!(board.commands(), [Command::MicOn], "nothing reached it after the unplug");

        // Dropping the board unplugs it too.
        let (usb, board) = Fake::usb();
        drop(board);
        let start = Instant::now();
        let late = Gyro::on(&usb);
        assert!(matches!(late, Err(Error::Disconnected)), "{late:?}");
        assert!(start.elapsed() < Duration::from_millis(100), "waited {:?}", start.elapsed());
    }
}
