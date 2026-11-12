//! The connection to the firmware. One type per link (`Usb`, `Ble`,
//! `Uart`), each with `open()`; all of them send commands and receive
//! messages the same way.
//!
//! Depends on: `xiao-proto::frame`, `error`.

pub mod ble;
pub mod uart;
pub mod usb;
