//! The USB link: finds the XIAO's serial port by its USB id.
//!
//! Depends on: `link`.

use crate::Error;
use crate::link::{Baud, open_serial};
use serialport::SerialPortType;

/// Seeed Studio's USB vendor id, which the firmware reports.
const SEEED_VID: u16 = 0x2886;

/// The speed asked of the port. A USB serial port (CDC-ACM) ignores it.
const USB_BAUD: Baud = Baud(115_200);

/// The link over the board's USB plug.
#[derive(Debug)]
pub struct Usb {
    pub(super) connection: super::Connection,
}

impl Usb {
    /// Opens the first XIAO found on USB (API-1).
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] when no XIAO is plugged in; [`Error::Io`]
    /// when its port cannot be opened.
    pub fn open() -> Result<Self, Error> {
        let ports = serialport::available_ports().map_err(|e| Error::Io(e.into()))?;
        // DECISION: the first port with Seeed's vendor id, in the order the OS
        // lists them; the product name is not checked, as in the plan.
        let port = ports
            .into_iter()
            .find(|port| {
                matches!(&port.port_type, SerialPortType::UsbPort(usb) if usb.vid == SEEED_VID)
            })
            .ok_or(Error::NotFound)?;
        Ok(Self { connection: open_serial(&port.port_name, USB_BAUD)? })
    }
}
