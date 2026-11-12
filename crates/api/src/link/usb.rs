//! The USB link: finds the XIAO's serial port by its USB id.
//!
//! Depends on: `link`.

use crate::Error;

/// The link over the board's USB plug.
#[derive(Debug)]
pub struct Usb {
    _private: (),
}

impl Usb {
    /// Opens the first XIAO found on USB (API-1).
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] when no XIAO is plugged in; [`Error::Io`]
    /// when its port cannot be opened.
    pub fn open() -> Result<Self, Error> {
        todo!()
    }
}
