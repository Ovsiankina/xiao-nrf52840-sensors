//! What the developer calls: pick the XIAO's sensors, configure them,
//! read them. Runs off the board and talks to the firmware over a link.
//!
//! Depends on: `xiao-proto`.

pub mod error;
pub mod imu;
pub mod link;
pub mod mic;
pub mod stream;
