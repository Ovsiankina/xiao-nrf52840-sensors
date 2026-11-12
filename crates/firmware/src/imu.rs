//! The LSM6DS3TR-C: the few registers needed to switch the
//! accelerometer and gyroscope on and off, set rate and range, and
//! read them. Generic over the I2C bus, so it is tested on the host.
//!
//! Depends on: `xiao-proto::message` (rates, ranges, reading types).
