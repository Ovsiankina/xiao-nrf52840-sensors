//! Which IMU half each chip sample carries.
//!
//! The IMU task reads the chip once per sample of the faster half (INT1
//! data-ready, `imu::Lsm6ds3::apply`). Each half goes out at its own
//! rate (FW-5): the faster one every tick, the slower one every 2^k
//! ticks (the chip's rates are one oscillator divided by powers of 2).
//! A half that was just switched on or changed is held back until its
//! samples are valid. Counted in ticks, so it needs no clock.
//!
//! Depends on: `xiao-proto::message`.

use xiao_proto::message::{ImuConfig, Rate};

/// Gyroscope turn-on time from power-down.
// DECISION: 70 ms, the LSM6DS3TR-C gyroscope turn-on time the reviewers
// cite from ST's datasheet and AN5130 (not in the research file, not
// measured). The 35 ms `board::IMU_SETTLE` is the chip's boot only.
const GYRO_TURN_ON_US: u64 = 70_000;

/// Samples of a half thrown away after it is switched on or changed.
// DECISION: 2, after AN5130: the first sample(s) after power-down or an
// ODR change are not settled. Datasheet reasoning only.
const DISCARD: u32 = 2;

/// Rate index: 12.5 Hz = 0 … 833 Hz = 6; each step doubles the rate.
const fn index(rate: Rate) -> u32 {
    match rate {
        Rate::Hz12_5 => 0,
        Rate::Hz26 => 1,
        Rate::Hz52 => 2,
        Rate::Hz104 => 3,
        Rate::Hz208 => 4,
        Rate::Hz416 => 5,
        Rate::Hz833 => 6,
    }
}

/// Time between two samples at `rate`, in microseconds.
pub const fn period_us(rate: Rate) -> u64 {
    match rate {
        Rate::Hz12_5 => 80_000,
        Rate::Hz26 => 38_462,
        Rate::Hz52 => 19_231,
        Rate::Hz104 => 9_615,
        Rate::Hz208 => 4_808,
        Rate::Hz416 => 2_404,
        Rate::Hz833 => 1_200,
    }
}

/// One half's place in the ticks.
#[derive(Debug, Clone, Copy)]
struct Half {
    /// Ticks per sample of this half: 1 for the faster half.
    every: u32,
    /// The first tick whose sample is valid.
    from: u32,
}

/// The pacing of one IMU stream.
#[derive(Debug)]
pub struct Pace {
    period_us: u64,
    accel: Option<Half>,
    gyro: Option<Half>,
    tick: u32,
}

impl Pace {
    /// The pacing after switching the chip from `before` to `after`.
    /// `None` when `after` has no half on.
    pub fn new(before: ImuConfig, after: ImuConfig) -> Option<Self> {
        let accel = after.accel.map(|a| a.rate);
        let gyro = after.gyro.map(|g| g.rate);
        // The faster half paces; the accelerometer on a tie, as INT1.
        let fast = match (accel, gyro) {
            (Some(a), Some(g)) if index(g) > index(a) => g,
            (Some(a), _) => a,
            (None, Some(g)) => g,
            (None, None) => return None,
        };
        let period_us = period_us(fast);
        let half = |rate: Rate, changed: bool, turn_on_us: u64| {
            let every = 1 << (index(fast) - index(rate));
            let from = if changed {
                let turn_on = u32::try_from(turn_on_us.div_ceil(period_us)).unwrap_or(u32::MAX);
                turn_on.saturating_add(DISCARD * every)
            } else {
                0
            };
            Half { every, from }
        };
        let gyro_turn_on = if before.gyro.is_none() { GYRO_TURN_ON_US } else { 0 };
        Some(Self {
            period_us,
            accel: accel.map(|rate| half(rate, before.accel != after.accel, 0)),
            gyro: gyro.map(|rate| half(rate, before.gyro != after.gyro, gyro_turn_on)),
            tick: 0,
        })
    }

    /// Time between two ticks (samples of the faster half), in µs.
    pub const fn period_us(&self) -> u64 {
        self.period_us
    }

    /// The earliest the chip may be read again after a read, in µs:
    /// 3/4 of a tick. INT1 is not looked at before.
    // DECISION: 3/4, so a chip whose oscillator runs fast (well under
    // 25 %) is still read on every data-ready, while an INT1 stuck high
    // cannot make the reads back to back.
    pub const fn floor_us(&self) -> u64 {
        self.period_us * 3 / 4
    }

    /// Which halves the next chip sample carries: `(accel, gyro)`.
    /// Neither: send nothing for this sample.
    pub fn tick(&mut self) -> (bool, bool) {
        let t = self.tick;
        self.tick = t.wrapping_add(1);
        let carries = |half: Option<Half>| {
            half.is_some_and(|h| t >= h.from && t.wrapping_add(1).is_multiple_of(h.every))
        };
        (carries(self.accel), carries(self.gyro))
    }
}

#[cfg(test)]
mod tests {
    use super::Pace;
    use xiao_proto::message::{AccelConfig, AccelRange, GyroConfig, GyroRange, ImuConfig, Rate};

    const OFF: ImuConfig = ImuConfig { accel: None, gyro: None };

    fn config(accel: Option<Rate>, gyro: Option<Rate>) -> ImuConfig {
        ImuConfig {
            accel: accel.map(|rate| AccelConfig { rate, range: AccelRange::G2 }),
            gyro: gyro.map(|rate| GyroConfig { rate, range: GyroRange::Dps250 }),
        }
    }

    /// How many samples of each half `ticks` ticks carry.
    fn count(pace: &mut Pace, ticks: u32) -> (u32, u32) {
        (0..ticks).fold((0, 0), |(a, g), _| {
            let (ca, cg) = pace.tick();
            (a + u32::from(ca), g + u32::from(cg))
        })
    }

    #[test]
    fn fw_5_each_half_at_its_own_rate() {
        // Accel 12.5 Hz, gyro 833 Hz, both running already: the gyro
        // every tick, the accel once every 64 ticks, never repeated.
        let both = config(Some(Rate::Hz12_5), Some(Rate::Hz833));
        let mut pace = Pace::new(both, both).unwrap();
        assert_eq!(pace.period_us(), 1_200);
        assert_eq!(count(&mut pace, 64 * 10), (10, 640));
    }

    #[test]
    fn first_samples_after_on_are_held_back() {
        // Gyro 833 Hz from power-down: 70 ms (59 ticks) + 2 samples.
        let gyro = config(None, Some(Rate::Hz833));
        let mut pace = Pace::new(OFF, gyro).unwrap();
        for _ in 0..61 {
            assert_eq!(pace.tick(), (false, false), "settling");
        }
        assert_eq!(pace.tick(), (false, true));

        // Accel 104 Hz from power-down: 2 samples thrown away.
        let accel = config(Some(Rate::Hz104), None);
        let mut pace = Pace::new(OFF, accel).unwrap();
        assert_eq!(count(&mut pace, 2), (0, 0));
        assert_eq!(pace.tick(), (true, false));
    }

    #[test]
    fn running_half_keeps_streaming_while_the_new_one_settles() {
        let accel = config(Some(Rate::Hz104), None);
        let both = config(Some(Rate::Hz104), Some(Rate::Hz104));
        let mut pace = Pace::new(accel, both).unwrap();
        // 70 ms at 104 Hz = 8 ticks, + 2 samples: the gyro joins at tick 10.
        assert_eq!(count(&mut pace, 10), (10, 0));
        assert_eq!(pace.tick(), (true, true));
    }

    #[test]
    fn reads_are_at_least_three_quarters_of_a_tick_apart() {
        let fast = config(Some(Rate::Hz833), None);
        assert_eq!(Pace::new(OFF, fast).unwrap().floor_us(), 900);
        let slow = config(None, Some(Rate::Hz12_5));
        assert_eq!(Pace::new(OFF, slow).unwrap().floor_us(), 60_000);
    }

    #[test]
    fn nothing_on_is_no_pace() {
        assert!(Pace::new(OFF, OFF).is_none());
    }
}
