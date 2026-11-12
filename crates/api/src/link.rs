//! The connection to the firmware. One type per link (`Usb`, `Ble`,
//! `Uart`), each with `open()`; all of them send commands and receive
//! messages the same way.
//!
//! Depends on: `xiao-proto::frame`, `error`.

pub mod ble;
#[cfg(any(test, feature = "fake"))]
mod fake;
pub mod uart;
pub mod usb;

pub use ble::{Ble, BleBuilder};
// DECISION: `fake` stays a private module and only `Fake` and `Board` are
// re-exported, the two items X03 names; the crate's own tests get them too.
#[cfg(any(test, feature = "fake"))]
pub use fake::{Board, Fake};
pub use uart::{Uart, UartBuilder};
pub use usb::Usb;

use crate::Error;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;
use xiao_proto::frame::{self, Decoder, MAX_FRAME};
use xiao_proto::message::{Command, ImuConfig, Link, Message, Refusal, Sensor};

/// A serial speed, in bits per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Baud(pub u32);

/// A data rate, in bytes per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BytesPerSec(pub u32);

/// How long [`Connection::request`] waits for `On`, `Off` or `Refused`.
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);

// DECISION: each subscriber queues at most 4096 readings (about 24 s of
// audio, 5 s of IMU at 833 Hz); readings past that are dropped at the route,
// so a sensor held but never read keeps memory bounded, and the counter gap
// shows them in `dropped()` (API-4).
/// Readings one subscriber may have waiting before new ones are dropped.
const QUEUE: usize = 4096;

/// Bytes moved to and from the board. Every link (USB, UART, BLE, the
/// in-memory test pair) is one of these; nothing above it knows which.
///
/// `read` must return now and then even when nothing arrives (an error
/// of kind `TimedOut` or `WouldBlock`), so the reader thread can notice
/// that its [`Connection`] was dropped. `Ok(0)` means the link is gone.
// DECISION: `try_clone` added to the planned write/read pair, the shape of
// `serialport::SerialPort::try_clone`: the reader thread owns one handle and
// writers the other, so a blocked read never holds up a command.
pub(crate) trait Transport: Send {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    fn try_clone(&self) -> io::Result<Box<dyn Transport>>;
}

/// How long a serial read waits before giving the reader thread a turn.
const SERIAL_POLL: Duration = Duration::from_millis(100);

/// A serial port: the USB link (CDC-ACM) and the UART link both are one.
struct Serial(Box<dyn serialport::SerialPort>);

impl Transport for Serial {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        io::Write::write_all(&mut self.0, bytes)?;
        io::Write::flush(&mut self.0)
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        io::Read::read(&mut self.0, buf)
    }

    fn try_clone(&self) -> io::Result<Box<dyn Transport>> {
        Ok(Box::new(Self(self.0.try_clone()?)))
    }
}

/// Opens a serial link on `path` at `baud`: the USB and UART links' `open`.
pub(crate) fn open_serial(path: &str, baud: Baud) -> Result<Connection, Error> {
    let port = serialport::new(path, baud.0).timeout(SERIAL_POLL).open().map_err(|e| {
        // The OS error alone does not say which port; the path does.
        let error = io::Error::from(e);
        Error::Io(io::Error::new(error.kind(), format!("{path}: {error}")))
    })?;
    Connection::new(Box::new(Serial(port)))
}

/// The shared inside of every link: sends commands, routes what comes
/// back to the sensor it belongs to. The link value and every sensor on
/// it hold one handle each; the link closes when the last one goes.
// DECISION: a handle around an `Arc`, cloned by the private `share` and not
// by a `Clone` impl (a new trait impl on a pub type), so the links keep
// their field and `AsLink` its signature.
#[derive(Debug)]
pub struct Connection {
    inner: Arc<Inner>,
}

/// What every handle to one [`Connection`] shares.
#[derive(Debug)]
struct Inner {
    writer: Mutex<Writer>,
    shared: Arc<Shared>,
    held: Mutex<Held>,
    /// How long a sensor's stream on this link waits for a reading.
    patience: Duration,
    _stop: StopOnDrop,
}

/// What the sensor values alive on this link hold. A new value reads it
/// to know what is on already: the IMU is one sensor (FW-2), so turning
/// one half on or off must name the other half too.
#[derive(Debug)]
pub(crate) struct Held {
    pub(crate) mic: bool,
    pub(crate) imu: ImuConfig,
}

/// What the reader thread and the [`Connection`] both see.
#[derive(Debug, Default)]
struct Shared {
    routes: Mutex<Routes>,
    /// Set when the last `Connection` handle is dropped; the reader thread
    /// then ends.
    closed: AtomicBool,
}

/// Where each incoming message goes. Indexed by [`slot`].
#[derive(Debug, Default)]
struct Routes {
    /// Why the reader thread ended, once it has; nothing will arrive any more.
    dead: Option<Error>,
    // DECISION: several subscribers per sensor, each getting every reading:
    // a `Gyro` and an `Accel` on one link both read the one IMU stream (FW-2).
    streams: [Vec<Subscriber>; 2],
    replies: [Option<Pending>; 2],
}

/// One reader of a sensor's stream.
#[derive(Debug)]
struct Subscriber {
    tx: SyncSender<Result<Message, Error>>,
    /// `false` until the board answers `On` for the sensor: readings
    /// before it belong to a stream that was running already.
    // DECISION: a new subscriber skips readings until the next `On` for
    // its sensor. The firmware starts the counter at 0 on every `On` and
    // sends `On` before the stream's readings, so what comes earlier is
    // an old stream (a UART sensor still held from a previous process)
    // whose counter would count as drops (API-4).
    on: bool,
}

/// A [`Connection::request`] waiting for its answer.
#[derive(Debug)]
struct Pending {
    /// `true` waits for `On`, `false` for `Off`; `Refused` answers both.
    wants_on: bool,
    reply: Sender<Result<(), Refusal>>,
}

/// The write half, behind its own lock.
struct Writer(Box<dyn Transport>);

impl std::fmt::Debug for Writer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Writer(..)")
    }
}

/// Tells the reader thread to end when the last handle to the
/// [`Connection`] goes.
// DECISION: the stop signal lives in a private field's Drop, not in
// `impl Drop for Connection`, which would be a new trait impl on a pub type.
#[derive(Debug)]
struct StopOnDrop(Arc<Shared>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.closed.store(true, Ordering::Release);
    }
}

impl Connection {
    /// Starts the reader thread on a clone of `transport` and keeps
    /// `transport` itself for writing.
    pub(crate) fn new(transport: Box<dyn Transport>) -> Result<Self, Error> {
        let reader = transport.try_clone()?;
        let shared = Arc::new(Shared::default());
        let for_thread = Arc::clone(&shared);
        // DECISION: the reader thread is detached, not joined on drop: a
        // transport whose read never returns would otherwise hang the drop.
        thread::Builder::new()
            .name("xiao-link-reader".into())
            .spawn(move || read_loop(reader, &for_thread))?;
        let inner = Inner {
            writer: Mutex::new(Writer(transport)),
            held: Mutex::new(Held { mic: false, imu: ImuConfig { accel: None, gyro: None } }),
            patience: crate::stream::PATIENCE,
            _stop: StopOnDrop(Arc::clone(&shared)),
            shared,
        };
        Ok(Self { inner: Arc::new(inner) })
    }

    /// Another handle to this connection, for a sensor turned on over it:
    /// the link stays open while the sensor lives.
    pub(crate) fn share(&self) -> Self {
        Self { inner: Arc::clone(&self.inner) }
    }

    /// Sends `cmd` and waits up to 2 s for the board's answer.
    ///
    /// # Errors
    ///
    /// [`Error::Busy`] / [`Error::TooSlow`] when the board refuses,
    /// [`Error::Timeout`] when it does not answer in time,
    /// [`Error::Disconnected`] when the link is gone, [`Error::Io`] when
    /// the command cannot be written.
    pub(crate) fn request(&self, cmd: Command) -> Result<(), Error> {
        let (sensor, wants_on) = target(cmd);
        let (tx, rx) = mpsc::channel();
        let shared = &self.inner.shared;
        {
            let mut routes = lock(&shared.routes);
            if let Some(cause) = &routes.dead {
                return Err(again(cause));
            }
            routes.replies[slot(sensor)] = Some(Pending { wants_on, reply: tx });
        }

        if let Err(error) = self.write(cmd) {
            lock(&shared.routes).replies[slot(sensor)] = None;
            return Err(error);
        }

        match rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(refusal)) => Err(refused(refusal)),
            Err(RecvTimeoutError::Timeout) => {
                lock(&shared.routes).replies[slot(sensor)] = None;
                // DECISION: one unanswered command counts as one attempt;
                // `attempts` > 1 only comes from the BLE reconnect (api-05).
                Err(Error::Timeout { attempts: 1 })
            }
            // The reader ended and dropped the reply sender: say why.
            Err(RecvTimeoutError::Disconnected) => {
                Err(lock(&shared.routes).dead.as_ref().map_or(Error::Disconnected, again))
            }
        }
    }

    /// Sends `cmd` without waiting for an answer: undoes an `On` whose
    /// request timed out, so a late `On` does not leave the sensor running.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when the command cannot be written.
    pub(crate) fn tell(&self, cmd: Command) -> Result<(), Error> {
        self.write(cmd)
    }

    /// Every reading for `sensor` from now on, in order. Ends after one
    /// `Err(Error::Disconnected)` when the link is gone. Every subscriber
    /// of a sensor gets every reading; dropping the receiver unsubscribes.
    pub(crate) fn subscribe(&self, sensor: Sensor) -> Receiver<Result<Message, Error>> {
        lock(&self.inner.shared.routes).add(sensor)
    }

    /// How long a sensor's stream on this link waits for a reading
    /// before it ends with [`Error::Timeout`].
    pub(crate) fn patience(&self) -> Duration {
        self.inner.patience
    }

    /// Sets how long the streams on this new link wait for a reading.
    ///
    /// # Panics
    ///
    /// When a sensor already shares the connection; a link calls it
    /// right after [`Connection::new`].
    fn set_patience(&mut self, patience: Duration) {
        Arc::get_mut(&mut self.inner).expect("a new link has one handle").patience = patience;
    }

    /// What the sensor values on this link hold. Kept locked across a
    /// whole on or off, so two of them never compute a command from the
    /// same stale state.
    pub(crate) fn held(&self) -> MutexGuard<'_, Held> {
        lock(&self.inner.held)
    }

    fn write(&self, cmd: Command) -> Result<(), Error> {
        let mut buf = [0; MAX_FRAME];
        let frame = frame::encode(&cmd, &mut buf)
            .map_err(|_| Error::Io(io::Error::other("command does not fit one frame")))?;
        // DECISION: every command goes out after a lone zero, in the same
        // write. Bytes the board got before it (the tty echoing the zero
        // the firmware parks in the IN endpoint as `^@` on the first open,
        // before raw mode) are ended by it and dropped by the firmware's
        // decoder, instead of being glued to the command and failing it.
        // A lone zero is an empty gap to the decoder: no frame, no reply.
        let mut bytes = Vec::with_capacity(frame.len() + 1);
        bytes.push(0);
        bytes.extend_from_slice(frame);
        lock(&self.inner.writer).0.write(&bytes)?;
        Ok(())
    }
}

/// The reader thread: decodes frames and routes each message until the
/// link dies or the last [`Connection`] handle is dropped, then ends
/// every route.
fn read_loop(mut transport: Box<dyn Transport>, shared: &Shared) {
    let mut decoder = Decoder::new();
    let mut buf = [0; MAX_FRAME];
    let mut cause = Error::Disconnected;
    while !shared.closed.load(Ordering::Acquire) {
        match transport.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                for &byte in buf.iter().take(n) {
                    // DECISION: a malformed frame is skipped silently; the
                    // decoder resyncs at the next zero (PROTO-3) and the
                    // counter gap shows up in `dropped()`.
                    if let Some(Ok(message)) = decoder.feed::<Message>(byte) {
                        route(shared, &message);
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::Interrupted
                ) => {}
            // DECISION: a transport that knows why it ended (the BLE link out
            // of reconnect attempts) wraps that `Error` in the I/O error, and
            // readers get it; any other read error is `Disconnected`.
            Err(e) => {
                if let Some(error) = e.into_inner().and_then(|inner| inner.downcast::<Error>().ok())
                {
                    cause = *error;
                }
                break;
            }
        }
    }
    shut(shared, &cause);
}

/// Sends one message to whoever waits for it; drops it when nobody does.
fn route(shared: &Shared, message: &Message) {
    let mut routes = lock(&shared.routes);
    match *message {
        Message::On(sensor) => {
            for subscriber in &mut routes.streams[slot(sensor)] {
                subscriber.on = true;
            }
            routes.answer(sensor, Some(true), Ok(()));
        }
        Message::Off(sensor) => routes.answer(sensor, Some(false), Ok(())),
        Message::Refused { sensor, reason } => routes.answer(sensor, None, Err(reason)),
        Message::Audio(_) => routes.stream(Sensor::Mic, message),
        Message::Imu(_) => routes.stream(Sensor::Imu, message),
    }
}

impl Routes {
    /// A new subscriber to `sensor`. On a dead link it gets its one error
    /// at once and then ends; it never blocks.
    fn add(&mut self, sensor: Sensor) -> Receiver<Result<Message, Error>> {
        let (tx, rx) = mpsc::sync_channel(QUEUE);
        match &self.dead {
            Some(cause) => end(tx, again(cause)),
            None => self.streams[slot(sensor)].push(Subscriber { tx, on: false }),
        }
        rx
    }

    /// Wakes the request waiting on `sensor`, if this is its answer.
    /// `on` is `Some(true)` for `On`, `Some(false)` for `Off`, `None` for
    /// `Refused`, which answers either.
    fn answer(&mut self, sensor: Sensor, on: Option<bool>, answer: Result<(), Refusal>) {
        let waiting = &mut self.replies[slot(sensor)];
        let fits =
            waiting.as_ref().is_some_and(|pending| on.is_none_or(|on| on == pending.wants_on));
        if fits && let Some(pending) = waiting.take() {
            let _ = pending.reply.send(answer);
        } else if let Err(refusal) = answer {
            // DECISION: a `Refused` nobody asked for (the board refusing what
            // a BLE reconnect sent again) ends that sensor's streams with the
            // refusal, instead of leaving them waiting for readings that will
            // never come.
            for subscriber in std::mem::take(&mut self.streams[slot(sensor)]) {
                end(subscriber.tx, refused(refusal));
            }
        } else if on == Some(false) {
            // DECISION: an `Off` nobody asked for (the board switched the
            // sensor off: its chip stopped answering) ends the streams that
            // saw `On` with `Timeout { attempts: 1 }`, the error their
            // patience would have given, only now instead of 2 s to 41 s
            // later. A subscriber still waiting for its `On` stays: the
            // `Off` belongs to the stream before it.
            let streams = std::mem::take(&mut self.streams[slot(sensor)]);
            let (ended, waiting): (Vec<_>, Vec<_>) = streams.into_iter().partition(|s| s.on);
            self.streams[slot(sensor)] = waiting;
            for subscriber in ended {
                end(subscriber.tx, Error::Timeout { attempts: 1 });
            }
        }
    }

    /// Passes a reading to each of the sensor's subscribers that saw
    /// `On`; forgets the ones that have gone away, skips the ones whose
    /// queue is full.
    fn stream(&mut self, sensor: Sensor, message: &Message) {
        self.streams[slot(sensor)].retain(|subscriber| {
            if !subscriber.on {
                return true;
            }
            match subscriber.tx.try_send(Ok(message.clone())) {
                Ok(()) | Err(TrySendError::Full(_)) => true,
                Err(TrySendError::Disconnected(_)) => false,
            }
        });
    }
}

/// Hands a subscriber its stream's one error. A full queue gets it after
/// the readings already waiting, from a short-lived thread that ends when
/// the subscriber reads it or goes away.
fn end(tx: SyncSender<Result<Message, Error>>, error: Error) {
    if let Err(TrySendError::Full(error)) = tx.try_send(Err(error)) {
        let _ =
            thread::Builder::new().name("xiao-link-end".into()).spawn(move || drop(tx.send(error)));
    }
}

/// The link is gone because of `cause`: every subscriber gets one error,
/// then its stream ends; every waiting request wakes up.
fn shut(shared: &Shared, cause: &Error) {
    let mut routes = lock(&shared.routes);
    routes.dead = Some(again(cause));
    for subscriber in routes.streams.iter_mut().flat_map(std::mem::take) {
        end(subscriber.tx, again(cause));
    }
    // Dropping the reply senders wakes `request`, which reads `dead`.
    routes.replies = [None, None];
}

/// A copy of `error`, for each reader of a link's end. An I/O error keeps
/// its kind and text.
fn again(error: &Error) -> Error {
    match error {
        Error::Busy { held_by } => Error::Busy { held_by: *held_by },
        Error::TooSlow { needs, link_has } => Error::TooSlow { needs: *needs, link_has: *link_has },
        Error::Timeout { attempts } => Error::Timeout { attempts: *attempts },
        Error::NotFound => Error::NotFound,
        Error::Disconnected => Error::Disconnected,
        Error::Io(e) => Error::Io(io::Error::new(e.kind(), e.to_string())),
    }
}

/// The sensor a command is for, and whether it turns it on.
fn target(cmd: Command) -> (Sensor, bool) {
    match cmd {
        Command::MicOn => (Sensor::Mic, true),
        Command::MicOff => (Sensor::Mic, false),
        Command::ImuOn(_) => (Sensor::Imu, true),
        Command::ImuOff => (Sensor::Imu, false),
    }
}

fn slot(sensor: Sensor) -> usize {
    match sensor {
        Sensor::Mic => 0,
        Sensor::Imu => 1,
    }
}

fn refused(refusal: Refusal) -> Error {
    match refusal {
        Refusal::Busy { held_by } => Error::Busy { held_by },
        Refusal::TooSlow { needs, link_has } => {
            Error::TooSlow { needs: BytesPerSec(needs), link_has: BytesPerSec(link_has) }
        }
    }
}

// DECISION: a poisoned lock is still used: the data behind it (routes, a
// write handle) stays consistent even if a holder panicked.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One end of an in-memory link: what one end writes, the other reads.
#[cfg(any(test, feature = "fake"))]
#[derive(Debug, Clone)]
pub(crate) struct Memory {
    tx: Sender<u8>,
    rx: Arc<Mutex<Receiver<u8>>>,
}

/// Two connected in-memory ends: (API side, board side).
#[cfg(any(test, feature = "fake"))]
pub(crate) fn memory_pair() -> (Memory, Memory) {
    let (to_board, board_rx) = mpsc::channel();
    let (to_api, api_rx) = mpsc::channel();
    (
        Memory { tx: to_board, rx: Arc::new(Mutex::new(api_rx)) },
        Memory { tx: to_api, rx: Arc::new(Mutex::new(board_rx)) },
    )
}

/// A [`Usb`] link over memory, and the board end to play the firmware.
#[cfg(test)]
pub(crate) fn memory_usb() -> (Usb, Memory) {
    let (api, board) = memory_pair();
    let connection = Connection::new(Box::new(api)).expect("in-memory link opens");
    (Usb { connection }, board)
}

#[cfg(any(test, feature = "fake"))]
impl Transport for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        for &byte in bytes {
            self.tx.send(byte).map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        }
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let rx = lock(&self.rx);
        let mut n = 0;
        for slot in buf.iter_mut() {
            let next = if n == 0 {
                match rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(byte) => Some(byte),
                    Err(RecvTimeoutError::Timeout) => {
                        return Err(io::ErrorKind::TimedOut.into());
                    }
                    Err(RecvTimeoutError::Disconnected) => return Ok(0),
                }
            } else {
                rx.try_recv().ok()
            };
            let Some(byte) = next else { break };
            *slot = byte;
            n += 1;
        }
        Ok(n)
    }

    fn try_clone(&self) -> io::Result<Box<dyn Transport>> {
        Ok(Box::new(self.clone()))
    }
}

/// The board's side of the conversation, for tests.
#[cfg(test)]
impl Memory {
    /// Sends one message as the firmware would.
    pub(crate) fn send(&mut self, message: &Message) {
        let mut buf = [0; MAX_FRAME];
        let bytes = frame::encode(message, &mut buf).expect("message fits a frame");
        Transport::write(self, bytes).expect("API end is alive");
    }

    /// Answers the next command the API sent as the firmware would (`On`,
    /// or `Off` for an off or an `ImuOn` with both halves off), and
    /// returns it; `None` after `wait` of silence.
    pub(crate) fn reply(&mut self, wait: Duration) -> Option<Command> {
        let command = self.command(wait)?;
        self.send(&fake::reply_to(command));
        Some(command)
    }

    /// Whether the API end goes away within `wait`, skipping what it still
    /// sends: true once every handle to its connection is dropped.
    pub(crate) fn closed(&self, wait: Duration) -> bool {
        let deadline = std::time::Instant::now() + wait;
        let rx = lock(&self.rx);
        loop {
            let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) else {
                return false;
            };
            match rx.recv_timeout(left) {
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => return false,
                Err(RecvTimeoutError::Disconnected) => return true,
            }
        }
    }

    /// The next command the API sent, or `None` after `wait` of silence.
    pub(crate) fn command(&mut self, wait: Duration) -> Option<Command> {
        let deadline = std::time::Instant::now() + wait;
        let mut decoder = Decoder::new();
        let rx = lock(&self.rx);
        loop {
            let left = deadline.checked_duration_since(std::time::Instant::now())?;
            let byte = rx.recv_timeout(left).ok()?;
            if let Some(Ok(command)) = decoder.feed::<Command>(byte) {
                return Some(command);
            }
        }
    }
}

/// Runs `act` while the board answers the one command it sends, as the
/// firmware would ([`Memory::reply`]); returns `act`'s result and that
/// command.
#[cfg(test)]
pub(crate) fn answered<T>(board: &Memory, act: impl FnOnce() -> T) -> (T, Command) {
    let mut firmware = board.clone();
    let replied = thread::spawn(move || firmware.reply(Duration::from_secs(5)));
    let result = act();
    let command = replied.join().expect("firmware thread").expect("a command arrives");
    (result, command)
}

mod sealed {
    pub trait Sealed {
        fn connection(&self) -> &super::Connection;
        fn kind(&self) -> super::Link;
    }
}

/// The connection behind `link`, and which link it is.
pub(crate) fn parts(link: &impl AsLink) -> (&Connection, Link) {
    (link.connection(), link.kind())
}

/// Anything a sensor can be turned on over: [`Usb`], [`Ble`], [`Uart`]
/// (API-1). Cannot be implemented outside this crate.
pub trait AsLink: sealed::Sealed {}

impl sealed::Sealed for Usb {
    fn connection(&self) -> &Connection {
        &self.connection
    }
    fn kind(&self) -> Link {
        Link::Usb
    }
}
impl AsLink for Usb {}

impl sealed::Sealed for Ble {
    fn connection(&self) -> &Connection {
        &self.connection
    }
    fn kind(&self) -> Link {
        Link::Ble
    }
}
impl AsLink for Ble {}

impl sealed::Sealed for Uart {
    fn connection(&self) -> &Connection {
        &self.connection
    }
    fn kind(&self) -> Link {
        Link::Uart
    }
}
impl AsLink for Uart {}

#[cfg(test)]
mod tests {
    use super::sealed::Sealed;
    use super::{
        AsLink, Baud, BytesPerSec, Connection, Memory, QUEUE, Shared, Uart, answered, lock,
        memory_pair, memory_usb, open_serial, route,
    };
    use crate::Dropped;
    use crate::Error;
    use crate::stream::Stream;
    use crate::{Gyro, Mic};
    use std::thread;
    use std::time::{Duration, Instant};
    use xiao_proto::frame::Decoder;
    use xiao_proto::message::{AudioBlock, Command, ImuReading, Link, Message, Refusal, Sensor};

    const QUIET: Duration = Duration::from_millis(200);

    fn memory_uart() -> (Uart, Memory) {
        let (api, board) = memory_pair();
        let connection = Connection::new(Box::new(api)).unwrap();
        (Uart { connection }, board)
    }

    /// Plays the firmware: answers the next command with `reply`, then
    /// returns the command and the board end.
    fn answer_once(mut board: Memory, reply: Message) -> thread::JoinHandle<(Command, Memory)> {
        thread::spawn(move || {
            let command = board.command(Duration::from_secs(5)).expect("a command arrives");
            board.send(&reply);
            (command, board)
        })
    }

    // What a sensor's `on` does, before the sensors exist (api-04).
    fn mic_on(link: &impl AsLink) -> Result<(), Error> {
        link.connection().request(Command::MicOn)
    }

    #[test]
    fn api_1_sensor_goes_on_over_the_link_it_was_given() {
        let (usb, usb_board) = memory_usb();
        let (uart, mut uart_board) = memory_uart();

        let usb_board = answer_once(usb_board, Message::On(Sensor::Mic));
        mic_on(&usb).unwrap();
        let (command, _usb_board) = usb_board.join().unwrap();
        assert_eq!(command, Command::MicOn);
        assert_eq!(uart_board.command(QUIET), None, "the other link heard nothing");

        let uart_board = answer_once(uart_board, Message::On(Sensor::Mic));
        mic_on(&uart).unwrap();
        assert_eq!(uart_board.join().unwrap().0, Command::MicOn);
    }

    #[test]
    fn request_times_out_without_reply() {
        let (usb, mut board) = memory_usb();
        let start = Instant::now();
        let result = mic_on(&usb);
        let waited = start.elapsed();

        assert!(matches!(result, Err(Error::Timeout { attempts: 1 })), "{result:?}");
        assert!(waited >= Duration::from_secs(2), "gave up after {waited:?}");
        assert!(waited < Duration::from_secs(3), "hung for {waited:?}");
        assert_eq!(board.command(QUIET), Some(Command::MicOn), "the command was sent");
    }

    #[test]
    fn command_after_a_zero_survives_garbage_before_it() {
        // The tty echoed `^@` to the board before the first command (the
        // firmware's parked zero, on the first open): the zero the API
        // sends first ends it, and the command still decodes.
        let (usb, board) = memory_usb();
        usb.connection().tell(Command::MicOn).unwrap();
        let sent: Vec<u8> = {
            let rx = lock(&board.rx);
            std::iter::from_fn(|| rx.recv_timeout(QUIET).ok()).collect()
        };
        assert_eq!(sent.first(), Some(&0), "{sent:?}");

        let decode = |wire: &[u8]| {
            let mut decoder = Decoder::new();
            wire.iter().filter_map(|&b| decoder.feed::<Command>(b)).collect::<Vec<_>>()
        };
        // Alone, the zero makes no frame: nothing for the firmware to answer.
        assert_eq!(decode(&sent), vec![Ok(Command::MicOn)]);
        let mut echoed = vec![0x5E, 0x40];
        echoed.extend_from_slice(&sent);
        let decoded = decode(&echoed);
        assert_eq!(decoded.last(), Some(&Ok(Command::MicOn)), "{decoded:?}");
        assert!(decoded.iter().rev().skip(1).all(Result::is_err), "{decoded:?}");
    }

    #[test]
    fn refusal_becomes_error() {
        let (usb, board) = memory_usb();

        let busy = Refusal::Busy { held_by: Link::Ble };
        let board = answer_once(board, Message::Refused { sensor: Sensor::Mic, reason: busy });
        let result = mic_on(&usb);
        assert!(matches!(result, Err(Error::Busy { held_by: Link::Ble })), "{result:?}");
        let (_, board) = board.join().unwrap();

        let slow = Refusal::TooSlow { needs: 32_000, link_has: 11_520 };
        let board = answer_once(board, Message::Refused { sensor: Sensor::Mic, reason: slow });
        match mic_on(&usb) {
            Err(Error::TooSlow { needs, link_has }) => {
                assert_eq!(needs, BytesPerSec(32_000));
                assert_eq!(link_has, BytesPerSec(11_520));
            }
            other => panic!("expected TooSlow, got {other:?}"),
        }
        board.join().unwrap();
    }

    #[test]
    fn readings_go_to_their_own_sensor() {
        let (usb, mut board) = memory_usb();
        let mic = usb.connection().subscribe(Sensor::Mic);
        let imu = usb.connection().subscribe(Sensor::Imu);
        board.send(&Message::On(Sensor::Mic));
        board.send(&Message::On(Sensor::Imu));

        let reading = ImuReading { counter: 7, accel: Some([1, 2, 3]), gyro: None };
        board.send(&Message::Imu(reading));
        board.send(&Message::Audio(AudioBlock { counter: 3, pcm: [1, 2].into_iter().collect() }));

        let got = imu.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
        assert_eq!(got, Message::Imu(reading));
        let got = mic.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
        assert!(matches!(got, Message::Audio(AudioBlock { counter: 3, .. })), "{got:?}");
        assert!(imu.recv_timeout(QUIET).is_err(), "audio did not reach the IMU");
    }

    #[test]
    fn reading_before_on_is_an_old_stream() {
        // A UART sensor still streams for a previous process when a new
        // one subscribes and asks: its readings before `On` are not
        // handed out, so they cannot count as drops (API-4).
        let shared = Shared::default();
        let rx = lock(&shared.routes).add(Sensor::Imu);
        let reading = |counter| Message::Imu(ImuReading { counter, accel: None, gyro: None });
        route(&shared, &reading(6_240));
        route(&shared, &Message::On(Sensor::Imu));
        route(&shared, &reading(0));
        let stream = Stream::new(rx, |message| match message {
            Message::Imu(reading) => Some(reading.counter),
            _ => None,
        });
        assert_eq!(stream.next().unwrap().unwrap(), 0);
        assert_eq!(stream.dropped(), Dropped(0));
    }

    #[test]
    fn unasked_off_ends_the_running_stream() {
        let shared = Shared::default();
        let running = lock(&shared.routes).add(Sensor::Imu);
        route(&shared, &Message::On(Sensor::Imu));
        let starting = lock(&shared.routes).add(Sensor::Imu);
        let mic = lock(&shared.routes).add(Sensor::Mic);
        route(&shared, &Message::On(Sensor::Mic));

        route(&shared, &Message::Off(Sensor::Imu));
        let got = running.recv_timeout(QUIET);
        assert!(matches!(got, Ok(Err(Error::Timeout { attempts: 1 }))), "{got:?}");
        assert!(running.recv_timeout(QUIET).is_err(), "then the stream ends");

        // Not yet on, or another sensor: untouched.
        route(&shared, &Message::On(Sensor::Imu));
        let reading = ImuReading { counter: 0, accel: None, gyro: Some([1, 2, 3]) };
        route(&shared, &Message::Imu(reading));
        assert_eq!(starting.recv_timeout(QUIET).unwrap().unwrap(), Message::Imu(reading));
        assert!(mic.try_recv().is_err(), "the mic stream goes on");
    }

    #[test]
    fn dead_transport_ends_subscribers_and_requests() {
        let (usb, board) = memory_usb();
        let mic = usb.connection().subscribe(Sensor::Mic);
        drop(board);

        let first = mic.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(first, Err(Error::Disconnected)), "{first:?}");
        assert!(mic.recv_timeout(Duration::from_secs(1)).is_err(), "then the stream ends");

        let late = usb.connection().subscribe(Sensor::Imu);
        assert!(matches!(late.recv_timeout(QUIET), Ok(Err(Error::Disconnected))));
        assert!(late.recv_timeout(QUIET).is_err());

        let start = Instant::now();
        assert!(matches!(mic_on(&usb), Err(Error::Disconnected)));
        assert!(start.elapsed() < Duration::from_secs(1), "no wait on a dead link");
    }

    #[test]
    fn dropping_the_connection_ends_the_reader() {
        let (usb, _board) = memory_usb();
        let mic = usb.connection().subscribe(Sensor::Mic);
        drop(usb);

        let first = mic.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(first, Err(Error::Disconnected)), "{first:?}");
        assert!(mic.recv_timeout(Duration::from_secs(1)).is_err());
    }

    #[test]
    fn the_link_closes_when_the_last_holder_drops() {
        let (usb, board) = memory_usb();
        let gyro = answered(&board, || Gyro::on(&usb)).0.unwrap();
        let mic = answered(&board, || Mic::on(&usb)).0.unwrap();

        drop(usb);
        assert!(!board.closed(QUIET), "the sensors keep the link open");
        answered(&board, || drop(gyro));
        assert!(!board.closed(QUIET), "the mic still holds it");
        answered(&board, || drop(mic));
        assert!(board.closed(Duration::from_secs(1)), "closed with its last holder");
    }

    #[test]
    fn links_are_send_and_sync() {
        fn check<T: Send + Sync + std::panic::UnwindSafe + std::panic::RefUnwindSafe>() {}
        check::<Connection>();
        check::<super::Usb>();
        check::<super::Ble>();
        check::<Uart>();
    }

    #[test]
    fn unread_stream_is_bounded_and_counts_overflow() {
        // A sensor held but not read: its queue stops at QUEUE readings, and
        // what was dropped shows up as a counter gap.
        let shared = Shared::default();
        let rx = lock(&shared.routes).add(Sensor::Imu);
        route(&shared, &Message::On(Sensor::Imu));
        let reading = |counter| Message::Imu(ImuReading { counter, accel: None, gyro: None });
        let extra = 5;
        let queue = u32::try_from(QUEUE).unwrap();
        for counter in 0..queue + extra {
            route(&shared, &reading(counter));
        }
        let stream = Stream::new(rx, |message| match message {
            Message::Imu(reading) => Some(reading.counter),
            _ => None,
        });
        for counter in 0..queue {
            assert_eq!(stream.next().unwrap().unwrap(), counter);
        }
        route(&shared, &reading(queue + extra));
        assert_eq!(stream.next().unwrap().unwrap(), queue + extra);
        assert_eq!(stream.dropped(), Dropped(u64::from(extra)));
    }

    #[test]
    fn serial_open_error_names_the_port() {
        let path = "/nonexistent/xiao-port";
        match open_serial(path, Baud(115_200)) {
            Err(Error::Io(e)) => assert!(e.to_string().contains(path), "{e}"),
            other => panic!("expected an I/O error, got {other:?}"),
        }
    }
}
