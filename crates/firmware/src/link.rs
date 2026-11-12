//! The three ways in: USB, BLE, UART. Each turns incoming bytes into
//! commands and outgoing messages into bytes.
//!
//! Depends on: `xiao-proto::frame`.

mod ble;
mod uart;
mod usb;
