//! Runs on the XIAO nRF52840 Sense: powers and reads the sensors the
//! API asks for and streams the readings back.
//!
//! This library holds the hardware-free half (`sensors`, `imu`), tested
//! on the host. The board half (`board`, `mic`, `link`) lives in the
//! binary, `main.rs`.
//!
//! Depends on: `xiao-proto`.
#![cfg_attr(target_os = "none", no_std)]

pub mod imu;
pub mod sensors;
