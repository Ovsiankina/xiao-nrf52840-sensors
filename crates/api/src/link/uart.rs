//! The UART link: a named serial port at a chosen baud (default 921600).
//!
//! Depends on: `link`.

use crate::{
    Error,
    link::{Baud, open_serial},
};
use xiao_proto::message::UART_DEFAULT_BAUD;

/// The link over the board's TX/RX pins.
#[derive(Debug)]
pub struct Uart {
    pub(super) connection: super::Connection,
}

impl Uart {
    /// Starts a UART link on a serial port such as `/dev/ttyUSB0`
    /// (API-1). The speed defaults to 921600 baud (FW-3).
    pub fn port(name: &str) -> UartBuilder {
        UartBuilder { port: name.to_owned(), baud: Baud(UART_DEFAULT_BAUD) }
    }
}

/// Settings for a [`Uart`] link before it is opened.
#[derive(Debug)]
pub struct UartBuilder {
    port: String,
    baud: Baud,
}

impl UartBuilder {
    /// Sets the speed. Must match the firmware's (FW-3).
    #[must_use]
    pub fn baud(self, baud: Baud) -> Self {
        Self { baud, ..self }
    }

    /// Opens the port.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when the port cannot be opened.
    pub fn open(self) -> Result<Uart, Error> {
        Ok(Uart { connection: open_serial(&self.port, self.baud)? })
    }
}
