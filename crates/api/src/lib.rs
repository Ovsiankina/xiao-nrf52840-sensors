//! What the developer calls: pick the XIAO's sensors, configure them,
//! read them. Runs off the board and talks to the firmware over a link.
//!
//! ```no_run
//! use xiao_api::{Mic, Usb};
//!
//! let usb = Usb::open()?;
//! let mic = Mic::on(&usb)?;
//! for block in &mic {
//!     let pcm: &[i16] = block?.samples();
//! }
//! # Ok::<(), xiao_api::Error>(())
//! ```
//!
//! Depends on: `xiao-proto`.

pub mod error;
pub mod imu;
pub mod link;
pub mod mic;
pub mod stream;

pub use error::Error;
pub use imu::{Accel, Acceleration, Dps, G, Gyro, Imu, Motion, Rotation};
pub use link::{Baud, Ble, BytesPerSec, Uart, Usb};
pub use mic::{AudioBlock, Hertz, Mic};
pub use stream::Dropped;
pub use xiao_proto::message::{AccelRange, GyroRange, Link, Rate};
