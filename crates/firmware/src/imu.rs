//! The LSM6DS3TR-C: the few registers needed to switch the
//! accelerometer and gyroscope on and off, set rate and range, and
//! read them. Generic over the I2C bus, so it is tested on the host.
//!
//! Depends on: `xiao-proto::message` (rates, ranges, reading types).

use embedded_hal_async::i2c::I2c;
use xiao_proto::message::{AccelRange, GyroRange, ImuConfig, Rate};

/// The chip's 7-bit I2C address (SA0 tied to ground on the XIAO Sense).
const ADDRESS: u8 = 0x6A;
/// Register holding the chip id.
const WHO_AM_I: u8 = 0x0F;
/// What an LSM6DS3TR-C answers in `WHO_AM_I`.
const CHIP_ID: u8 = 0x6A;
/// Accelerometer rate and range.
const CTRL1_XL: u8 = 0x10;
/// Gyroscope rate and range.
const CTRL2_G: u8 = 0x11;
/// Bus behaviour.
const CTRL3_C: u8 = 0x12;
/// What drives the INT1 pin.
const INT1_CTRL: u8 = 0x0D;
/// INT1 goes high when the accelerometer has a new sample.
const INT1_DRDY_XL: u8 = 1 << 0;
/// INT1 goes high when the gyroscope has a new sample.
const INT1_DRDY_G: u8 = 1 << 1;
/// Block data update: the two bytes of a reading come from one sample.
const CTRL3_C_BDU: u8 = 1 << 6;
/// Register address increments on multi-byte access.
const CTRL3_C_IF_INC: u8 = 1 << 2;
/// First output register: gyro x, y, z then accel x, y, z, low byte first.
const OUTX_L_G: u8 = 0x22;
/// The 125 dps bit of `CTRL2_G`; it overrides the `FS_G` field.
const CTRL2_G_FS_125: u8 = 1 << 1;
/// A control register value that powers its half down (ODR = 0).
const POWER_DOWN: u8 = 0x00;

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
    // DECISION: remember which halves are on so `read` reports an off
    // half as zeros instead of the chip's stale last sample.
    config: ImuConfig,
}

/// The `ODR` field value (bits 7:4) for `rate`.
const fn odr(rate: Rate) -> u8 {
    let field = match rate {
        Rate::Hz12_5 => 1,
        Rate::Hz26 => 2,
        Rate::Hz52 => 3,
        Rate::Hz104 => 4,
        Rate::Hz208 => 5,
        Rate::Hz416 => 6,
        Rate::Hz833 => 7,
    };
    field << 4
}

/// `CTRL1_XL` for an accelerometer config, or power-down for `None`.
fn ctrl1_xl(config: ImuConfig) -> u8 {
    config.accel.map_or(POWER_DOWN, |accel| {
        let fs: u8 = match accel.range {
            AccelRange::G2 => 0,
            AccelRange::G16 => 1,
            AccelRange::G4 => 2,
            AccelRange::G8 => 3,
        };
        odr(accel.rate) | fs << 2
    })
}

/// `CTRL2_G` for a gyroscope config, or power-down for `None`.
fn ctrl2_g(config: ImuConfig) -> u8 {
    config.gyro.map_or(POWER_DOWN, |gyro| {
        let fs = match gyro.range {
            GyroRange::Dps125 => CTRL2_G_FS_125,
            GyroRange::Dps250 => 0,
            GyroRange::Dps500 => 1 << 2,
            GyroRange::Dps1000 => 2 << 2,
            GyroRange::Dps2000 => 3 << 2,
        };
        odr(gyro.rate) | fs
    })
}

/// `INT1_CTRL` for `config`: the data-ready of the faster half, so INT1
/// rises once per sample the firmware reads.
// DECISION: only the faster half (the accelerometer on a tie): both
// halves run off one oscillator, so the slower one's samples land on
// the faster one's, and two data-ready sources on one pin could rise
// twice per tick. The firmware's IMU task paces on the same rule. The
// DRDY signal stays latched (DRDY_PULSE_CFG_G reset value) until the
// output registers are read.
fn int1_ctrl(config: ImuConfig) -> u8 {
    match (config.accel, config.gyro) {
        (None, None) => 0,
        (Some(a), Some(g)) if odr(g.rate) > odr(a.rate) => INT1_DRDY_G,
        (None, Some(_)) => INT1_DRDY_G,
        (Some(_), _) => INT1_DRDY_XL,
    }
}

/// Three little-endian i16 from six bytes.
fn xyz(bytes: &[u8]) -> [i16; 3] {
    let axis = |i: usize| i16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]);
    [axis(0), axis(1), axis(2)]
}

impl<I2C: I2c> Lsm6ds3<I2C> {
    /// Wraps the bus. Nothing is sent yet.
    pub const fn new(i2c: I2C) -> Self {
        Self { i2c, config: ImuConfig { accel: None, gyro: None } }
    }

    /// Checks the chip's id, then sets each half to its config; a half
    /// that is `None` is powered down (FW-6). INT1 then signals each new
    /// sample of the faster half.
    ///
    /// # Errors
    ///
    /// [`ImuError::WrongChip`] when the id is not 0x6A;
    /// [`ImuError::Bus`] on a bus error.
    pub async fn apply(&mut self, config: ImuConfig) -> Result<(), ImuError<I2C::Error>> {
        let mut id = [0];
        self.i2c.write_read(ADDRESS, &[WHO_AM_I], &mut id).await.map_err(ImuError::Bus)?;
        if id[0] != CHIP_ID {
            return Err(ImuError::WrongChip(id[0]));
        }
        // DECISION: CTRL3_C is written whole (BDU + IF_INC, every other
        // bit at its reset value 0) on each apply; nothing else uses it.
        self.write(CTRL3_C, CTRL3_C_BDU | CTRL3_C_IF_INC).await?;
        self.write(INT1_CTRL, int1_ctrl(config)).await?;
        self.write(CTRL1_XL, ctrl1_xl(config)).await?;
        self.write(CTRL2_G, ctrl2_g(config)).await?;
        self.config = config;
        Ok(())
    }

    /// Writes one register.
    async fn write(&mut self, register: u8, value: u8) -> Result<(), ImuError<I2C::Error>> {
        self.i2c.write(ADDRESS, &[register, value]).await.map_err(ImuError::Bus)
    }

    /// Reads one sample of both halves in raw counts: `(accel, gyro)`,
    /// each x, y, z. A half that is off reads as zeros.
    ///
    /// # Errors
    ///
    /// [`ImuError::Bus`] on a bus error.
    pub async fn read(&mut self) -> Result<([i16; 3], [i16; 3]), ImuError<I2C::Error>> {
        let mut raw = [0; 12];
        self.i2c.write_read(ADDRESS, &[OUTX_L_G], &mut raw).await.map_err(ImuError::Bus)?;
        let gyro = if self.config.gyro.is_some() { xyz(&raw[..6]) } else { [0; 3] };
        let accel = if self.config.accel.is_some() { xyz(&raw[6..]) } else { [0; 3] };
        Ok((accel, gyro))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embassy_futures::block_on;
    use embedded_hal_async::i2c::{ErrorKind, ErrorType, NoAcknowledgeSource, Operation};
    use xiao_proto::message::{AccelConfig, GyroConfig};

    const NACK: ErrorKind = ErrorKind::NoAcknowledge(NoAcknowledgeSource::Address);

    /// A register file behind an I2C address: a write sets the register
    /// pointer then stores bytes; a read returns bytes from the pointer;
    /// both auto-increment.
    struct FakeI2c {
        regs: [u8; 128],
        /// Every `(register, value)` written, in order.
        writes: Vec<(u8, u8)>,
        /// `false`: nothing acknowledges the address.
        present: bool,
    }

    impl FakeI2c {
        fn new(chip_id: u8) -> Self {
            let mut regs = [0; 128];
            regs[usize::from(WHO_AM_I)] = chip_id;
            Self { regs, writes: Vec::new(), present: true }
        }
    }

    impl ErrorType for FakeI2c {
        type Error = ErrorKind;
    }

    impl I2c for FakeI2c {
        async fn transaction(
            &mut self,
            address: u8,
            operations: &mut [Operation<'_>],
        ) -> Result<(), ErrorKind> {
            if address != ADDRESS || !self.present {
                return Err(NACK);
            }
            let mut pointer = 0usize;
            for operation in operations {
                match operation {
                    Operation::Write(bytes) => {
                        let (&register, data) = bytes.split_first().unwrap();
                        pointer = usize::from(register);
                        for &value in data {
                            self.writes.push((u8::try_from(pointer).unwrap(), value));
                            self.regs[pointer] = value;
                            pointer += 1;
                        }
                    }
                    Operation::Read(buffer) => {
                        for byte in buffer.iter_mut() {
                            *byte = self.regs[pointer];
                            pointer += 1;
                        }
                    }
                }
            }
            Ok(())
        }
    }

    fn reg(imu: &Lsm6ds3<FakeI2c>, register: u8) -> u8 {
        imu.i2c.regs[usize::from(register)]
    }

    #[test]
    fn apply_writes_ctrl_registers() {
        let mut imu = Lsm6ds3::new(FakeI2c::new(CHIP_ID));
        let config = ImuConfig {
            accel: Some(AccelConfig { rate: Rate::Hz104, range: AccelRange::G4 }),
            gyro: Some(GyroConfig { rate: Rate::Hz104, range: GyroRange::Dps500 }),
        };
        block_on(imu.apply(config)).unwrap();
        assert_eq!(reg(&imu, CTRL3_C), 0x44);
        assert_eq!(reg(&imu, CTRL1_XL), 0x48);
        assert_eq!(reg(&imu, CTRL2_G), 0x44);
        // BDU, auto-increment and INT1 are set before any half is on.
        assert_eq!(
            imu.i2c.writes,
            vec![(CTRL3_C, 0x44), (INT1_CTRL, INT1_DRDY_XL), (CTRL1_XL, 0x48), (CTRL2_G, 0x44)]
        );
        // 125 dps is its own bit, not an FS_G value.
        let config = ImuConfig {
            accel: None,
            gyro: Some(GyroConfig { rate: Rate::Hz12_5, range: GyroRange::Dps125 }),
        };
        block_on(imu.apply(config)).unwrap();
        assert_eq!(reg(&imu, CTRL2_G), 0x12);
    }

    #[test]
    fn int1_follows_the_faster_half() {
        let mut imu = Lsm6ds3::new(FakeI2c::new(CHIP_ID));
        let accel = |rate| Some(AccelConfig { rate, range: AccelRange::G2 });
        let gyro = |rate| Some(GyroConfig { rate, range: GyroRange::Dps250 });
        let cases = [
            (accel(Rate::Hz12_5), gyro(Rate::Hz833), INT1_DRDY_G),
            (accel(Rate::Hz833), gyro(Rate::Hz12_5), INT1_DRDY_XL),
            (accel(Rate::Hz104), gyro(Rate::Hz104), INT1_DRDY_XL),
            (None, gyro(Rate::Hz26), INT1_DRDY_G),
            (None, None, 0),
        ];
        for (accel, gyro, int1) in cases {
            block_on(imu.apply(ImuConfig { accel, gyro })).unwrap();
            assert_eq!(reg(&imu, INT1_CTRL), int1, "{accel:?} {gyro:?}");
        }
    }

    #[test]
    fn fw_6_off_half_is_powered_down() {
        let mut imu = Lsm6ds3::new(FakeI2c::new(CHIP_ID));
        let both = ImuConfig {
            accel: Some(AccelConfig::default()),
            gyro: Some(GyroConfig::default()),
        };
        block_on(imu.apply(both)).unwrap();
        let gyro_only = ImuConfig { accel: None, ..both };
        block_on(imu.apply(gyro_only)).unwrap();
        assert_eq!(reg(&imu, CTRL1_XL), 0x00);
        assert_eq!(reg(&imu, CTRL2_G), 0x40);
        block_on(imu.apply(ImuConfig { accel: None, gyro: None })).unwrap();
        assert_eq!(reg(&imu, CTRL1_XL), 0x00);
        assert_eq!(reg(&imu, CTRL2_G), 0x00);
    }

    #[test]
    fn wrong_chip_is_an_error() {
        // The LSM6DS3 (no TR-C) answers 0x69.
        let mut imu = Lsm6ds3::new(FakeI2c::new(0x69));
        let config = ImuConfig { accel: Some(AccelConfig::default()), gyro: None };
        assert_eq!(block_on(imu.apply(config)), Err(ImuError::WrongChip(0x69)));
        // Nothing was switched on.
        assert!(imu.i2c.writes.is_empty());
        // No chip at all: the bus error comes through.
        let mut absent = Lsm6ds3::new(FakeI2c::new(CHIP_ID));
        absent.i2c.present = false;
        assert_eq!(block_on(absent.apply(config)), Err(ImuError::Bus(NACK)));
    }

    #[test]
    fn read_splits_gyro_then_accel() {
        let mut imu = Lsm6ds3::new(FakeI2c::new(CHIP_ID));
        let both = ImuConfig {
            accel: Some(AccelConfig::default()),
            gyro: Some(GyroConfig::default()),
        };
        block_on(imu.apply(both)).unwrap();
        // gyro x=1, y=-2, z=0x1234; accel x=-1, y=256, z=-32768
        let out = [
            0x01, 0x00, 0xFE, 0xFF, 0x34, 0x12, //
            0xFF, 0xFF, 0x00, 0x01, 0x00, 0x80,
        ];
        let start = usize::from(OUTX_L_G);
        imu.i2c.regs[start..start + 12].copy_from_slice(&out);
        let (accel, gyro) = block_on(imu.read()).unwrap();
        assert_eq!(gyro, [1, -2, 0x1234]);
        assert_eq!(accel, [-1, 256, -32768]);
        // An off half reads as zeros, whatever the chip still holds.
        block_on(imu.apply(ImuConfig { gyro: None, ..both })).unwrap();
        let (accel, gyro) = block_on(imu.read()).unwrap();
        assert_eq!(gyro, [0; 3]);
        assert_eq!(accel, [-1, 256, -32768]);
    }
}
