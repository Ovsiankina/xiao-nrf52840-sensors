//! Who holds which sensor. Decides every request: grant, refuse as
//! busy (naming the holding link), or refuse as too fast for the link.
//! Pure logic, no hardware.
//!
//! Depends on: `xiao-proto::message`.

use xiao_proto::message::{Command, ImuConfig, Link, Rate, Refusal, Sensor};

/// The IMU with both halves off.
const IMU_OFF: ImuConfig = ImuConfig { accel: None, gyro: None };

/// The answer to a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Do it; the table already records the new state.
    Grant,
    /// Do nothing and tell the asker why.
    Refuse(Refusal),
}

/// Which link holds each sensor, and how the IMU is set.
#[derive(Debug)]
pub struct Table {
    mic: Option<Link>,
    imu: Option<Link>,
    imu_config: ImuConfig,
}

impl Table {
    /// Nothing held, everything off (FW-6).
    #[must_use]
    pub const fn new() -> Self {
        Self { mic: None, imu: None, imu_config: IMU_OFF }
    }

    /// Decides a command from `link`, whose speed is `link_speed` bytes
    /// per second. Refuses with `Busy` when another link holds the
    /// sensor (FW-1, FW-2) and with `TooSlow` when the stream does not
    /// fit (FW-4). The holding link may ask again to change settings.
    /// An `ImuOn` with both halves off is an `ImuOff` (FW-6).
    pub fn request(&mut self, link: Link, cmd: &Command, link_speed: u32) -> Decision {
        // FW-6: an `ImuOn` that turns nothing on is an `ImuOff`.
        let cmd = match *cmd {
            Command::ImuOn(ImuConfig { accel: None, gyro: None }) => Command::ImuOff,
            other => other,
        };
        let sensor = match cmd {
            Command::MicOn | Command::MicOff => Sensor::Mic,
            Command::ImuOn(_) | Command::ImuOff => Sensor::Imu,
        };
        // DECISION: an Off from a link that does not hold the sensor is
        // refused as busy too, so a second link can never stop the
        // holder's stream (FW-1); an Off for a free sensor is granted.
        if let Some(held_by) = self.holder(sensor).filter(|&held_by| held_by != link) {
            return Decision::Refuse(Refusal::Busy { held_by });
        }
        let needs = needs(&cmd);
        if needs > link_speed {
            return Decision::Refuse(Refusal::TooSlow { needs, link_has: link_speed });
        }
        match cmd {
            Command::MicOn => self.mic = Some(link),
            Command::MicOff => self.mic = None,
            Command::ImuOn(config) => {
                self.imu = Some(link);
                self.imu_config = config;
            }
            Command::ImuOff => {
                self.imu = None;
                self.imu_config = IMU_OFF;
            }
        }
        Decision::Grant
    }

    /// The link holding `sensor`, if any (FW-1).
    #[must_use]
    pub fn holder(&self, sensor: Sensor) -> Option<Link> {
        match sensor {
            Sensor::Mic => self.mic,
            Sensor::Imu => self.imu,
        }
    }

    /// How the IMU is set now; both halves `None` when it is off (FW-6).
    #[must_use]
    pub fn imu_config(&self) -> ImuConfig {
        self.imu_config
    }

    /// Frees every sensor `link` holds, for when the link goes away.
    pub fn release_link(&mut self, link: Link) {
        if self.mic == Some(link) {
            self.mic = None;
        }
        if self.imu == Some(link) {
            self.imu = None;
            self.imu_config = IMU_OFF;
        }
    }
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

/// What a link carries, in bytes per second: USB 500 000, UART a tenth
/// of its baud (FW-3), BLE 60 000.
#[must_use]
pub const fn link_speed(link: Link, uart_baud: u32) -> u32 {
    // DECISION: BLE 60 000 B/s is a placeholder until measured on the
    // board (SRS hole "whether BLE can carry the mic stream").
    match link {
        Link::Usb => 500_000,
        // 8N1: ten bits on the wire per byte.
        Link::Uart => uart_baud / 10,
        Link::Ble => 60_000,
    }
}

/// Bytes per second the microphone stream needs (FW-4): 32 000 of
/// samples plus framing.
// DECISION: 34 000 = 32 000 B/s of PCM + ~6 % for the per-block
// postcard/COBS overhead (about 11 bytes per 192-byte block at 166.7
// blocks/s = 33 800), rounded up.
const MIC_NEEDS: u32 = 34_000;

/// Bytes on the wire per IMU message, whatever it carries: the message
/// tag, the counter (up to 5 bytes as a varint), one `Option` tag per
/// half, and 2 bytes of framing (COBS overhead and the closing zero).
const IMU_BYTES_PER_MESSAGE: u32 = 10;

/// Bytes a half adds to a message that carries it: three `i16` as
/// zigzag varints, up to 3 bytes each.
const IMU_BYTES_PER_HALF: u32 = 9;

/// `rate` in tenths of a hertz, so 12.5 Hz stays exact without floats.
const fn decihertz(rate: Rate) -> u32 {
    match rate {
        Rate::Hz12_5 => 125,
        Rate::Hz26 => 260,
        Rate::Hz52 => 520,
        Rate::Hz104 => 1_040,
        Rate::Hz208 => 2_080,
        Rate::Hz416 => 4_160,
        Rate::Hz833 => 8_330,
    }
}

/// What a command's stream needs, in bytes per second (FW-4): the
/// microphone 34 000; the IMU its worst case on the wire, rounded up.
// DECISION: the IMU sends one message per sample of its faster half,
// carrying each half only when that half has a new sample (FW-5, own
// rate). So: 10 B per message at the faster rate, plus 9 B per half at
// that half's own rate; every field at its longest. One half at 416 Hz
// needs 19 × 416 = 7 904 B/s, which is what goes out. The plan's
// "16 B per half" undercounted a single-half message (19 B).
#[must_use]
pub fn needs(cmd: &Command) -> u32 {
    match cmd {
        Command::MicOn => MIC_NEEDS,
        Command::ImuOn(config) => {
            let accel = config.accel.map_or(0, |a| decihertz(a.rate));
            let gyro = config.gyro.map_or(0, |g| decihertz(g.rate));
            let messages = accel.max(gyro) * IMU_BYTES_PER_MESSAGE;
            (messages + (accel + gyro) * IMU_BYTES_PER_HALF).div_ceil(10)
        }
        Command::MicOff | Command::ImuOff => 0,
    }
}

/// Numbers the messages of one stream (FW-5).
#[derive(Debug)]
pub struct Counter(u32);

impl Counter {
    /// Starts at zero.
    #[must_use]
    pub const fn new() -> Self {
        Self(0)
    }

    /// The number for the next message; wraps after `u32::MAX`.
    #[allow(clippy::should_implement_trait, reason = "the name is the step-4 contract")]
    pub fn next(&mut self) -> u32 {
        let n = self.0;
        self.0 = n.wrapping_add(1);
        n
    }
}

impl Default for Counter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiao_proto::message::{
        AccelConfig, AccelRange, GyroConfig, GyroRange, UART_DEFAULT_BAUD,
    };

    const FAST: u32 = 500_000;

    fn accel_only() -> ImuConfig {
        ImuConfig { accel: Some(AccelConfig::default()), gyro: None }
    }

    fn both() -> ImuConfig {
        ImuConfig { accel: Some(AccelConfig::default()), gyro: Some(GyroConfig::default()) }
    }

    #[test]
    fn fw_1_busy_names_holding_link() {
        let mut table = Table::new();
        assert_eq!(table.request(Link::Usb, &Command::MicOn, FAST), Decision::Grant);
        assert_eq!(
            table.request(Link::Ble, &Command::MicOn, FAST),
            Decision::Refuse(Refusal::Busy { held_by: Link::Usb })
        );
        // The second link cannot stop the holder's stream either.
        assert_eq!(
            table.request(Link::Uart, &Command::MicOff, FAST),
            Decision::Refuse(Refusal::Busy { held_by: Link::Usb })
        );
        assert_eq!(table.holder(Sensor::Mic), Some(Link::Usb));
    }

    #[test]
    fn fw_1_two_sensors_two_links() {
        let mut table = Table::new();
        assert_eq!(table.request(Link::Usb, &Command::MicOn, FAST), Decision::Grant);
        assert_eq!(table.request(Link::Ble, &Command::ImuOn(both()), 60_000), Decision::Grant);
        assert_eq!(table.holder(Sensor::Mic), Some(Link::Usb));
        assert_eq!(table.holder(Sensor::Imu), Some(Link::Ble));
    }

    #[test]
    fn fw_2_imu_halves_share_one_link() {
        let mut table = Table::new();
        assert_eq!(table.request(Link::Usb, &Command::ImuOn(accel_only()), FAST), Decision::Grant);
        // The gyroscope is the same sensor: another link may not take it.
        let gyro_only = ImuConfig { accel: None, gyro: Some(GyroConfig::default()) };
        assert_eq!(
            table.request(Link::Ble, &Command::ImuOn(gyro_only), FAST),
            Decision::Refuse(Refusal::Busy { held_by: Link::Usb })
        );
        assert_eq!(table.imu_config(), accel_only());
    }

    #[test]
    fn fw_3_uart_default_is_921600() {
        assert_eq!(link_speed(Link::Uart, UART_DEFAULT_BAUD), 92_160);
        assert_eq!(link_speed(Link::Usb, UART_DEFAULT_BAUD), 500_000);
        assert_eq!(link_speed(Link::Ble, UART_DEFAULT_BAUD), 60_000);
    }

    #[test]
    fn fw_4_mic_refused_on_slow_uart() {
        let mut table = Table::new();
        assert_eq!(
            table.request(Link::Uart, &Command::MicOn, link_speed(Link::Uart, 115_200)),
            Decision::Refuse(Refusal::TooSlow { needs: 34_000, link_has: 11_520 })
        );
        assert_eq!(table.holder(Sensor::Mic), None);
        // The IMU at 12.5 Hz fits on the same slow link: 350 B/s.
        let slow = ImuConfig {
            accel: Some(AccelConfig { rate: Rate::Hz12_5, range: AccelRange::G2 }),
            gyro: Some(GyroConfig { rate: Rate::Hz12_5, range: GyroRange::Dps250 }),
        };
        assert_eq!(needs(&Command::ImuOn(slow)), 350);
        assert_eq!(needs(&Command::ImuOn(both())), 28 * 104);
        assert_eq!(table.request(Link::Uart, &Command::ImuOn(slow), 11_520), Decision::Grant);
    }

    #[test]
    fn imu_needs_cover_the_longest_message() {
        // One half: 19 B per message (1 tag + 5 counter + 2 option tags
        // + 9 data + 2 framing) at its own rate.
        let gyro_416 = ImuConfig {
            accel: None,
            gyro: Some(GyroConfig { rate: Rate::Hz416, range: GyroRange::Dps250 }),
        };
        assert_eq!(needs(&Command::ImuOn(gyro_416)), 19 * 416);
        assert_eq!(
            table_request(gyro_416, link_speed(Link::Uart, 76_800)),
            Decision::Refuse(Refusal::TooSlow { needs: 7_904, link_has: 7_680 })
        );
        // Two halves at different rates: a message per fast sample (10 B
        // + 9 B of the fast half), plus 9 B per slow sample.
        let mixed = ImuConfig {
            accel: Some(AccelConfig { rate: Rate::Hz833, range: AccelRange::G2 }),
            gyro: Some(GyroConfig { rate: Rate::Hz12_5, range: GyroRange::Dps250 }),
        };
        assert_eq!(needs(&Command::ImuOn(mixed)), 15_940);
    }

    fn table_request(config: ImuConfig, speed: u32) -> Decision {
        Table::new().request(Link::Uart, &Command::ImuOn(config), speed)
    }

    #[test]
    fn fw_5_counter_counts_and_wraps() {
        let mut counter = Counter::new();
        assert_eq!(counter.next(), 0);
        assert_eq!(counter.next(), 1);
        assert_eq!(counter.next(), 2);
        let mut counter = Counter(u32::MAX);
        assert_eq!(counter.next(), u32::MAX);
        assert_eq!(counter.next(), 0);
    }

    #[test]
    fn fw_6_unrequested_half_stays_off() {
        let mut table = Table::new();
        assert_eq!(table.imu_config(), IMU_OFF);
        assert_eq!(table.holder(Sensor::Mic), None);
        assert_eq!(table.holder(Sensor::Imu), None);
        assert_eq!(table.request(Link::Usb, &Command::ImuOn(accel_only()), FAST), Decision::Grant);
        assert_eq!(table.imu_config().gyro, None);
        // An ImuOn with neither half is an ImuOff: the IMU is freed.
        assert_eq!(table.request(Link::Usb, &Command::ImuOn(IMU_OFF), FAST), Decision::Grant);
        assert_eq!(table.holder(Sensor::Imu), None);
        assert_eq!(table.imu_config(), IMU_OFF);
    }

    #[test]
    fn same_link_may_reconfigure() {
        let mut table = Table::new();
        assert_eq!(table.request(Link::Ble, &Command::ImuOn(accel_only()), FAST), Decision::Grant);
        assert_eq!(table.request(Link::Ble, &Command::ImuOn(both()), FAST), Decision::Grant);
        assert_eq!(table.imu_config(), both());
        assert_eq!(table.holder(Sensor::Imu), Some(Link::Ble));
        assert_eq!(table.request(Link::Ble, &Command::ImuOff, FAST), Decision::Grant);
        assert_eq!(table.holder(Sensor::Imu), None);
    }

    #[test]
    fn release_link_frees_its_sensors() {
        let mut table = Table::new();
        assert_eq!(table.request(Link::Ble, &Command::MicOn, FAST), Decision::Grant);
        assert_eq!(table.request(Link::Ble, &Command::ImuOn(both()), FAST), Decision::Grant);
        table.release_link(Link::Usb);
        assert_eq!(table.holder(Sensor::Mic), Some(Link::Ble));
        table.release_link(Link::Ble);
        assert_eq!(table.holder(Sensor::Mic), None);
        assert_eq!(table.holder(Sensor::Imu), None);
        assert_eq!(table.imu_config(), IMU_OFF);
        assert_eq!(table.request(Link::Usb, &Command::MicOn, FAST), Decision::Grant);
    }
}
