//! The LSM6DS3TR-C: the few registers needed to switch the
//! accelerometer and gyroscope on and off, set rate and range, and
//! read them. Generic over the I2C bus, so it is tested on the host.
//!
//! Depends on: `xiao-proto::message` (rates, ranges, reading types).

use embedded_hal_async::i2c::I2c;
use xiao_proto::message::ImuConfig;

/// Why talking to the IMU failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImuError<E> {
    /// The I2C bus reported an error.
    Bus(E),
    /// Something answered, but not an LSM6DS3TR-C; holds the id it gave.
    WrongChip(u8),
}

/// The IMU chip on an I2C bus, at address 0x6A.
#[derive(Debug)]
pub struct Lsm6ds3<I2C> {
    i2c: I2C,
}

impl<I2C: I2c> Lsm6ds3<I2C> {
    /// Wraps the bus. Nothing is sent yet.
    pub const fn new(i2c: I2C) -> Self {
        Self { i2c }
    }

    /// Checks the chip's id, then sets each half to its config; a half
    /// that is `None` is powered down (FW-6).
    ///
    /// # Errors
    ///
    /// [`ImuError::WrongChip`] when the id is not 0x6A;
    /// [`ImuError::Bus`] on a bus error.
    pub async fn apply(&mut self, config: ImuConfig) -> Result<(), ImuError<I2C::Error>> {
        let _ = (config, &mut self.i2c);
        todo!()
    }

    /// Reads one sample of both halves in raw counts: `(accel, gyro)`,
    /// each x, y, z. A half that is off reads as zeros.
    ///
    /// # Errors
    ///
    /// [`ImuError::Bus`] on a bus error.
    pub async fn read(&mut self) -> Result<([i16; 3], [i16; 3]), ImuError<I2C::Error>> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn apply_writes_ctrl_registers() {
        todo!()
    }

    #[test]
    fn fw_6_off_half_is_powered_down() {
        todo!()
    }

    #[test]
    fn wrong_chip_is_an_error() {
        todo!()
    }

    #[test]
    fn read_splits_gyro_then_accel() {
        todo!()
    }
}
