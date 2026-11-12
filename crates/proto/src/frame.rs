//! How a message becomes bytes on a link and back: postcard encoding
//! with COBS framing, so a reader can find message edges in a byte
//! stream after a drop.
//!
//! Depends on: `message`.

use serde::{Serialize, de::DeserializeOwned};

/// The longest frame, in bytes, including the ending zero.
pub const MAX_FRAME: usize = 256;

/// Why a frame could not be written or read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// The frame is longer than [`MAX_FRAME`].
    TooLong,
    /// The bytes are not a valid frame.
    Malformed,
}

/// Writes `value` as one frame into `buf` and returns the bytes to
/// send; the last one is zero (PROTO-1, PROTO-2).
///
/// # Errors
///
/// [`FrameError::TooLong`] when the frame does not fit.
pub fn encode<'b, T: Serialize>(
    value: &T,
    buf: &'b mut [u8; MAX_FRAME],
) -> Result<&'b [u8], FrameError> {
    postcard::to_slice_cobs(value, buf).map(|frame| &*frame).map_err(|_| FrameError::TooLong)
}

/// Finds frames in a stream of bytes (PROTO-3).
#[derive(Debug)]
pub struct Decoder {
    buf: [u8; MAX_FRAME],
    len: usize,
    overflowed: bool,
}

impl Decoder {
    /// An empty decoder.
    #[must_use]
    pub const fn new() -> Self {
        Self { buf: [0; MAX_FRAME], len: 0, overflowed: false }
    }

    /// Takes the next byte from the link. Returns `None` until a frame
    /// ends, then the decoded value or why it failed. After a failure
    /// the decoder is ready for the next frame (PROTO-3).
    pub fn feed<T: DeserializeOwned>(&mut self, byte: u8) -> Option<Result<T, FrameError>> {
        if byte != 0 {
            match self.buf.get_mut(self.len) {
                Some(slot) => {
                    *slot = byte;
                    self.len += 1;
                }
                None => self.overflowed = true,
            }
            return None;
        }

        // A zero ends the frame, good or bad; start clean for the next.
        let (len, overflowed) = (self.len, self.overflowed);
        self.len = 0;
        self.overflowed = false;

        if overflowed {
            return Some(Err(FrameError::TooLong));
        }
        if len == 0 {
            // DECISION: a lone zero is an empty gap between frames, not an error.
            return None;
        }
        let frame = self.buf.get_mut(..len)?;
        Some(postcard::from_bytes_cobs(frame).map_err(|_| FrameError::Malformed))
    }
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{
        AUDIO_BLOCK_BYTES, AccelConfig, AudioBlock, Command, GyroConfig, ImuConfig, ImuReading,
        Link, Message, Refusal, Sensor,
    };
    use serde::de::DeserializeOwned;

    fn decode_all<T: DeserializeOwned>(bytes: &[u8]) -> Vec<Result<T, FrameError>> {
        let mut decoder = Decoder::new();
        bytes.iter().filter_map(|&b| decoder.feed(b)).collect()
    }

    fn clone_via_wire<T: Serialize + DeserializeOwned>(value: &T) -> T {
        let mut buf = [0; MAX_FRAME];
        let frame = encode(value, &mut buf).unwrap().to_vec();
        assert_eq!(frame.last(), Some(&0), "a frame ends with a zero");
        decode_all::<T>(&frame).pop().unwrap().unwrap()
    }

    fn full_audio_block() -> AudioBlock {
        let mut pcm = heapless::Vec::new();
        // Worst case for COBS: no zero bytes at all.
        pcm.resize(AUDIO_BLOCK_BYTES, 0xFF).unwrap();
        AudioBlock { counter: u32::MAX, pcm }
    }

    #[test]
    fn proto_1_round_trip_every_command_and_message() {
        let config =
            ImuConfig { accel: Some(AccelConfig::default()), gyro: Some(GyroConfig::default()) };
        for command in [Command::MicOn, Command::MicOff, Command::ImuOn(config), Command::ImuOff] {
            assert_eq!(clone_via_wire(&command), command);
        }
        let messages = [
            Message::On(Sensor::Mic),
            Message::Off(Sensor::Imu),
            Message::Refused { sensor: Sensor::Mic, reason: Refusal::Busy { held_by: Link::Ble } },
            Message::Refused {
                sensor: Sensor::Mic,
                reason: Refusal::TooSlow { needs: 34_000, link_has: 11_520 },
            },
            Message::Audio(full_audio_block()),
            Message::Imu(ImuReading {
                counter: 7,
                accel: Some([i16::MIN, 0, i16::MAX]),
                gyro: None,
            }),
        ];
        for message in messages {
            assert_eq!(clone_via_wire(&message), message);
        }
    }

    #[test]
    fn proto_2_full_audio_block_fits_one_ble_packet() {
        let mut buf = [0; MAX_FRAME];
        let frame = encode(&Message::Audio(full_audio_block()), &mut buf).unwrap();
        assert!(frame.len() <= 244, "frame is {} bytes", frame.len());
    }

    #[test]
    fn proto_3_garbage_then_frame_decodes() {
        let mut buf = [0; MAX_FRAME];
        let good = encode(&Command::MicOn, &mut buf).unwrap();
        let mut bytes = vec![0x03, 0xFF, 0x17, 0x00]; // garbage, ended by a zero
        bytes.extend_from_slice(good);
        let decoded = decode_all::<Command>(&bytes);
        assert_eq!(decoded.last(), Some(&Ok(Command::MicOn)));
        assert!(decoded.iter().rev().skip(1).all(Result::is_err));
    }

    #[test]
    fn proto_3_too_long_then_recovers() {
        let mut buf = [0; MAX_FRAME];
        let good = encode(&Command::ImuOff, &mut buf).unwrap();
        let mut bytes = vec![0x55; MAX_FRAME + 10];
        bytes.push(0);
        bytes.extend_from_slice(good);
        assert_eq!(
            decode_all::<Command>(&bytes),
            vec![Err(FrameError::TooLong), Ok(Command::ImuOff)]
        );
    }

    #[test]
    fn two_frames_back_to_back() {
        let (mut a, mut b) = ([0; MAX_FRAME], [0; MAX_FRAME]);
        let mut bytes = encode(&Command::MicOn, &mut a).unwrap().to_vec();
        bytes.extend_from_slice(encode(&Command::MicOff, &mut b).unwrap());
        assert_eq!(decode_all::<Command>(&bytes), vec![Ok(Command::MicOn), Ok(Command::MicOff)]);
    }
}
