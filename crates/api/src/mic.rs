//! The microphone: turn it on over a link, get a stream of audio.
//!
//! Depends on: `link`, `stream`, `error`.

use crate::{Dropped, Error, link::AsLink};
use std::marker::PhantomData;

/// A frequency, in cycles (or samples) per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hertz(pub u32);

/// A short run of audio: 96 samples, 6 ms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioBlock {
    _private: (),
}

impl AudioBlock {
    /// The samples, 16-bit signed, mono.
    #[must_use]
    pub fn samples(&self) -> &[i16] {
        todo!()
    }

    /// The sample rate: 16 kHz.
    #[must_use]
    pub fn rate(&self) -> Hertz {
        todo!()
    }
}

/// The microphone, on. Iterate over `&mic` for audio; dropping it
/// turns the microphone off (FW-6).
#[derive(Debug)]
pub struct Mic<'l> {
    _link: PhantomData<&'l ()>,
}

impl<'l> Mic<'l> {
    /// Turns the microphone on over `link` (API-1).
    ///
    /// # Errors
    ///
    /// [`Error::Busy`] when another link holds it, [`Error::TooSlow`]
    /// when audio does not fit this link, [`Error::Timeout`] when the
    /// board does not answer.
    pub fn on(link: &'l impl AsLink) -> Result<Self, Error> {
        let _ = link;
        todo!()
    }

    /// The newest block, skipping any older ones not yet read (API-3).
    ///
    /// # Errors
    ///
    /// [`Error::Disconnected`] when the link is gone.
    pub fn latest(&self) -> Result<AudioBlock, Error> {
        todo!()
    }

    /// Blocks lost since the microphone was turned on (API-4).
    #[must_use]
    pub fn dropped(&self) -> Dropped {
        todo!()
    }
}

/// Every block in order, waiting for the next one (API-3). Ends after
/// one `Err` when the link is gone.
impl Iterator for &Mic<'_> {
    type Item = Result<AudioBlock, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

impl Drop for Mic<'_> {
    fn drop(&mut self) {
        // Contract: sends `MicOff` once (FW-6). Filled in at step 5.
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn mic_iterates_audio_blocks() {
        todo!()
    }

    #[test]
    fn drop_sends_off_once() {
        todo!()
    }

    #[test]
    fn busy_names_the_holding_link() {
        todo!()
    }
}
