//! The connection to the firmware. One type per link (`Usb`, `Ble`,
//! `Uart`), each with `open()`; all of them send commands and receive
//! messages the same way.
//!
//! Depends on: `xiao-proto::frame`, `error`.

pub mod ble;
pub mod uart;
pub mod usb;

pub use ble::{Ble, BleBuilder};
pub use uart::{Uart, UartBuilder};
pub use usb::Usb;

/// A serial speed, in bits per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Baud(pub u32);

/// A data rate, in bytes per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BytesPerSec(pub u32);

/// The shared inside of every link: sends commands, routes what comes
/// back to the sensor it belongs to.
#[derive(Debug)]
pub struct Connection {
    _private: (),
}

mod sealed {
    pub trait Sealed {
        fn connection(&self) -> &super::Connection;
    }
}

/// Anything a sensor can be turned on over: [`Usb`], [`Ble`], [`Uart`]
/// (API-1). Cannot be implemented outside this crate.
pub trait AsLink: sealed::Sealed {}

impl sealed::Sealed for Usb {
    fn connection(&self) -> &Connection {
        todo!()
    }
}
impl AsLink for Usb {}

impl sealed::Sealed for Ble {
    fn connection(&self) -> &Connection {
        todo!()
    }
}
impl AsLink for Ble {}

impl sealed::Sealed for Uart {
    fn connection(&self) -> &Connection {
        todo!()
    }
}
impl AsLink for Uart {}

#[cfg(test)]
mod tests {
    #[test]
    fn api_1_sensor_goes_on_over_the_link_it_was_given() {
        todo!()
    }

    #[test]
    fn request_times_out_without_reply() {
        todo!()
    }

    #[test]
    fn refusal_becomes_error() {
        todo!()
    }
}
