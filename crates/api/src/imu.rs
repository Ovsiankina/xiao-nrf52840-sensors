//! The IMU: accelerometer, gyroscope, or both, turned on over one link;
//! each gives a stream of readings.
//!
//! Depends on: `link`, `stream`, `error`.

use crate::{Dropped, Error, link::AsLink};
use std::marker::PhantomData;
use xiao_proto::message::{AccelRange, GyroRange, Rate};

/// An acceleration, in g (1 g = gravity at rest).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct G(pub f32);

/// A rotation speed, in degrees per second.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Dps(pub f32);

/// One accelerometer reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Acceleration {
    /// Along the board's x axis.
    pub x: G,
    /// Along the board's y axis.
    pub y: G,
    /// Along the board's z axis.
    pub z: G,
}

/// One gyroscope reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rotation {
    /// Around the board's x axis.
    pub x: Dps,
    /// Around the board's y axis.
    pub y: Dps,
    /// Around the board's z axis.
    pub z: Dps,
}

/// One reading of both halves, taken at the same instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    /// The accelerometer's half.
    pub accel: Acceleration,
    /// The gyroscope's half.
    pub gyro: Rotation,
}

/// The gyroscope, on. Iterate over `&gyro` for readings; dropping it
/// turns the gyroscope off (FW-6).
#[derive(Debug)]
pub struct Gyro<'l> {
    _link: PhantomData<&'l ()>,
}

impl<'l> Gyro<'l> {
    /// Turns the gyroscope on over `link` with the default settings:
    /// 104 Hz, smallest range (API-1).
    ///
    /// # Errors
    ///
    /// [`Error::Busy`] when another link holds the IMU (FW-1, FW-2),
    /// [`Error::TooSlow`] when the stream does not fit this link,
    /// [`Error::Timeout`] when the board does not answer.
    pub fn on(link: &'l impl AsLink) -> Result<Self, Error> {
        let _ = link;
        todo!()
    }

    /// Starts from the defaults with another rate.
    pub fn rate(rate: Rate) -> GyroBuilder {
        let _ = rate;
        todo!()
    }

    /// Starts from the defaults with another range.
    pub fn range(range: GyroRange) -> GyroBuilder {
        let _ = range;
        todo!()
    }

    /// The newest reading, skipping any older ones not yet read (API-3).
    ///
    /// # Errors
    ///
    /// [`Error::Disconnected`] when the link is gone.
    pub fn latest(&self) -> Result<Rotation, Error> {
        todo!()
    }

    /// Readings lost since it was turned on (API-4).
    #[must_use]
    pub fn dropped(&self) -> Dropped {
        todo!()
    }
}

/// Every reading in order, waiting for the next one (API-3). Ends after
/// one `Err` when the link is gone.
impl Iterator for &Gyro<'_> {
    type Item = Result<Rotation, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

impl Drop for Gyro<'_> {
    fn drop(&mut self) {
        // Contract: switches it off once (FW-6). Filled in at step 5.
    }
}

/// Settings for [`Gyro`] before it is turned on.
#[derive(Debug, Clone, Copy)]
pub struct GyroBuilder {
    _private: (),
}

impl GyroBuilder {
    /// Sets readings per second.
    #[must_use]
    pub fn rate(self, rate: Rate) -> Self {
        let _ = rate;
        todo!()
    }

    /// Sets the fastest rotation reported.
    #[must_use]
    pub fn range(self, range: GyroRange) -> Self {
        let _ = range;
        todo!()
    }


    /// Turns it on over `link` with these settings (API-1).
    ///
    /// # Errors
    ///
    /// Same as [`Gyro::on`].
    pub fn on(self, link: &impl AsLink) -> Result<Gyro<'_>, Error> {
        let _ = link;
        todo!()
    }
}

/// The accelerometer, on. Iterate over `&accel` for readings; dropping it
/// turns the accelerometer off (FW-6).
#[derive(Debug)]
pub struct Accel<'l> {
    _link: PhantomData<&'l ()>,
}

impl<'l> Accel<'l> {
    /// Turns the accelerometer on over `link` with the default settings:
    /// 104 Hz, smallest range (API-1).
    ///
    /// # Errors
    ///
    /// [`Error::Busy`] when another link holds the IMU (FW-1, FW-2),
    /// [`Error::TooSlow`] when the stream does not fit this link,
    /// [`Error::Timeout`] when the board does not answer.
    pub fn on(link: &'l impl AsLink) -> Result<Self, Error> {
        let _ = link;
        todo!()
    }

    /// Starts from the defaults with another rate.
    pub fn rate(rate: Rate) -> AccelBuilder {
        let _ = rate;
        todo!()
    }

    /// Starts from the defaults with another range.
    pub fn range(range: AccelRange) -> AccelBuilder {
        let _ = range;
        todo!()
    }

    /// The newest reading, skipping any older ones not yet read (API-3).
    ///
    /// # Errors
    ///
    /// [`Error::Disconnected`] when the link is gone.
    pub fn latest(&self) -> Result<Acceleration, Error> {
        todo!()
    }

    /// Readings lost since it was turned on (API-4).
    #[must_use]
    pub fn dropped(&self) -> Dropped {
        todo!()
    }
}

/// Every reading in order, waiting for the next one (API-3). Ends after
/// one `Err` when the link is gone.
impl Iterator for &Accel<'_> {
    type Item = Result<Acceleration, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

impl Drop for Accel<'_> {
    fn drop(&mut self) {
        // Contract: switches it off once (FW-6). Filled in at step 5.
    }
}

/// Settings for [`Accel`] before it is turned on.
#[derive(Debug, Clone, Copy)]
pub struct AccelBuilder {
    _private: (),
}

impl AccelBuilder {
    /// Sets readings per second.
    #[must_use]
    pub fn rate(self, rate: Rate) -> Self {
        let _ = rate;
        todo!()
    }

    /// Sets the largest acceleration reported.
    #[must_use]
    pub fn range(self, range: AccelRange) -> Self {
        let _ = range;
        todo!()
    }


    /// Turns it on over `link` with these settings (API-1).
    ///
    /// # Errors
    ///
    /// Same as [`Accel::on`].
    pub fn on(self, link: &impl AsLink) -> Result<Accel<'_>, Error> {
        let _ = link;
        todo!()
    }
}

/// Both halves of the IMU, on. Iterate over `&imu` for readings; dropping it
/// turns the whole IMU off (FW-6).
#[derive(Debug)]
pub struct Imu<'l> {
    _link: PhantomData<&'l ()>,
}

impl<'l> Imu<'l> {
    /// Turns both halves of the IMU on over `link` with the default settings:
    /// 104 Hz, smallest range (API-1).
    ///
    /// # Errors
    ///
    /// [`Error::Busy`] when another link holds the IMU (FW-1, FW-2),
    /// [`Error::TooSlow`] when the stream does not fit this link,
    /// [`Error::Timeout`] when the board does not answer.
    pub fn on(link: &'l impl AsLink) -> Result<Self, Error> {
        let _ = link;
        todo!()
    }

    /// Starts from the defaults with another rate for both halves.
    pub fn rate(rate: Rate) -> ImuBuilder {
        let _ = rate;
        todo!()
    }

    /// Starts from the defaults with another accelerometer range.
    pub fn accel_range(range: AccelRange) -> ImuBuilder {
        let _ = range;
        todo!()
    }

    /// Starts from the defaults with another gyroscope range.
    pub fn gyro_range(range: GyroRange) -> ImuBuilder {
        let _ = range;
        todo!()
    }

    /// The newest reading, skipping any older ones not yet read (API-3).
    ///
    /// # Errors
    ///
    /// [`Error::Disconnected`] when the link is gone.
    pub fn latest(&self) -> Result<Motion, Error> {
        todo!()
    }

    /// Readings lost since it was turned on (API-4).
    #[must_use]
    pub fn dropped(&self) -> Dropped {
        todo!()
    }
}

/// Every reading in order, waiting for the next one (API-3). Ends after
/// one `Err` when the link is gone.
impl Iterator for &Imu<'_> {
    type Item = Result<Motion, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

impl Drop for Imu<'_> {
    fn drop(&mut self) {
        // Contract: switches it off once (FW-6). Filled in at step 5.
    }
}

/// Settings for [`Imu`] before it is turned on.
#[derive(Debug, Clone, Copy)]
pub struct ImuBuilder {
    _private: (),
}

impl ImuBuilder {
    /// Sets readings per second, for both halves.
    #[must_use]
    pub fn rate(self, rate: Rate) -> Self {
        let _ = rate;
        todo!()
    }

    /// Sets the largest acceleration reported.
    #[must_use]
    pub fn accel_range(self, range: AccelRange) -> Self {
        let _ = range;
        todo!()
    }

    /// Sets the fastest rotation reported.
    #[must_use]
    pub fn gyro_range(self, range: GyroRange) -> Self {
        let _ = range;
        todo!()
    }


    /// Turns it on over `link` with these settings (API-1).
    ///
    /// # Errors
    ///
    /// Same as [`Imu::on`].
    pub fn on(self, link: &impl AsLink) -> Result<Imu<'_>, Error> {
        let _ = link;
        todo!()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn gyro_counts_become_dps() {
        todo!()
    }

    #[test]
    fn accel_counts_become_g() {
        todo!()
    }

    #[test]
    fn imu_yields_both_halves() {
        todo!()
    }

    #[test]
    fn drop_sends_off_once() {
        todo!()
    }
}
