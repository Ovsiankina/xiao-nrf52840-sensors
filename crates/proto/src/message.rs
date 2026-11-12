//! What is said: commands from the API, replies and readings from the
//! firmware, and the names both use (sensor, link, refusal reason).
//!
//! Depends on: nothing.

use serde::{Deserialize, Serialize};

/// Microphone sample rate, in samples per second.
pub const MIC_SAMPLE_RATE_HZ: u32 = 16_000;
/// Samples in one [`AudioBlock`].
pub const AUDIO_BLOCK_SAMPLES: usize = 96;
/// Bytes in one [`AudioBlock`]: two per sample (PROTO-2).
pub const AUDIO_BLOCK_BYTES: usize = 192;
/// The name the firmware advertises over BLE and the API looks for.
pub const BLE_NAME: &str = "XIAO-Sense";
/// Default UART speed (FW-3).
pub const UART_DEFAULT_BAUD: u32 = 921_600;

/// A sensor on the board. The IMU is one sensor (FW-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sensor {
    /// The PDM microphone.
    Mic,
    /// The accelerometer and gyroscope chip.
    Imu,
}

/// A connection between the API and the firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Link {
    /// The board's USB plug.
    Usb,
    /// Bluetooth Low Energy.
    Ble,
    /// The board's TX/RX pins.
    Uart,
}

/// How many readings per second the IMU produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rate {
    /// 12.5 Hz.
    Hz12_5,
    /// 26 Hz.
    Hz26,
    /// 52 Hz.
    Hz52,
    /// 104 Hz.
    Hz104,
    /// 208 Hz.
    Hz208,
    /// 416 Hz.
    Hz416,
    /// 833 Hz.
    Hz833,
}

impl Rate {
    /// The rate in readings per second.
    pub fn hertz(self) -> f32 {
        match self {
            Self::Hz12_5 => 12.5,
            Self::Hz26 => 26.0,
            Self::Hz52 => 52.0,
            Self::Hz104 => 104.0,
            Self::Hz208 => 208.0,
            Self::Hz416 => 416.0,
            Self::Hz833 => 833.0,
        }
    }
}

/// The largest acceleration the accelerometer can report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccelRange {
    /// ±2 g.
    G2,
    /// ±4 g.
    G4,
    /// ±8 g.
    G8,
    /// ±16 g.
    G16,
}

impl AccelRange {
    /// How many g one raw count is worth (PROTO-4).
    pub fn g_per_count(self) -> f32 {
        match self {
            Self::G2 => 0.000_061,
            Self::G4 => 0.000_122,
            Self::G8 => 0.000_244,
            Self::G16 => 0.000_488,
        }
    }
}

/// The fastest rotation the gyroscope can report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GyroRange {
    /// ±125 degrees per second.
    Dps125,
    /// ±250 degrees per second.
    Dps250,
    /// ±500 degrees per second.
    Dps500,
    /// ±1000 degrees per second.
    Dps1000,
    /// ±2000 degrees per second.
    Dps2000,
}

impl GyroRange {
    /// How many degrees per second one raw count is worth (PROTO-4).
    pub fn dps_per_count(self) -> f32 {
        match self {
            Self::Dps125 => 0.004_375,
            Self::Dps250 => 0.008_75,
            Self::Dps500 => 0.017_5,
            Self::Dps1000 => 0.035,
            Self::Dps2000 => 0.07,
        }
    }
}

/// Accelerometer settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccelConfig {
    /// Readings per second.
    pub rate: Rate,
    /// Largest acceleration reported.
    pub range: AccelRange,
}

impl Default for AccelConfig {
    /// 104 Hz, ±2 g.
    fn default() -> Self {
        Self { rate: Rate::Hz104, range: AccelRange::G2 }
    }
}

/// Gyroscope settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GyroConfig {
    /// Readings per second.
    pub rate: Rate,
    /// Fastest rotation reported.
    pub range: GyroRange,
}

impl Default for GyroConfig {
    /// 104 Hz, ±250 degrees per second.
    fn default() -> Self {
        Self { rate: Rate::Hz104, range: GyroRange::Dps250 }
    }
}

/// Which halves of the IMU are on, and how. `None` = that half is off
/// (FW-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImuConfig {
    /// The accelerometer, or `None` to keep it off.
    pub accel: Option<AccelConfig>,
    /// The gyroscope, or `None` to keep it off.
    pub gyro: Option<GyroConfig>,
}

/// What the API asks the firmware to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// Power the microphone and start streaming audio.
    MicOn,
    /// Stop streaming and power the microphone off.
    MicOff,
    /// Power the IMU halves named in the config and start streaming.
    ImuOn(ImuConfig),
    /// Stop streaming and power the IMU off.
    ImuOff,
}

/// Why the firmware said no.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    /// Another link holds the sensor (FW-1).
    Busy {
        /// The link that holds it.
        held_by: Link,
    },
    /// The stream does not fit the link (FW-4). Both in bytes per second.
    TooSlow {
        /// What the stream needs.
        needs: u32,
        /// What the link carries.
        link_has: u32,
    },
}

/// 96 audio samples, as 16-bit little-endian bytes (FW-5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioBlock {
    /// Counts blocks since the microphone was turned on; wraps.
    pub counter: u32,
    /// The samples, two bytes each, low byte first.
    pub pcm: heapless::Vec<u8, AUDIO_BLOCK_BYTES>,
}

/// One IMU reading in raw counts; x, y, z (FW-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImuReading {
    /// Counts readings since the IMU was turned on; wraps.
    pub counter: u32,
    /// Accelerometer counts, or `None` when that half is off.
    pub accel: Option<[i16; 3]>,
    /// Gyroscope counts, or `None` when that half is off.
    pub gyro: Option<[i16; 3]>,
}

/// What the firmware sends to the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    /// The sensor is on and held by the asking link.
    On(Sensor),
    /// The sensor is off and free.
    Off(Sensor),
    /// The request was refused.
    Refused {
        /// The sensor that was asked for.
        sensor: Sensor,
        /// Why.
        reason: Refusal,
    },
    /// Audio from the microphone.
    Audio(AudioBlock),
    /// A reading from the IMU.
    Imu(ImuReading),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::float_cmp, reason = "the factors are exact constants")]
    fn proto_4_scale_factors_match_st_driver() {
        // ST lsm6ds3tr-c-pid: mg/LSB and mdps/LSB per full scale.
        assert_eq!(AccelRange::G2.g_per_count(), 0.061e-3);
        assert_eq!(AccelRange::G4.g_per_count(), 0.122e-3);
        assert_eq!(AccelRange::G8.g_per_count(), 0.244e-3);
        assert_eq!(AccelRange::G16.g_per_count(), 0.488e-3);
        assert_eq!(GyroRange::Dps125.dps_per_count(), 4.375e-3);
        assert_eq!(GyroRange::Dps250.dps_per_count(), 8.75e-3);
        assert_eq!(GyroRange::Dps500.dps_per_count(), 17.5e-3);
        assert_eq!(GyroRange::Dps1000.dps_per_count(), 35.0e-3);
        assert_eq!(GyroRange::Dps2000.dps_per_count(), 70.0e-3);
    }

    #[test]
    #[allow(clippy::float_cmp, reason = "the rate is an exact constant")]
    fn defaults_are_104_hz() {
        assert_eq!(AccelConfig::default().rate.hertz(), 104.0);
        assert_eq!(GyroConfig::default().rate, Rate::Hz104);
        assert_eq!(AccelConfig::default().range, AccelRange::G2);
        assert_eq!(GyroConfig::default().range, GyroRange::Dps250);
    }
}
