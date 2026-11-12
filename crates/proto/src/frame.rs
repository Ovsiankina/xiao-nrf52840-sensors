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
    let _ = (value, buf);
    todo!()
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
        let _ = (byte, &mut self.buf, &mut self.len, &mut self.overflowed);
        todo!()
    }
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn proto_1_round_trip_every_command_and_message() {
        todo!()
    }

    #[test]
    fn proto_2_full_audio_block_fits_one_ble_packet() {
        todo!()
    }

    #[test]
    fn proto_3_garbage_then_frame_decodes() {
        todo!()
    }

    #[test]
    fn proto_3_too_long_then_recovers() {
        todo!()
    }

    #[test]
    fn two_frames_back_to_back() {
        todo!()
    }
}
