//! Keeps the readings of a dead session off the next one.
//!
//! When a link goes down its `Outbox` is cleared, but a sensor task may
//! still be in the middle of a reading for it (a 6 ms audio block, an
//! I2C read) and queue it afterwards. A new session on that link holds
//! no sensor until one of its commands is granted, and the firmware
//! sends `On` before any reading of a stream. So a session passes a
//! sensor's readings only once it has written `On` for that sensor.
//!
//! Depends on: `xiao-proto::message`.

use xiao_proto::message::{Message, Sensor};

/// What one session of a link has turned on so far.
#[derive(Debug, Default)]
pub struct Gate {
    mic: bool,
    imu: bool,
}

impl Gate {
    /// A new session: no sensor on yet.
    pub const fn new() -> Self {
        Self { mic: false, imu: false }
    }

    /// Whether `message` goes out on this session. Replies always do;
    /// a reading only after this session wrote `On` for its sensor.
    pub fn pass(&mut self, message: &Message) -> bool {
        match message {
            Message::On(sensor) => {
                *self.open(*sensor) = true;
                true
            }
            Message::Off(_) | Message::Refused { .. } => true,
            Message::Audio(_) => self.mic,
            Message::Imu(_) => self.imu,
        }
    }

    fn open(&mut self, sensor: Sensor) -> &mut bool {
        match sensor {
            Sensor::Mic => &mut self.mic,
            Sensor::Imu => &mut self.imu,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Gate;
    use xiao_proto::message::{AudioBlock, ImuReading, Link, Message, Refusal, Sensor};

    fn audio(counter: u32) -> Message {
        Message::Audio(AudioBlock { counter, pcm: heapless::Vec::new() })
    }

    fn imu(counter: u32) -> Message {
        Message::Imu(ImuReading { counter, accel: Some([0; 3]), gyro: None })
    }

    #[test]
    fn stale_reading_before_on_is_dropped() {
        // The old session's last block, queued after the Outbox was
        // cleared, reaches the new session before any command.
        let mut gate = Gate::new();
        assert!(!gate.pass(&audio(10_000)), "stale block dropped");
        assert!(!gate.pass(&imu(6_240)), "stale IMU reading dropped");
        assert!(gate.pass(&Message::On(Sensor::Mic)));
        assert!(gate.pass(&audio(0)), "the new stream goes out");
        assert!(!gate.pass(&imu(6_241)), "the IMU is still not on for this session");
    }

    #[test]
    fn replies_always_pass() {
        let mut gate = Gate::new();
        assert!(gate.pass(&Message::Off(Sensor::Imu)));
        let busy = Refusal::Busy { held_by: Link::Uart };
        assert!(gate.pass(&Message::Refused { sensor: Sensor::Imu, reason: busy }));
        assert!(gate.pass(&Message::On(Sensor::Imu)));
        assert!(gate.pass(&imu(0)));
    }
}
