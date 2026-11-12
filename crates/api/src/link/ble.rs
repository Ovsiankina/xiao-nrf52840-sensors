//! The BLE link: finds the XIAO by name, reconnects up to `attempts`
//! times when it drops, then times out.
//!
//! Depends on: `link`.

use crate::Error;
use crate::link::{Connection, Transport};
use btleplug::api::{
    Central as _, CentralEvent, Characteristic, Manager as _, Peripheral as _, ScanFilter,
    WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::stream::{self, BoxStream, StreamExt as _};
use std::io;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use uuid::Uuid;
use xiao_proto::frame::{self, Decoder, MAX_FRAME};
use xiao_proto::message::{BLE_NAME, Command, ImuConfig};

/// Nordic UART Service: the board's one serial-like BLE service.
const NUS_SERVICE: Uuid = Uuid::from_u128(0x6e40_0001_b5a3_f393_e0a9_e50e_24dc_ca9e);
/// The characteristic the API writes commands to.
const NUS_WRITE: Uuid = Uuid::from_u128(0x6e40_0002_b5a3_f393_e0a9_e50e_24dc_ca9e);
/// The characteristic the board notifies messages on.
const NUS_NOTIFY: Uuid = Uuid::from_u128(0x6e40_0003_b5a3_f393_e0a9_e50e_24dc_ca9e);

/// Connection attempts when none are given (API-2).
const DEFAULT_ATTEMPTS: u8 = 3;
/// The wait between two connection attempts.
const PAUSE: Duration = Duration::from_secs(1);
/// How long one attempt scans for the board's name.
const SCAN: Duration = Duration::from_secs(5);
/// How often the scan looks at what it found.
const SCAN_POLL: Duration = Duration::from_millis(200);
/// How long a read waits before giving the reader thread a turn.
const READ_POLL: Duration = Duration::from_millis(100);

/// The link over Bluetooth Low Energy.
#[derive(Debug)]
pub struct Ble {
    pub(super) connection: super::Connection,
}

impl Ble {
    /// Connects to the board advertising the default name,
    /// `XIAO-Sense`, with 3 attempts (API-1, API-2).
    ///
    /// # Errors
    ///
    /// [`Error::Timeout`] when every attempt failed.
    pub fn open() -> Result<Self, Error> {
        // DECISION: default 3 attempts, 1 s apart; the SRS leaves both open.
        BleBuilder::defaults().open()
    }

    /// Starts a BLE link to a board advertising another name.
    pub fn named(name: &str) -> BleBuilder {
        BleBuilder::defaults().named(name)
    }

    /// Starts a BLE link to the default name with another number of
    /// connection attempts (API-2).
    pub fn attempts(attempts: u8) -> BleBuilder {
        BleBuilder::defaults().attempts(attempts)
    }
}

/// Settings for a [`Ble`] link before it is opened.
#[derive(Debug)]
pub struct BleBuilder {
    name: String,
    attempts: u8,
}

// DECISION: the defaults are a private function, not `impl Default`, which
// would be a new trait impl on a pub type.
impl BleBuilder {
    fn defaults() -> Self {
        Self { name: BLE_NAME.to_owned(), attempts: DEFAULT_ATTEMPTS }
    }
}

impl BleBuilder {
    /// Sets the name to look for.
    #[must_use]
    pub fn named(self, name: &str) -> Self {
        Self { name: name.to_owned(), ..self }
    }

    /// Sets how many times to try connecting, at first and after every
    /// drop, before giving up with [`Error::Timeout`] (API-2).
    #[must_use]
    pub fn attempts(self, attempts: u8) -> Self {
        Self { attempts, ..self }
    }

    /// Connects.
    ///
    /// # Errors
    ///
    /// [`Error::Timeout`] when every attempt failed.
    pub fn open(self) -> Result<Ble, Error> {
        let (writes, to_board) = unbounded_channel();
        let (from_board, incoming) = mpsc::channel();
        let (ready, connected) = mpsc::channel();
        thread::Builder::new()
            .name("xiao-ble".into())
            .spawn(move || run(&self, to_board, &from_board, &ready))?;
        connected
            .recv()
            .map_err(|_| Error::Io(io::Error::other("the BLE thread ended before connecting")))??;
        let incoming = Arc::new(Mutex::new(Incoming { rx: incoming, left: Vec::new() }));
        Ok(Ble { connection: Connection::new(Box::new(BleTransport { writes, incoming }))? })
    }
}

/// Calls `connect` up to `attempts` times, `pause` apart, until one
/// succeeds (API-2).
///
/// # Errors
///
/// [`Error::Timeout`] with the number of attempts made when all failed;
/// [`Error::Io`] at once when the host side fails.
// DECISION: `pause` is a parameter (the plan's signature has none) so the
// tests run without the real 1 s wait between attempts.
// DECISION: `attempts == 0` still tries once: a link that never tries cannot
// open, and `Timeout { attempts: 0 }` would read as a bug.
fn retry<T>(
    attempts: u8,
    pause: Duration,
    mut connect: impl FnMut() -> Result<T, Error>,
) -> Result<T, Error> {
    let attempts = attempts.max(1);
    for attempt in 1..=attempts {
        if attempt > 1 {
            thread::sleep(pause);
        }
        match connect() {
            Ok(connected) => return Ok(connected),
            // DECISION: an I/O error is the host's (adapter off, no
            // permission): no number of attempts fixes it, so it is returned
            // at once with its cause. Board-side failures are retried.
            Err(error @ Error::Io(_)) => return Err(error),
            Err(_) => {}
        }
    }
    Err(Error::Timeout { attempts })
}

/// The API's side of the BLE thread: bytes to write go out through a
/// channel, notified bytes come in through another.
#[derive(Clone)]
struct BleTransport {
    writes: UnboundedSender<Vec<u8>>,
    incoming: Arc<Mutex<Incoming>>,
}

/// What the BLE thread passes up: notified bytes, or why it gave up.
type Inbound = Result<Vec<u8>, Error>;

/// Notified bytes not read yet.
struct Incoming {
    rx: Receiver<Inbound>,
    /// The rest of a notification longer than the last read's buffer.
    left: Vec<u8>,
}

impl Transport for BleTransport {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writes.send(bytes.to_vec()).map_err(|_| io::ErrorKind::BrokenPipe.into())
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut incoming = super::lock(&self.incoming);
        if incoming.left.is_empty() {
            incoming.left = match incoming.rx.recv_timeout(READ_POLL) {
                Ok(Ok(bytes)) => bytes,
                // The BLE thread ran out of reconnect attempts: the reader
                // thread unwraps this error and hands it to every reader.
                Ok(Err(error)) => return Err(io::Error::other(error)),
                Err(RecvTimeoutError::Timeout) => return Err(io::ErrorKind::TimedOut.into()),
                // The BLE thread gave up reconnecting: the link is gone.
                Err(RecvTimeoutError::Disconnected) => return Ok(0),
            };
        }
        let n = buf.len().min(incoming.left.len());
        buf[..n].copy_from_slice(&incoming.left[..n]);
        incoming.left.drain(..n);
        Ok(n)
    }

    fn try_clone(&self) -> io::Result<Box<dyn Transport>> {
        Ok(Box::new(self.clone()))
    }
}

/// A connected board, ready to talk.
struct Board {
    peripheral: Peripheral,
    write: Characteristic,
    notifications: BoxStream<'static, btleplug::api::ValueNotification>,
    events: BoxStream<'static, CentralEvent>,
}

/// Why [`pump`] returned.
enum End {
    /// Every API handle is gone: the [`Ble`] was dropped.
    Closed,
    /// The board went away.
    Dropped,
}

/// One thing that happened while connected.
enum Event {
    Notified(Vec<u8>),
    Write(Vec<u8>),
    Gone,
    Closed,
}

/// The BLE thread: owns the tokio runtime, connects, moves bytes, and
/// reconnects after a drop. Tells `ready` how the first connection went.
// DECISION: one private thread with a current-thread runtime: the public API
// stays blocking, and btleplug's futures all run on this one thread.
fn run(
    settings: &BleBuilder,
    mut to_board: UnboundedReceiver<Vec<u8>>,
    from_board: &Sender<Inbound>,
    ready: &Sender<Result<(), Error>>,
) {
    let start = || -> Result<(Runtime, Adapter), Error> {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        let adapter = runtime.block_on(adapter())?;
        Ok((runtime, adapter))
    };
    let (runtime, adapter) = match start() {
        Ok(started) => started,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let with_retry =
        || retry(settings.attempts, PAUSE, || runtime.block_on(connect(&adapter, &settings.name)));
    let mut board = match with_retry() {
        Ok(board) => board,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let _ = ready.send(Ok(()));

    let mut held = Held::default();
    loop {
        match runtime.block_on(pump(&mut board, &mut to_board, from_board, &mut held)) {
            End::Closed => {
                let _ = runtime.block_on(board.peripheral.disconnect());
                return;
            }
            End::Dropped => {
                board = match with_retry() {
                    Ok(again) => again,
                    Err(error) => {
                        // After the last failed reconnect the link ends with
                        // the retry's error, `Timeout { attempts }` (API-2).
                        let _ = from_board.send(Err(error));
                        return;
                    }
                };
                forget_queued(&mut to_board, &mut held);
                runtime.block_on(held.replay(&board));
            }
        }
    }
}

/// Drops the commands written while the board was away (their requests
/// already timed out), noting each, so an off sent during the outage is
/// not undone by the replay.
// DECISION: the board released its sensors on the drop, so what is still on
// after these commands is what the replay sends again.
fn forget_queued(to_board: &mut UnboundedReceiver<Vec<u8>>, held: &mut Held) {
    while let Ok(bytes) = to_board.try_recv() {
        held.note(&bytes);
    }
}

/// The first Bluetooth adapter.
async fn adapter() -> Result<Adapter, Error> {
    let manager = Manager::new().await.map_err(io_error)?;
    let adapters = manager.adapters().await.map_err(io_error)?;
    // DECISION: no Bluetooth adapter is an I/O error, returned at once rather
    // than retried: no number of attempts would find one.
    adapters.into_iter().next().ok_or_else(|| Error::Io(io::Error::other("no Bluetooth adapter")))
}

/// One connection attempt: scans for `name`, connects, finds the Nordic
/// UART Service and subscribes to its notifications.
async fn connect(adapter: &Adapter, name: &str) -> Result<Board, Error> {
    let events = adapter.events().await.map_err(io_error)?;
    let peripheral = find(adapter, name).await?;
    peripheral.connect().await.map_err(board_error)?;
    peripheral.discover_services().await.map_err(board_error)?;
    let characteristic = |uuid| {
        peripheral
            .characteristics()
            .into_iter()
            .find(|c| c.service_uuid == NUS_SERVICE && c.uuid == uuid)
            .ok_or(Error::NotFound)
    };
    let write = characteristic(NUS_WRITE)?;
    let notify = characteristic(NUS_NOTIFY)?;
    let notifications = peripheral.notifications().await.map_err(board_error)?;
    peripheral.subscribe(&notify).await.map_err(board_error)?;
    Ok(Board { peripheral, write, notifications, events })
}

/// Scans up to [`SCAN`] for a board advertising `name`.
async fn find(adapter: &Adapter, name: &str) -> Result<Peripheral, Error> {
    // DECISION: the scan filters by name, not by service UUID: the name is
    // what the firmware is sure to advertise (BLE_NAME); the UUID may not fit
    // the advertising packet.
    adapter.start_scan(ScanFilter::default()).await.map_err(io_error)?;
    let deadline = Instant::now() + SCAN;
    let found = loop {
        if let Some(peripheral) = named(adapter, name).await? {
            break Ok(peripheral);
        }
        if Instant::now() >= deadline {
            break Err(Error::NotFound);
        }
        tokio::time::sleep(SCAN_POLL).await;
    };
    let _ = adapter.stop_scan().await;
    found
}

/// A peripheral seen so far whose advertised name is `name`.
async fn named(adapter: &Adapter, name: &str) -> Result<Option<Peripheral>, Error> {
    for peripheral in adapter.peripherals().await.map_err(io_error)? {
        let Ok(Some(properties)) = peripheral.properties().await else {
            continue;
        };
        let names = [properties.local_name, properties.advertisement_name];
        if names.iter().flatten().any(|n| n == name) {
            return Ok(Some(peripheral));
        }
    }
    Ok(None)
}

/// Moves bytes both ways until the board goes away or the API is done.
async fn pump(
    board: &mut Board,
    to_board: &mut UnboundedReceiver<Vec<u8>>,
    from_board: &Sender<Inbound>,
    held: &mut Held,
) -> End {
    let id = board.peripheral.id();
    let notified = board
        .notifications
        .by_ref()
        .filter(|n| std::future::ready(n.uuid == NUS_NOTIFY))
        .map(|n| Event::Notified(n.value));
    let gone = board.events.by_ref().filter_map(|event| {
        let gone = matches!(event, CentralEvent::DeviceDisconnected(ref d) if *d == id);
        std::future::ready(gone.then_some(Event::Gone))
    });
    let writes = stream::poll_fn(|cx| to_board.poll_recv(cx))
        .map(Event::Write)
        .chain(stream::once(std::future::ready(Event::Closed)));
    let mut events = stream::select(stream::select(notified, gone), writes);

    while let Some(event) = events.next().await {
        match event {
            Event::Notified(bytes) => {
                if from_board.send(Ok(bytes)).is_err() {
                    return End::Closed;
                }
            }
            Event::Write(bytes) => {
                held.note(&bytes);
                // DECISION: a failed write is not reported here; the request
                // that sent it times out, and a real drop arrives as `Gone`.
                let _ = write(&board.peripheral, &board.write, &bytes).await;
            }
            Event::Gone => return End::Dropped,
            Event::Closed => return End::Closed,
        }
    }
    End::Dropped
}

/// Writes `bytes` to the board, split to fit the link's MTU.
async fn write(peripheral: &Peripheral, to: &Characteristic, bytes: &[u8]) -> Result<(), Error> {
    // An ATT write carries the MTU less 3 bytes of header.
    let chunk = usize::from(peripheral.mtu().saturating_sub(3)).max(20);
    for part in bytes.chunks(chunk) {
        // DECISION: write with response: commands are rare, and the board
        // acknowledging each piece keeps them in order.
        peripheral.write(to, part, WriteType::WithResponse).await.map_err(io_error)?;
    }
    Ok(())
}

/// What the API last turned on, read from the commands it wrote, so a
/// reconnect can turn it on again.
#[derive(Default)]
struct Held {
    decoder: Decoder,
    mic: Option<Command>,
    imu: Option<Command>,
}

impl Held {
    fn note(&mut self, bytes: &[u8]) {
        const IMU_NONE: ImuConfig = ImuConfig { accel: None, gyro: None };
        for &byte in bytes {
            let Some(Ok(command)) = self.decoder.feed::<Command>(byte) else {
                continue;
            };
            match command {
                Command::MicOn => self.mic = Some(command),
                Command::MicOff => self.mic = None,
                Command::ImuOff | Command::ImuOn(IMU_NONE) => self.imu = None,
                Command::ImuOn(_) => self.imu = Some(command),
            }
        }
    }

    /// The commands that left a sensor on, in the order to send them.
    fn commands(&self) -> impl Iterator<Item = &Command> {
        self.mic.iter().chain(&self.imu)
    }

    /// Sends again every command that left a sensor on. The board's `On`
    /// answers reach no waiting request and are dropped by the routes.
    async fn replay(&self, board: &Board) {
        for command in self.commands() {
            let mut buf = [0; MAX_FRAME];
            if let Ok(bytes) = frame::encode(command, &mut buf) {
                let _ = write(&board.peripheral, &board.write, bytes).await;
            }
        }
    }
}

/// A failure of the board's side of a connection attempt (it went out of
/// range mid-connect, or dropped the link), which a retry may get past.
// DECISION: `Disconnected`, not `Io`, so `retry` tries again; the cause is
// not kept, since a retry that runs out reports `Timeout { attempts }`.
#[allow(clippy::needless_pass_by_value, reason = "used as `map_err(board_error)`")]
fn board_error(_: btleplug::Error) -> Error {
    Error::Disconnected
}

/// btleplug's errors, as the I/O error they are to the developer.
#[allow(clippy::needless_pass_by_value, reason = "used as `map_err(io_error)`")]
fn io_error(error: btleplug::Error) -> Error {
    Error::Io(io::Error::other(error))
}

#[cfg(test)]
mod tests {
    use super::{Ble, BleTransport, Held, Inbound, Incoming, forget_queued, retry};
    use crate::link::Connection;
    use crate::{Error, Mic};
    use std::cell::Cell;
    use std::io;
    use std::sync::mpsc::{self, Sender};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
    use xiao_proto::frame::{self, Decoder, MAX_FRAME};
    use xiao_proto::message::{Command, GyroConfig, ImuConfig, Message, Sensor};

    fn frame_of(command: Command) -> Vec<u8> {
        let mut buf = [0; MAX_FRAME];
        frame::encode(&command, &mut buf).unwrap().to_vec()
    }

    fn message_frame(message: &Message) -> Vec<u8> {
        let mut buf = [0; MAX_FRAME];
        frame::encode(message, &mut buf).unwrap().to_vec()
    }

    /// A [`Ble`] whose BLE thread is played by the test: the receiver gets
    /// what the API writes, the sender passes up what the board notifies.
    fn memory_ble() -> (Ble, UnboundedReceiver<Vec<u8>>, Sender<Inbound>) {
        let (writes, to_board) = unbounded_channel();
        let (from_board, rx) = mpsc::channel();
        let incoming = Arc::new(Mutex::new(Incoming { rx, left: Vec::new() }));
        let connection = Connection::new(Box::new(BleTransport { writes, incoming })).unwrap();
        (Ble { connection }, to_board, from_board)
    }

    #[test]
    fn api_2_retries_then_times_out() {
        let calls = Cell::new(0);
        let result: Result<(), Error> = retry(5, Duration::ZERO, || {
            calls.set(calls.get() + 1);
            Err(Error::NotFound)
        });
        assert_eq!(calls.get(), 5);
        assert!(matches!(result, Err(Error::Timeout { attempts: 5 })), "{result:?}");
    }

    #[test]
    fn retry_stops_at_first_success() {
        let calls = Cell::new(0);
        let result = retry(5, Duration::ZERO, || {
            calls.set(calls.get() + 1);
            if calls.get() == 2 { Ok("connected") } else { Err(Error::Disconnected) }
        });
        assert_eq!(result.ok(), Some("connected"));
        assert_eq!(calls.get(), 2, "no attempt after the one that worked");
    }

    #[test]
    fn zero_attempts_still_tries_once() {
        let calls = Cell::new(0);
        let result: Result<(), Error> = retry(0, Duration::ZERO, || {
            calls.set(calls.get() + 1);
            Err(Error::NotFound)
        });
        assert_eq!(calls.get(), 1);
        assert!(matches!(result, Err(Error::Timeout { attempts: 1 })), "{result:?}");
    }

    #[test]
    fn host_error_is_not_retried() {
        let calls = Cell::new(0);
        let result: Result<(), Error> = retry(5, Duration::ZERO, || {
            calls.set(calls.get() + 1);
            Err(Error::Io(io::Error::other("Bluetooth is powered off")))
        });
        assert_eq!(calls.get(), 1, "no retry for a host-side failure");
        match result {
            Err(Error::Io(e)) => assert!(e.to_string().contains("powered off"), "{e}"),
            other => panic!("expected the I/O error, got {other:?}"),
        }
    }

    #[test]
    fn api_2_reconnect_gives_up_with_timeout() {
        let (ble, mut to_board, from_board) = memory_ble();

        // The board answers MicOn.
        let board = from_board.clone();
        let firmware = thread::spawn(move || {
            let mut decoder = Decoder::new();
            let command = loop {
                let bytes = to_board.blocking_recv().expect("a command arrives");
                if let Some(Ok(command)) = bytes.iter().find_map(|&b| decoder.feed::<Command>(b)) {
                    break command;
                }
            };
            board.send(Ok(message_frame(&Message::On(Sensor::Mic)))).unwrap();
            (command, to_board)
        });
        let mic = Mic::on(&ble).unwrap();
        let (command, _to_board) = firmware.join().unwrap();
        assert_eq!(command, Command::MicOn);

        // The link drops and every reconnect fails: the reader gets the
        // retry's Timeout with the attempts made, then the stream ends.
        from_board.send(Err(Error::Timeout { attempts: 3 })).unwrap();
        let got = (&mic).next();
        assert!(matches!(got, Some(Err(Error::Timeout { attempts: 3 }))), "{got:?}");
        assert!((&mic).next().is_none());

        // Calls on the dead link say the same, at once.
        let again = Mic::on(&ble);
        assert!(matches!(again, Err(Error::Busy { .. })), "the first Mic still holds it");
        drop(mic);
        let again = Mic::on(&ble);
        assert!(matches!(again, Err(Error::Timeout { attempts: 3 })), "{again:?}");
    }

    #[test]
    fn reconnect_does_not_replay_what_was_turned_off() {
        let mut held = Held::default();
        held.note(&frame_of(Command::MicOn));
        let imu = ImuConfig { accel: None, gyro: Some(GyroConfig::default()) };
        held.note(&frame_of(Command::ImuOn(imu)));

        // MicOff was written while the board was away, then dropped.
        let (tx, mut to_board) = unbounded_channel();
        tx.send(frame_of(Command::MicOff)).unwrap();
        forget_queued(&mut to_board, &mut held);

        let replayed: Vec<Command> = held.commands().copied().collect();
        assert_eq!(replayed, [Command::ImuOn(imu)], "the mic is not turned on again");
    }
}
