//! The UART link: a named serial port at a chosen baud (default 921600).
//!
//! Depends on: `link`.

use crate::{Error, link::Baud};

/// The link over the board's TX/RX pins.
#[derive(Debug)]
pub struct Uart {
    _private: (),
}

impl Uart {
    /// Starts a UART link on a serial port such as `/dev/ttyUSB0`
    /// (API-1). The speed defaults to 921600 baud (FW-3).
    pub fn port(name: &str) -> UartBuilder {
        let _ = name;
        todo!()
    }
}

/// Settings for a [`Uart`] link before it is opened.
#[derive(Debug)]
pub struct UartBuilder {
    _private: (),
}

impl UartBuilder {
    /// Sets the speed. Must match the firmware's (FW-3).
    #[must_use]
    pub fn baud(self, baud: Baud) -> Self {
        let _ = baud;
        todo!()
    }

    /// Opens the port.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when the port cannot be opened.
    pub fn open(self) -> Result<Uart, Error> {
        todo!()
    }
}
