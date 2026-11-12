//! Who holds which sensor. Decides every request: grant, refuse as
//! busy (naming the holding link), or refuse as too fast for the link.
//! Pure logic, no hardware.
//!
//! Depends on: `xiao-proto::message`.

use xiao_proto::message::{Command, ImuConfig, Link, Refusal, Sensor};

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
        Self { mic: None, imu: None, imu_config: ImuConfig { accel: None, gyro: None } }
    }

    /// Decides a command from `link`, whose speed is `link_speed` bytes
    /// per second. Refuses with `Busy` when another link holds the
    /// sensor (FW-1, FW-2) and with `TooSlow` when the stream does not
    /// fit (FW-4). The holding link may ask again to change settings.
    /// An `ImuOn` with both halves off is an `ImuOff` (FW-6).
    pub fn request(&mut self, link: Link, cmd: &Command, link_speed: u32) -> Decision {
        let _ = (link, cmd, link_speed, &mut self.mic, &mut self.imu, &mut self.imu_config);
        todo!()
    }

    /// The link holding `sensor`, if any (FW-1).
    #[must_use]
    pub fn holder(&self, sensor: Sensor) -> Option<Link> {
        let _ = sensor;
        todo!()
    }

    /// How the IMU is set now; both halves `None` when it is off (FW-6).
    #[must_use]
    pub fn imu_config(&self) -> ImuConfig {
        todo!()
    }

    /// Frees every sensor `link` holds, for when the link goes away.
    pub fn release_link(&mut self, link: Link) {
        let _ = link;
        todo!()
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
    let _ = (link, uart_baud);
    todo!()
}

/// What a command's stream needs, in bytes per second (FW-4): the
/// microphone 34 000; the IMU 16 bytes per reading per half that is on.
#[must_use]
pub fn needs(cmd: &Command) -> u32 {
    let _ = cmd;
    todo!()
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
    pub fn next(&mut self) -> u32 {
        let _ = &mut self.0;
        todo!()
    }
}

impl Default for Counter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fw_1_busy_names_holding_link() {
        todo!()
    }

    #[test]
    fn fw_1_two_sensors_two_links() {
        todo!()
    }

    #[test]
    fn fw_2_imu_halves_share_one_link() {
        todo!()
    }

    #[test]
    fn fw_3_uart_default_is_921600() {
        todo!()
    }

    #[test]
    fn fw_4_mic_refused_on_slow_uart() {
        todo!()
    }

    #[test]
    fn fw_5_counter_counts_and_wraps() {
        todo!()
    }

    #[test]
    fn fw_6_unrequested_half_stays_off() {
        todo!()
    }

    #[test]
    fn same_link_may_reconfigure() {
        todo!()
    }

    #[test]
    fn release_link_frees_its_sensors() {
        todo!()
    }
}
