//! The microphone: turn it on over a link, get a stream of audio.
//!
//! Depends on: `link`, `stream`, `error`.

use crate::link::{self, AsLink, Connection};
use crate::stream::Stream;
use crate::{Dropped, Error};
use xiao_proto::message::{Command, MIC_SAMPLE_RATE_HZ, Message, Sensor};

/// A frequency, in cycles (or samples) per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hertz(pub u32);

/// A short run of audio: 96 samples, 6 ms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioBlock {
    samples: Vec<i16>,
}

impl AudioBlock {
    /// The samples, 16-bit signed, mono.
    #[must_use]
    pub fn samples(&self) -> &[i16] {
        &self.samples
    }

    /// The sample rate: 16 kHz.
    #[must_use]
    pub fn rate(&self) -> Hertz {
        Hertz(MIC_SAMPLE_RATE_HZ)
    }
}

/// The microphone, on. Iterate over `&mic` for audio; dropping it
/// turns the microphone off (FW-6).
///
/// It keeps its link open: the link closes once the link value and every
/// sensor on it are dropped.
#[derive(Debug)]
pub struct Mic {
    connection: Connection,
    stream: Stream<AudioBlock>,
}

impl Mic {
    /// Turns the microphone on over `link` (API-1).
    ///
    /// # Errors
    ///
    /// [`Error::Busy`] when another link holds it, [`Error::TooSlow`]
    /// when audio does not fit this link, [`Error::Timeout`] when the
    /// board does not answer.
    pub fn on(link: &impl AsLink) -> Result<Self, Error> {
        let (connection, kind) = link::parts(link);
        let mut held = connection.held();
        if held.mic {
            // DECISION: a second `Mic` on the link that holds it is `Busy`
            // naming that link: two values would share one stream, and the
            // first one dropped would switch the other's audio off.
            return Err(Error::Busy { held_by: kind });
        }
        // Subscribed before asking, so no block sent after `On` is lost.
        let rx = connection.subscribe(Sensor::Mic);
        if let Err(error) = connection.request(Command::MicOn) {
            if matches!(error, Error::Timeout { .. }) {
                // DECISION: an unanswered `MicOn` is undone with a `MicOff`
                // that nobody waits for: the board may have turned the mic on
                // and only its `On` was late, and no value would turn it off.
                let _ = connection.tell(Command::MicOff);
            }
            return Err(error);
        }
        held.mic = true;
        Ok(Self { connection: connection.share(), stream: Stream::new(rx, audio) })
    }

    /// The newest block, skipping any older ones not yet read (API-3).
    ///
    /// # Errors
    ///
    /// [`Error::Disconnected`] when the link is gone.
    pub fn latest(&self) -> Result<AudioBlock, Error> {
        self.stream.latest()
    }

    /// The oldest block already here, `None` when none is waiting. Never
    /// waits (API-3).
    ///
    /// # Errors
    ///
    /// Once, why the link ended: [`Error::Disconnected`] when it is gone,
    /// [`Error::Timeout`] when BLE ran out of reconnect attempts. Then
    /// `Ok(None)` for ever.
    pub fn try_next(&self) -> Result<Option<AudioBlock>, Error> {
        self.stream.try_next()
    }

    /// Blocks lost since the microphone was turned on (API-4).
    #[must_use]
    pub fn dropped(&self) -> Dropped {
        self.stream.dropped()
    }
}

/// Every block in order, waiting for the next one (API-3). Ends after
/// one `Err` when the link is gone.
// DECISION: `copy_iterator` allowed: the contract iterates over `&Mic`
// (`for block in &mic`), and every shared reference is `Copy`.
#[allow(clippy::copy_iterator, reason = "the contract's `for block in &mic`")]
impl Iterator for &Mic {
    type Item = Result<AudioBlock, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        self.stream.next()
    }
}

impl Drop for Mic {
    fn drop(&mut self) {
        let mut held = self.connection.held();
        held.mic = false;
        // DECISION: the drop waits for the board's `Off` (up to 2 s) and
        // ignores a failure: `drop` cannot return it, and a board that lost
        // the link frees the sensor on its own.
        let _ = self.connection.request(Command::MicOff);
    }
}

/// The samples of an audio message; nothing for any other.
fn audio(message: Message) -> Option<AudioBlock> {
    let Message::Audio(block) = message else {
        return None;
    };
    let samples =
        block.pcm.chunks_exact(2).map(|pair| i16::from_le_bytes([pair[0], pair[1]])).collect();
    Some(AudioBlock { samples })
}

#[cfg(test)]
mod tests {
    use super::{Hertz, Mic};
    use crate::link::{answered, memory_usb};
    use crate::{Dropped, Error};
    use std::thread;
    use std::time::{Duration, Instant};
    use xiao_proto::message::{AudioBlock, Command, Link, Message, Refusal, Sensor};

    const QUIET: Duration = Duration::from_millis(200);

    /// An audio message whose samples are `first`, `first + 1`, ... (96).
    fn block(counter: u32, first: i16) -> Message {
        let pcm = (0..96).flat_map(|i| (first + i).to_le_bytes()).collect();
        Message::Audio(AudioBlock { counter, pcm })
    }

    #[test]
    fn mic_iterates_audio_blocks() {
        let (usb, mut board) = memory_usb();
        let (mic, command) = answered(&board, || Mic::on(&usb));
        let mic = mic.unwrap();
        assert_eq!(command, Command::MicOn);

        board.send(&block(0, -48));
        board.send(&block(1, 1000));
        board.send(&block(3, 0));

        let mut blocks = &mic;
        let first = blocks.next().unwrap().unwrap();
        assert_eq!(first.samples().len(), 96);
        assert_eq!(first.samples()[0], -48, "little-endian, sign kept");
        assert_eq!(first.samples()[95], 47);
        assert_eq!(first.rate(), Hertz(16_000));
        assert_eq!(blocks.next().unwrap().unwrap().samples()[0], 1000);
        assert_eq!(mic.latest().unwrap().samples()[0], 0);
        assert_eq!(mic.dropped(), Dropped(1), "block 2 was lost");

        answered(&board, || drop(mic));
    }

    #[test]
    fn drop_sends_off_once() {
        let (usb, mut board) = memory_usb();
        let mic = answered(&board, || Mic::on(&usb)).0.unwrap();

        let ((), command) = answered(&board, || drop(mic));
        assert_eq!(command, Command::MicOff);
        assert_eq!(board.command(QUIET), None, "and nothing after it");

        // Off means free again: the same link may turn it on anew.
        let (again, command) = answered(&board, || Mic::on(&usb));
        assert_eq!(command, Command::MicOn);
        answered(&board, || drop(again.unwrap()));
    }

    #[test]
    fn busy_names_the_holding_link() {
        let (usb, mut board) = memory_usb();

        // The board says BLE holds the mic.
        let mut firmware = board.clone();
        let refuse = thread::spawn(move || {
            let command = firmware.command(Duration::from_secs(5));
            let busy = Refusal::Busy { held_by: Link::Ble };
            firmware.send(&Message::Refused { sensor: Sensor::Mic, reason: busy });
            command
        });
        let result = Mic::on(&usb);
        assert_eq!(refuse.join().unwrap(), Some(Command::MicOn));
        assert!(matches!(result, Err(Error::Busy { held_by: Link::Ble })), "{result:?}");
        assert_eq!(board.command(QUIET), None, "no value, so no MicOff");

        // A second Mic on the link that already holds it: busy, held by USB,
        // and nothing is sent to the board.
        let mic = answered(&board, || Mic::on(&usb)).0.unwrap();
        let second = Mic::on(&usb);
        assert!(matches!(second, Err(Error::Busy { held_by: Link::Usb })), "{second:?}");
        assert_eq!(board.command(QUIET), None);
        answered(&board, || drop(mic));
    }

    #[test]
    fn timed_out_on_is_undone() {
        let (usb, mut board) = memory_usb();
        let result = Mic::on(&usb);
        assert!(matches!(result, Err(Error::Timeout { attempts: 1 })), "{result:?}");
        assert_eq!(board.command(QUIET), Some(Command::MicOn));
        assert_eq!(board.command(QUIET), Some(Command::MicOff), "the late On is undone");
        assert_eq!(board.command(QUIET), None);
    }

    #[test]
    fn try_next_hands_blocks_out_in_order() {
        let (usb, mut board) = memory_usb();
        let mic = answered(&board, || Mic::on(&usb)).0.unwrap();

        let start = Instant::now();
        assert_eq!(mic.try_next().unwrap(), None);
        assert!(start.elapsed() < Duration::from_millis(10), "waited {:?}", start.elapsed());

        board.send(&block(0, 0));
        board.send(&block(1, 100));
        board.send(&block(3, 300));
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut firsts = Vec::new();
        while firsts.len() < 3 {
            if let Some(block) = mic.try_next().unwrap() {
                firsts.push(block.samples()[0]);
            } else {
                assert!(Instant::now() < deadline, "only {firsts:?} arrived");
                thread::sleep(Duration::from_millis(1));
            }
        }
        assert_eq!(firsts, [0, 100, 300], "oldest first, none skipped");
        assert_eq!(mic.try_next().unwrap(), None);
        assert_eq!(mic.dropped(), Dropped(1), "block 2 was lost");
        answered(&board, || drop(mic));
    }

    #[test]
    fn refusal_nobody_asked_for_ends_the_stream() {
        // After a reconnect the board refuses the mic it was asked for again:
        // the stream ends with that refusal instead of waiting for ever.
        let (usb, mut board) = memory_usb();
        let mic = answered(&board, || Mic::on(&usb)).0.unwrap();
        let busy = Refusal::Busy { held_by: Link::Usb };
        board.send(&Message::Refused { sensor: Sensor::Mic, reason: busy });

        let got = (&mic).next();
        assert!(matches!(got, Some(Err(Error::Busy { held_by: Link::Usb }))), "{got:?}");
        assert!((&mic).next().is_none());
        answered(&board, || drop(mic));
    }
}
