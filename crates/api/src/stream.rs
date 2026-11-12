//! Readings as they arrive: iterate over them, ask for the latest one,
//! and learn how many were dropped (from the counter in each message).
//!
//! Depends on: `link`, `error`.

use crate::Error;
use crate::link::lock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::{Duration, Instant};
use xiao_proto::message::Message;

/// How many readings were lost on the way since the sensor was turned
/// on (API-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Dropped(pub u64);

/// Turns one routed message into the reading a sensor hands out, or
/// `None` when the message carries nothing for it (e.g. an IMU reading
/// without the gyroscope half).
type Convert<T> = Box<dyn Fn(Message) -> Option<T> + Send>;

// DECISION: 2 s without a reading ends the stream with `Timeout`, so a link
// that cannot notice the board going away (UART behind an adapter) never
// blocks for ever; the slowest stream (12.5 Hz) sends every 80 ms.
/// How long `next` and `latest` wait for a reading before giving up.
pub(crate) const PATIENCE: Duration = Duration::from_secs(2);

/// One sensor's readings, from the [`Receiver`] that
/// `Connection::subscribe` returned. Counts counter gaps as it goes and
/// ends after exactly one `Err`.
// DECISION: the state sits behind a `Mutex` (and the drop count in an
// atomic) so the sensor types stay `Sync` and unwind-safe as in the contract
// stubs, while `next`/`latest` still take `&self` for `impl Iterator for &Mic`.
pub(crate) struct Stream<T> {
    state: Mutex<State<T>>,
    dropped: AtomicU64,
    patience: Duration,
}

struct State<T> {
    rx: Receiver<Result<Message, Error>>,
    convert: Convert<T>,
    /// The counter the next message should carry.
    expected: u32,
    /// `false` until the first message sets `expected` (see [`Stream::joining`]).
    synced: bool,
    /// An error met while `latest` drained past a newer reading; the next
    /// call hands it out.
    held: Option<Error>,
    /// The one `Err` has been handed out; nothing follows.
    ended: bool,
}

impl<T> std::fmt::Debug for Stream<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stream").field("dropped", &self.dropped).finish_non_exhaustive()
    }
}

/// What one look at the channel found.
enum Taken<T> {
    Reading(T),
    /// Nothing waiting (only when not blocking).
    Empty,
    /// The link is gone: the stream's one error.
    End(Error),
    /// The error was already handed out.
    Over,
}

impl<T> Stream<T> {
    pub(crate) fn new(
        rx: Receiver<Result<Message, Error>>,
        convert: impl Fn(Message) -> Option<T> + Send + 'static,
    ) -> Self {
        Self {
            state: Mutex::new(State {
                rx,
                convert: Box::new(convert),
                // DECISION: the first message is expected to carry 0, since the
                // counter counts from the moment the sensor went on (proto docs);
                // a first counter of N means N readings were lost.
                expected: 0,
                synced: true,
                held: None,
                ended: false,
            }),
            dropped: AtomicU64::new(0),
            patience: PATIENCE,
        }
    }

    /// The same stream, waiting up to `patience` for each reading.
    pub(crate) fn with_patience(mut self, patience: Duration) -> Self {
        self.patience = patience;
        self
    }

    /// Like [`Stream::new`], for a sensor that joins a stream already
    /// running: the first counter seen is the start, not a gap.
    // DECISION: an IMU half turned on while the other half streams may see
    // a counter that did not restart at 0 (whether `ImuOn` from the holding
    // link resets it is the firmware's choice); its first counter is not
    // counted as drops.
    pub(crate) fn joining(
        rx: Receiver<Result<Message, Error>>,
        convert: impl Fn(Message) -> Option<T> + Send + 'static,
    ) -> Self {
        let stream = Self::new(rx, convert);
        lock(&stream.state).synced = false;
        stream
    }

    /// The next reading, waiting for it. `Some(Err)` once when the link
    /// is gone or silent for 2 s, then `None` for ever.
    pub(crate) fn next(&self) -> Option<Result<T, Error>> {
        let mut state = lock(&self.state);
        match self.take(&mut state, true) {
            Taken::Reading(reading) => Some(Ok(reading)),
            Taken::End(error) => Some(Err(error)),
            Taken::Over | Taken::Empty => None,
        }
    }

    /// The newest reading, dropping older ones not yet read; waits only
    /// when none is waiting.
    pub(crate) fn latest(&self) -> Result<T, Error> {
        let mut state = lock(&self.state);
        if let Some(reading) = self.newest(&mut state)? {
            return Ok(reading);
        }
        match self.take(&mut state, true) {
            Taken::Reading(reading) => Ok(reading),
            Taken::End(error) => Err(error),
            Taken::Over | Taken::Empty => Err(Error::Disconnected),
        }
    }

    /// The newest reading waiting, dropping older ones not yet read;
    /// `None` when none is. Never waits: the stream's one `Err` once,
    /// then `None` for ever.
    pub(crate) fn try_latest(&self) -> Result<Option<T>, Error> {
        match self.try_lock() {
            Some(mut state) => self.newest(&mut state),
            None => Ok(None),
        }
    }

    /// The oldest reading waiting, `None` when none is. Never waits: the
    /// stream's one `Err` once, then `None` for ever.
    pub(crate) fn try_next(&self) -> Result<Option<T>, Error> {
        let Some(mut state) = self.try_lock() else {
            return Ok(None);
        };
        match self.take(&mut state, false) {
            Taken::Reading(reading) => Ok(Some(reading)),
            Taken::End(error) => Err(error),
            Taken::Over | Taken::Empty => Ok(None),
        }
    }

    // DECISION: the `try_` reads take the state with `try_lock`: while a
    // blocking `next` or `latest` on another thread holds it, they return
    // `Ok(None)` instead of waiting up to 2 s for that call to finish.
    /// The state, unless another call holds it right now.
    fn try_lock(&self) -> Option<MutexGuard<'_, State<T>>> {
        match self.state.try_lock() {
            Ok(state) => Some(state),
            Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    }

    /// Drains what is waiting and returns the newest reading, `None` when
    /// nothing was. An error met after a reading is held for the next call.
    fn newest(&self, state: &mut State<T>) -> Result<Option<T>, Error> {
        let mut newest = None;
        loop {
            match self.take(state, false) {
                Taken::Reading(reading) => newest = Some(reading),
                Taken::Empty | Taken::Over => return Ok(newest),
                Taken::End(error) => match newest {
                    // The reading came before the error: hand it out now,
                    // the error on the next call.
                    Some(_) => {
                        state.held = Some(error);
                        return Ok(newest);
                    }
                    None => return Err(error),
                },
            }
        }
    }

    /// Readings lost so far, from counter gaps.
    pub(crate) fn dropped(&self) -> Dropped {
        Dropped(self.dropped.load(Ordering::Relaxed))
    }

    fn take(&self, state: &mut State<T>, wait: bool) -> Taken<T> {
        if let Some(error) = state.held.take() {
            state.ended = true;
            return Taken::End(error);
        }
        if state.ended {
            return Taken::Over;
        }
        let deadline = Instant::now() + self.patience;
        loop {
            let got = if wait {
                match state.rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(got) => Some(got),
                    Err(RecvTimeoutError::Timeout) => {
                        state.ended = true;
                        return Taken::End(Error::Timeout { attempts: 1 });
                    }
                    Err(RecvTimeoutError::Disconnected) => None,
                }
            } else {
                match state.rx.try_recv() {
                    Ok(got) => Some(got),
                    Err(TryRecvError::Empty) => return Taken::Empty,
                    Err(TryRecvError::Disconnected) => None,
                }
            };
            match got {
                Some(Ok(message)) => {
                    self.count(state, &message);
                    if let Some(reading) = (state.convert)(message) {
                        return Taken::Reading(reading);
                    }
                }
                Some(Err(error)) => {
                    state.ended = true;
                    return Taken::End(error);
                }
                // DECISION: a channel closed without an error (its route was
                // replaced) still yields one `Disconnected`, so every way a
                // stream ends looks the same to the caller.
                None => {
                    state.ended = true;
                    return Taken::End(Error::Disconnected);
                }
            }
        }
    }

    /// Adds the gap between the expected counter and this message's.
    fn count(&self, state: &mut State<T>, message: &Message) {
        let counter = match message {
            Message::Audio(block) => block.counter,
            Message::Imu(reading) => reading.counter,
            _ => return,
        };
        if !std::mem::replace(&mut state.synced, true) {
            state.expected = counter;
        }
        let gap = counter.wrapping_sub(state.expected);
        // DECISION: a counter behind the expected one (a gap past half the
        // range) is a restart of the sensor, not ~4 billion drops: resync
        // without counting.
        if gap <= u32::MAX / 2 {
            self.dropped.fetch_add(u64::from(gap), Ordering::Relaxed);
        }
        state.expected = counter.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{Dropped, Stream};
    use crate::Error;
    use std::sync::mpsc::{self, Sender};
    use std::thread;
    use std::time::Duration;
    use xiao_proto::message::{ImuReading, Message};

    type Tx = Sender<Result<Message, Error>>;

    /// A stream of IMU counters, and the sender playing the link.
    fn counters() -> (Tx, Stream<u32>) {
        let (tx, rx) = mpsc::channel();
        let stream = Stream::new(rx, |message| match message {
            Message::Imu(reading) => Some(reading.counter),
            _ => None,
        });
        (tx, stream)
    }

    fn reading(counter: u32) -> Message {
        Message::Imu(ImuReading { counter, accel: Some([0, 0, 0]), gyro: None })
    }

    #[test]
    fn api_3_stream_and_latest() {
        let (tx, stream) = counters();
        for counter in 0..3 {
            tx.send(Ok(reading(counter))).unwrap();
        }
        assert_eq!(stream.next().unwrap().unwrap(), 0, "in order");
        assert_eq!(stream.next().unwrap().unwrap(), 1);

        for counter in 3..6 {
            tx.send(Ok(reading(counter))).unwrap();
        }
        assert_eq!(stream.latest().unwrap(), 5, "newest of 2..=5");
        assert_eq!(stream.dropped(), Dropped(0), "skipped by latest is not dropped");

        // Nothing waiting: latest blocks for the first one to come.
        let late = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            tx.send(Ok(reading(6))).unwrap();
            tx
        });
        assert_eq!(stream.latest().unwrap(), 6);
        drop(late.join().unwrap());
    }

    #[test]
    fn api_4_counter_gap_is_reported() {
        let (tx, stream) = counters();
        for counter in [0, 1, 5, 6, 10] {
            tx.send(Ok(reading(counter))).unwrap();
        }
        for _ in 0..5 {
            stream.next().unwrap().unwrap();
        }
        assert_eq!(stream.dropped(), Dropped(3 + 3), "2..=4 and 7..=9 were lost");

        // A first counter of N: N readings were lost since the sensor went on.
        let (tx, stream) = counters();
        tx.send(Ok(reading(4))).unwrap();
        stream.latest().unwrap();
        assert_eq!(stream.dropped(), Dropped(4));
    }

    #[test]
    fn counter_wrap_is_not_a_drop() {
        // 0, then a counter behind it (a restart: resync, not counted),
        // then across the wrap with nothing missing.
        let (tx, stream) = counters();
        for counter in [0, u32::MAX - 1, u32::MAX, 0, 1] {
            tx.send(Ok(reading(counter))).unwrap();
        }
        let got: Vec<u32> = (0..5).map(|_| stream.next().unwrap().unwrap()).collect();
        assert_eq!(got, [0, u32::MAX - 1, u32::MAX, 0, 1]);
        assert_eq!(stream.dropped(), Dropped(0));

        // A gap across the wrap counts what is missing, no more.
        let (tx, stream) = counters();
        for counter in [0, u32::MAX - 1, 2] {
            tx.send(Ok(reading(counter))).unwrap();
        }
        for _ in 0..3 {
            stream.next().unwrap().unwrap();
        }
        assert_eq!(stream.dropped(), Dropped(3), "u32::MAX, 0, 1 lost");
    }

    #[test]
    fn dead_link_ends_the_stream_with_one_error() {
        // The link reports its death: one Err, then the end.
        let (tx, stream) = counters();
        tx.send(Ok(reading(0))).unwrap();
        tx.send(Err(Error::Disconnected)).unwrap();
        drop(tx);
        assert_eq!(stream.next().unwrap().unwrap(), 0);
        assert!(matches!(stream.next(), Some(Err(Error::Disconnected))));
        assert!(stream.next().is_none());
        assert!(stream.next().is_none(), "and stays ended");
        assert!(matches!(stream.latest(), Err(Error::Disconnected)), "latest never blocks");

        // The channel just closes: still exactly one Err, never a hang.
        let (tx, stream) = counters();
        drop(tx);
        assert!(matches!(stream.next(), Some(Err(Error::Disconnected))));
        assert!(stream.next().is_none());

        // latest drains past a reading to the error: reading first, then
        // the one Err, then the end.
        let (tx, stream) = counters();
        tx.send(Ok(reading(0))).unwrap();
        tx.send(Ok(reading(1))).unwrap();
        tx.send(Err(Error::Disconnected)).unwrap();
        drop(tx);
        assert_eq!(stream.latest().unwrap(), 1);
        assert!(matches!(stream.next(), Some(Err(Error::Disconnected))));
        assert!(stream.next().is_none());
    }

    #[test]
    fn patience_is_set_per_link() {
        // A BLE link gives its streams time to outlast a reconnect.
        let (tx, stream) = counters();
        let stream = stream.with_patience(Duration::from_millis(600));
        let late = thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            tx.send(Ok(reading(0))).unwrap();
            tx
        });
        assert_eq!(stream.next().unwrap().unwrap(), 0, "waited past 300 ms");
        drop(late.join().unwrap());
    }

    #[test]
    fn silent_link_ends_the_stream_with_timeout() {
        // The link stays up but nothing arrives: one Timeout, then the end.
        let (tx, mut stream) = counters();
        stream.patience = Duration::from_millis(100);
        let start = std::time::Instant::now();
        assert!(matches!(stream.next(), Some(Err(Error::Timeout { attempts: 1 }))));
        assert!(start.elapsed() < Duration::from_secs(1), "gave up in time");
        assert!(stream.next().is_none());
        tx.send(Ok(reading(0))).unwrap();
        assert!(stream.next().is_none(), "ended streams stay ended");

        let (_tx, mut stream) = counters();
        stream.patience = Duration::from_millis(100);
        assert!(matches!(stream.latest(), Err(Error::Timeout { attempts: 1 })));
    }

    #[test]
    fn try_latest_returns_the_newest_and_counts_the_skipped_as_read_not_dropped() {
        let (tx, stream) = counters();
        for counter in 0..4 {
            tx.send(Ok(reading(counter))).unwrap();
        }
        assert_eq!(stream.try_latest().unwrap(), Some(3), "newest of 0..=3");
        assert_eq!(stream.dropped(), Dropped(0), "0..=2 were read, not lost");
        assert_eq!(stream.try_latest().unwrap(), None, "the skipped ones are gone");

        // A real gap still counts.
        tx.send(Ok(reading(6))).unwrap();
        assert_eq!(stream.try_latest().unwrap(), Some(6));
        assert_eq!(stream.dropped(), Dropped(2), "4 and 5 were lost");

        // try_next takes the oldest, one at a time.
        for counter in 7..9 {
            tx.send(Ok(reading(counter))).unwrap();
        }
        assert_eq!(stream.try_next().unwrap(), Some(7));
        assert_eq!(stream.try_next().unwrap(), Some(8));
        assert_eq!(stream.try_next().unwrap(), None);
    }

    #[test]
    fn try_latest_reports_a_dead_link_once_then_none() {
        // A reading queued before the error: the reading now, then the
        // error once, then nothing, never waiting.
        let (tx, stream) = counters();
        tx.send(Ok(reading(0))).unwrap();
        tx.send(Ok(reading(1))).unwrap();
        tx.send(Err(Error::Disconnected)).unwrap();
        drop(tx);
        assert_eq!(stream.try_latest().unwrap(), Some(1));
        assert!(matches!(stream.try_latest(), Err(Error::Disconnected)));
        let start = std::time::Instant::now();
        for _ in 0..3 {
            assert_eq!(stream.try_latest().unwrap(), None);
            assert_eq!(stream.try_next().unwrap(), None);
        }
        assert!(start.elapsed() < Duration::from_millis(10), "an ended stream never waits");

        // The channel just closes: one Disconnected, then nothing.
        let (tx, stream) = counters();
        drop(tx);
        assert!(matches!(stream.try_latest(), Err(Error::Disconnected)));
        assert_eq!(stream.try_latest().unwrap(), None);

        // try_next meets the error in its turn, after the readings before it.
        let (tx, stream) = counters();
        tx.send(Ok(reading(0))).unwrap();
        tx.send(Err(Error::Timeout { attempts: 3 })).unwrap();
        assert_eq!(stream.try_next().unwrap(), Some(0));
        assert!(matches!(stream.try_next(), Err(Error::Timeout { attempts: 3 })));
        assert_eq!(stream.try_next().unwrap(), None);
    }
}
