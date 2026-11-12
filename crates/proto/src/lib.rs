//! Messages and framing shared by the API and the firmware.
//!
//! Both sides build against this crate, so they cannot disagree about
//! what is on the wire. `no_std`: it runs on the board too.
//!
//! Depends on: nothing in this workspace.
#![cfg_attr(not(test), no_std)]

pub mod frame;
pub mod message;
