//! The IMU: accelerometer, gyroscope, or both, turned on over one link;
//! each gives a stream of readings.
//!
//! Depends on: `link`, `stream`, `error`.

use crate::link::{self, AsLink, Connection};
use crate::stream::Stream;
use crate::{Dropped, Error};
use std::sync::mpsc::Receiver;
use xiao_proto::message::{
    AccelConfig, AccelRange, Command, GyroConfig, GyroRange, ImuConfig, ImuReading, Message, Rate,
    Sensor,
};

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
///
/// It keeps its link open: the link closes once the link value and every
/// sensor on it are dropped.
#[derive(Debug)]
pub struct Gyro {
    connection: Connection,
    stream: Stream<Rotation>,
}

impl Gyro {
    /// Turns the gyroscope on over `link` with the default settings:
    /// 104 Hz, smallest range (API-1).
    ///
    /// # Errors
    ///
    /// [`Error::Busy`] when another link holds the IMU (FW-1, FW-2),
    /// [`Error::TooSlow`] when the stream does not fit this link,
    /// [`Error::Timeout`] when the board does not answer.
    pub fn on(link: &impl AsLink) -> Result<Self, Error> {
        GyroBuilder::DEFAULT.on(link)
    }

    /// Starts from the defaults with another rate.
    pub fn rate(rate: Rate) -> GyroBuilder {
        GyroBuilder::DEFAULT.rate(rate)
    }

    /// Starts from the defaults with another range.
    pub fn range(range: GyroRange) -> GyroBuilder {
        GyroBuilder::DEFAULT.range(range)
    }

    /// The newest reading, skipping any older ones not yet read (API-3).
    ///
    /// # Errors
    ///
    /// [`Error::Disconnected`] when the link is gone.
    pub fn latest(&self) -> Result<Rotation, Error> {
        self.stream.latest()
    }

    /// The newest reading already here, skipping older ones not yet read;
    /// `None` when none came since the last read. Never waits (API-3).
    ///
    /// # Errors
    ///
    /// Once, why the link ended: [`Error::Disconnected`] when it is gone,
    /// [`Error::Timeout`] when BLE ran out of reconnect attempts. Then
    /// `Ok(None)` for ever.
    pub fn try_latest(&self) -> Result<Option<Rotation>, Error> {
        self.stream.try_latest()
    }

    /// Readings lost since it was turned on (API-4).
    #[must_use]
    pub fn dropped(&self) -> Dropped {
        self.stream.dropped()
    }
}

/// Every reading in order, waiting for the next one (API-3). Ends after
/// one `Err` when the link is gone.
// DECISION: `copy_iterator` allowed: the contract iterates over `&gyro`
// (`for r in &gyro`), and every shared reference is `Copy`.
#[allow(clippy::copy_iterator, reason = "the contract's `for r in &sensor`")]
impl Iterator for &Gyro {
    type Item = Result<Rotation, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        self.stream.next()
    }
}

impl Drop for Gyro {
    fn drop(&mut self) {
        release(&self.connection, Halves { accel: false, gyro: true });
    }
}

/// Settings for [`Gyro`] before it is turned on.
#[derive(Debug, Clone, Copy)]
pub struct GyroBuilder {
    config: GyroConfig,
}

impl GyroBuilder {
    // DECISION: the defaults are proto's `GyroConfig::default()` (104 Hz,
    // ±250 dps), which pins them with a test, not the ±125 dps that
    // "smallest range" in `Gyro::on`'s doc could also mean.
    const DEFAULT: Self =
        Self { config: GyroConfig { rate: Rate::Hz104, range: GyroRange::Dps250 } };

    /// Sets readings per second.
    #[must_use]
    pub fn rate(self, rate: Rate) -> Self {
        Self { config: GyroConfig { rate, ..self.config } }
    }

    /// Sets the fastest rotation reported.
    #[must_use]
    pub fn range(self, range: GyroRange) -> Self {
        Self { config: GyroConfig { range, ..self.config } }
    }

    /// Turns it on over `link` with these settings (API-1).
    ///
    /// # Errors
    ///
    /// Same as [`Gyro::on`].
    pub fn on(self, link: &impl AsLink) -> Result<Gyro, Error> {
        let range = self.config.range;
        let want = ImuConfig { accel: None, gyro: Some(self.config) };
        let (connection, stream) =
            claim(link, want, move |reading| reading.gyro.map(|raw| rotation(raw, range)))?;
        Ok(Gyro { connection, stream })
    }
}

/// The accelerometer, on. Iterate over `&accel` for readings; dropping it
/// turns the accelerometer off (FW-6).
///
/// It keeps its link open: the link closes once the link value and every
/// sensor on it are dropped.
#[derive(Debug)]
pub struct Accel {
    connection: Connection,
    stream: Stream<Acceleration>,
}

impl Accel {
    /// Turns the accelerometer on over `link` with the default settings:
    /// 104 Hz, smallest range (API-1).
    ///
    /// # Errors
    ///
    /// [`Error::Busy`] when another link holds the IMU (FW-1, FW-2),
    /// [`Error::TooSlow`] when the stream does not fit this link,
    /// [`Error::Timeout`] when the board does not answer.
    pub fn on(link: &impl AsLink) -> Result<Self, Error> {
        AccelBuilder::DEFAULT.on(link)
    }

    /// Starts from the defaults with another rate.
    pub fn rate(rate: Rate) -> AccelBuilder {
        AccelBuilder::DEFAULT.rate(rate)
    }

    /// Starts from the defaults with another range.
    pub fn range(range: AccelRange) -> AccelBuilder {
        AccelBuilder::DEFAULT.range(range)
    }

    /// The newest reading, skipping any older ones not yet read (API-3).
    ///
    /// # Errors
    ///
    /// [`Error::Disconnected`] when the link is gone.
    pub fn latest(&self) -> Result<Acceleration, Error> {
        self.stream.latest()
    }

    /// The newest reading already here, skipping older ones not yet read;
    /// `None` when none came since the last read. Never waits (API-3).
    ///
    /// # Errors
    ///
    /// Same as [`Gyro::try_latest`].
    pub fn try_latest(&self) -> Result<Option<Acceleration>, Error> {
        self.stream.try_latest()
    }

    /// Readings lost since it was turned on (API-4).
    #[must_use]
    pub fn dropped(&self) -> Dropped {
        self.stream.dropped()
    }
}

/// Every reading in order, waiting for the next one (API-3). Ends after
/// one `Err` when the link is gone.
#[allow(clippy::copy_iterator, reason = "the contract's `for r in &sensor`")]
impl Iterator for &Accel {
    type Item = Result<Acceleration, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        self.stream.next()
    }
}

impl Drop for Accel {
    fn drop(&mut self) {
        release(&self.connection, Halves { accel: true, gyro: false });
    }
}

/// Settings for [`Accel`] before it is turned on.
#[derive(Debug, Clone, Copy)]
pub struct AccelBuilder {
    config: AccelConfig,
}

impl AccelBuilder {
    /// Proto's `AccelConfig::default()`: 104 Hz, ±2 g.
    const DEFAULT: Self = Self { config: AccelConfig { rate: Rate::Hz104, range: AccelRange::G2 } };

    /// Sets readings per second.
    #[must_use]
    pub fn rate(self, rate: Rate) -> Self {
        Self { config: AccelConfig { rate, ..self.config } }
    }

    /// Sets the largest acceleration reported.
    #[must_use]
    pub fn range(self, range: AccelRange) -> Self {
        Self { config: AccelConfig { range, ..self.config } }
    }

    /// Turns it on over `link` with these settings (API-1).
    ///
    /// # Errors
    ///
    /// Same as [`Accel::on`].
    pub fn on(self, link: &impl AsLink) -> Result<Accel, Error> {
        let range = self.config.range;
        let want = ImuConfig { accel: Some(self.config), gyro: None };
        let (connection, stream) =
            claim(link, want, move |reading| reading.accel.map(|raw| acceleration(raw, range)))?;
        Ok(Accel { connection, stream })
    }
}

/// Both halves of the IMU, on. Iterate over `&imu` for readings; dropping it
/// turns the whole IMU off (FW-6).
///
/// It keeps its link open: the link closes once the link value and every
/// sensor on it are dropped.
#[derive(Debug)]
pub struct Imu {
    connection: Connection,
    stream: Stream<Motion>,
}

impl Imu {
    /// Turns both halves of the IMU on over `link` with the default settings:
    /// 104 Hz, smallest range (API-1).
    ///
    /// # Errors
    ///
    /// [`Error::Busy`] when another link holds the IMU (FW-1, FW-2),
    /// [`Error::TooSlow`] when the stream does not fit this link,
    /// [`Error::Timeout`] when the board does not answer.
    pub fn on(link: &impl AsLink) -> Result<Self, Error> {
        ImuBuilder::DEFAULT.on(link)
    }

    /// Starts from the defaults with another rate for both halves.
    pub fn rate(rate: Rate) -> ImuBuilder {
        ImuBuilder::DEFAULT.rate(rate)
    }

    /// Starts from the defaults with another accelerometer range.
    pub fn accel_range(range: AccelRange) -> ImuBuilder {
        ImuBuilder::DEFAULT.accel_range(range)
    }

    /// Starts from the defaults with another gyroscope range.
    pub fn gyro_range(range: GyroRange) -> ImuBuilder {
        ImuBuilder::DEFAULT.gyro_range(range)
    }

    /// The newest reading, skipping any older ones not yet read (API-3).
    ///
    /// # Errors
    ///
    /// [`Error::Disconnected`] when the link is gone.
    pub fn latest(&self) -> Result<Motion, Error> {
        self.stream.latest()
    }

    /// The newest reading already here, skipping older ones not yet read;
    /// `None` when none came since the last read. Never waits (API-3).
    ///
    /// # Errors
    ///
    /// Same as [`Gyro::try_latest`].
    pub fn try_latest(&self) -> Result<Option<Motion>, Error> {
        self.stream.try_latest()
    }

    /// Readings lost since it was turned on (API-4).
    #[must_use]
    pub fn dropped(&self) -> Dropped {
        self.stream.dropped()
    }
}

/// Every reading in order, waiting for the next one (API-3). Ends after
/// one `Err` when the link is gone.
#[allow(clippy::copy_iterator, reason = "the contract's `for r in &sensor`")]
impl Iterator for &Imu {
    type Item = Result<Motion, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        self.stream.next()
    }
}

impl Drop for Imu {
    fn drop(&mut self) {
        release(&self.connection, Halves { accel: true, gyro: true });
    }
}

/// Settings for [`Imu`] before it is turned on.
#[derive(Debug, Clone, Copy)]
pub struct ImuBuilder {
    accel: AccelConfig,
    gyro: GyroConfig,
}

impl ImuBuilder {
    const DEFAULT: Self =
        Self { accel: AccelBuilder::DEFAULT.config, gyro: GyroBuilder::DEFAULT.config };

    /// Sets readings per second, for both halves.
    #[must_use]
    pub fn rate(self, rate: Rate) -> Self {
        Self { accel: AccelConfig { rate, ..self.accel }, gyro: GyroConfig { rate, ..self.gyro } }
    }

    /// Sets the largest acceleration reported.
    #[must_use]
    pub fn accel_range(self, range: AccelRange) -> Self {
        Self { accel: AccelConfig { range, ..self.accel }, ..self }
    }

    /// Sets the fastest rotation reported.
    #[must_use]
    pub fn gyro_range(self, range: GyroRange) -> Self {
        Self { gyro: GyroConfig { range, ..self.gyro }, ..self }
    }

    /// Turns it on over `link` with these settings (API-1).
    ///
    /// # Errors
    ///
    /// Same as [`Imu::on`].
    pub fn on(self, link: &impl AsLink) -> Result<Imu, Error> {
        let (accel_range, gyro_range) = (self.accel.range, self.gyro.range);
        let want = ImuConfig { accel: Some(self.accel), gyro: Some(self.gyro) };
        let (connection, stream) = claim(link, want, move |reading| {
            Some(Motion {
                accel: acceleration(reading.accel?, accel_range),
                gyro: rotation(reading.gyro?, gyro_range),
            })
        })?;
        Ok(Imu { connection, stream })
    }
}

/// Which IMU halves a sensor value holds.
#[derive(Debug, Clone, Copy)]
struct Halves {
    accel: bool,
    gyro: bool,
}

// DECISION: the IMU halves alive on one link are tracked per link
// (`Connection::held`). Turning a half on sends `ImuOn` with the other
// half's settings kept; dropping one sends `ImuOn` with only the other
// half, and `ImuOff` only when no half is left (FW-2, FW-6). A half
// already held on the same link is `Busy` naming that link: two values
// would read with different ranges from one setting.
/// Turns on the halves `want` names over `link`, keeping any half another
/// value on the link holds, and returns the stream of `convert`ed readings.
fn claim<T>(
    link: &impl AsLink,
    want: ImuConfig,
    convert: impl Fn(ImuReading) -> Option<T> + Send + 'static,
) -> Result<(Connection, Stream<T>), Error> {
    let (connection, kind) = link::parts(link);
    let mut held = connection.held();
    let has = held.imu;
    if (want.accel.is_some() && has.accel.is_some()) || (want.gyro.is_some() && has.gyro.is_some())
    {
        return Err(Error::Busy { held_by: kind });
    }
    let config = ImuConfig { accel: want.accel.or(has.accel), gyro: want.gyro.or(has.gyro) };
    // Subscribed before asking, so no reading sent after `On` is lost.
    let rx = connection.subscribe(Sensor::Imu);
    if let Err(error) = connection.request(Command::ImuOn(config)) {
        if matches!(error, Error::Timeout { .. }) {
            // DECISION: an unanswered `ImuOn` is undone, unanswered too, back to
            // what was held before, so a late `On` leaves no half running
            // that no value holds.
            let _ = connection.tell(command_for(has));
        }
        return Err(error);
    }
    held.imu = config;
    let joining = has.accel.is_some() || has.gyro.is_some();
    Ok((connection.share(), stream(rx, joining, convert)))
}

fn stream<T>(
    rx: Receiver<Result<Message, Error>>,
    joining: bool,
    convert: impl Fn(ImuReading) -> Option<T> + Send + 'static,
) -> Stream<T> {
    let convert = move |message| match message {
        Message::Imu(reading) => convert(reading),
        _ => None,
    };
    if joining { Stream::joining(rx, convert) } else { Stream::new(rx, convert) }
}

/// Switches off the halves `gone` names, leaving the rest as they are.
fn release(connection: &Connection, gone: Halves) {
    let mut held = connection.held();
    let left = ImuConfig {
        accel: held.imu.accel.filter(|_| !gone.accel),
        gyro: held.imu.gyro.filter(|_| !gone.gyro),
    };
    held.imu = left;
    let command = command_for(left);
    // DECISION: the drop waits for the board's answer (up to 2 s) and
    // ignores a failure: `drop` cannot return it, and a board that lost
    // the link frees the IMU on its own.
    let _ = connection.request(command);
}

/// The command that leaves exactly the halves `config` names on.
fn command_for(config: ImuConfig) -> Command {
    if config.accel.is_none() && config.gyro.is_none() {
        Command::ImuOff
    } else {
        Command::ImuOn(config)
    }
}

/// Raw accelerometer counts in g, at the range that was sent (PROTO-4).
fn acceleration(raw: [i16; 3], range: AccelRange) -> Acceleration {
    let g = |count: i16| G(f32::from(count) * range.g_per_count());
    Acceleration { x: g(raw[0]), y: g(raw[1]), z: g(raw[2]) }
}

/// Raw gyroscope counts in degrees per second, at the range that was sent
/// (PROTO-4).
fn rotation(raw: [i16; 3], range: GyroRange) -> Rotation {
    let dps = |count: i16| Dps(f32::from(count) * range.dps_per_count());
    Rotation { x: dps(raw[0]), y: dps(raw[1]), z: dps(raw[2]) }
}

#[cfg(test)]
mod tests {
    use super::{Accel, Acceleration, Dps, G, Gyro, Imu, Rotation};
    use crate::link::{Memory, answered, memory_usb};
    use crate::{Dropped, Error, Mic, Usb};
    use std::thread;
    use std::time::{Duration, Instant};
    use xiao_proto::message::{
        AccelConfig, AccelRange, Command, GyroConfig, GyroRange, ImuConfig, ImuReading, Link,
        Message, Rate,
    };

    const QUIET: Duration = Duration::from_millis(200);

    fn send(board: &mut Memory, counter: u32, accel: Option<[i16; 3]>, gyro: Option<[i16; 3]>) {
        board.send(&Message::Imu(ImuReading { counter, accel, gyro }));
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn gyro_counts_become_dps() {
        let (usb, mut board) = memory_usb();
        let (gyro, command) =
            answered(&board, || Gyro::rate(Rate::Hz208).range(GyroRange::Dps500).on(&usb));
        let gyro = gyro.unwrap();
        let config = GyroConfig { rate: Rate::Hz208, range: GyroRange::Dps500 };
        assert_eq!(command, Command::ImuOn(ImuConfig { accel: None, gyro: Some(config) }));

        send(&mut board, 0, None, Some([1000, -1000, 0]));
        let Rotation { x, y, z } = (&gyro).next().unwrap().unwrap();
        assert!(close(x.0, 17.5), "{x:?}");
        assert!(close(y.0, -17.5), "{y:?}");
        assert_eq!(z, Dps(0.0));

        // A reading without the gyro half is skipped, its counter still counts.
        send(&mut board, 1, Some([1, 1, 1]), None);
        send(&mut board, 3, None, Some([2000, 0, 0]));
        assert!(close(gyro.latest().unwrap().x.0, 35.0));
        assert_eq!(gyro.dropped(), Dropped(1));

        answered(&board, || drop(gyro));
    }

    #[test]
    fn accel_counts_become_g() {
        let (usb, mut board) = memory_usb();
        let (accel, command) = answered(&board, || Accel::on(&usb));
        let accel = accel.unwrap();
        let defaults = AccelConfig { rate: Rate::Hz104, range: AccelRange::G2 };
        assert_eq!(command, Command::ImuOn(ImuConfig { accel: Some(defaults), gyro: None }));

        send(&mut board, 0, Some([16384, -16384, 0]), None);
        let Acceleration { x, y, z } = accel.latest().unwrap();
        assert!(close(x.0, 0.9994), "{x:?}");
        assert!(close(y.0, -0.9994), "{y:?}");
        assert_eq!(z, G(0.0));
        answered(&board, || drop(accel));

        // Another range: the factor of the config that was sent.
        let accel = answered(&board, || Accel::range(AccelRange::G16).on(&usb)).0.unwrap();
        send(&mut board, 0, Some([1000, 0, 0]), None);
        assert!(close((&accel).next().unwrap().unwrap().x.0, 0.488));
        answered(&board, || drop(accel));
    }

    #[test]
    fn imu_yields_both_halves() {
        let (usb, mut board) = memory_usb();
        let (imu, command) = answered(&board, || {
            Imu::rate(Rate::Hz52)
                .gyro_range(GyroRange::Dps1000)
                .accel_range(AccelRange::G4)
                .on(&usb)
        });
        let imu = imu.unwrap();
        let accel = AccelConfig { rate: Rate::Hz52, range: AccelRange::G4 };
        let gyro = GyroConfig { rate: Rate::Hz52, range: GyroRange::Dps1000 };
        assert_eq!(command, Command::ImuOn(ImuConfig { accel: Some(accel), gyro: Some(gyro) }));

        send(&mut board, 0, Some([1000, 0, 0]), Some([0, 1000, 0]));
        let motion = (&imu).next().unwrap().unwrap();
        assert!(close(motion.accel.x.0, 0.122), "{motion:?}");
        assert!(close(motion.gyro.y.0, 35.0), "{motion:?}");
        answered(&board, || drop(imu));
    }

    #[test]
    fn drop_sends_off_once() {
        let (usb, mut board) = memory_usb();
        let gyro = answered(&board, || Gyro::on(&usb)).0.unwrap();
        let ((), command) = answered(&board, || drop(gyro));
        assert_eq!(command, Command::ImuOff);
        assert_eq!(board.command(QUIET), None, "and nothing after it");

        let imu = answered(&board, || Imu::on(&usb)).0.unwrap();
        let ((), command) = answered(&board, || drop(imu));
        assert_eq!(command, Command::ImuOff);
        assert_eq!(board.command(QUIET), None);
    }

    #[test]
    fn gyro_and_accel_share_the_imu() {
        let (usb, mut board) = memory_usb();
        let gyro_config = GyroConfig { rate: Rate::Hz104, range: GyroRange::Dps250 };
        let accel_config = AccelConfig { rate: Rate::Hz104, range: AccelRange::G2 };
        let gyro = answered(&board, || Gyro::on(&usb)).0.unwrap();
        send(&mut board, 0, None, Some([0, 0, 0]));
        send(&mut board, 1, None, Some([0, 0, 0]));

        // The accelerometer joins: ImuOn names both halves (FW-2).
        let (accel, command) = answered(&board, || Accel::on(&usb));
        let accel = accel.unwrap();
        let both = ImuConfig { accel: Some(accel_config), gyro: Some(gyro_config) };
        assert_eq!(command, Command::ImuOn(both));

        // Both read the one stream; the joiner's first counter is no drop.
        send(&mut board, 2, Some([16384, 0, 0]), Some([1000, 0, 0]));
        assert!(close(accel.latest().unwrap().x.0, 0.9994));
        assert!(close(gyro.latest().unwrap().x.0, 8.75));
        assert_eq!(accel.dropped(), Dropped(0));
        assert_eq!(gyro.dropped(), Dropped(0));

        // A half already held on this link is busy, and nothing is sent.
        let again = Gyro::on(&usb);
        assert!(matches!(again, Err(Error::Busy { held_by: Link::Usb })), "{again:?}");
        let whole = Imu::on(&usb);
        assert!(matches!(whole, Err(Error::Busy { held_by: Link::Usb })), "{whole:?}");
        assert_eq!(board.command(QUIET), None);

        // Dropping the gyro keeps the accelerometer on...
        let ((), command) = answered(&board, || drop(gyro));
        let accel_only = ImuConfig { accel: Some(accel_config), gyro: None };
        assert_eq!(command, Command::ImuOn(accel_only));
        send(&mut board, 3, Some([0, 16384, 0]), None);
        assert!(close(accel.latest().unwrap().y.0, 0.9994));

        // ...and the last half dropped switches the IMU off, once.
        let ((), command) = answered(&board, || drop(accel));
        assert_eq!(command, Command::ImuOff);
        assert_eq!(board.command(QUIET), None);
    }

    #[test]
    fn sensors_are_send_and_sync() {
        // The contract stubs' sensors had every auto trait; the filled ones keep them.
        fn check<T: Send + Sync + std::panic::UnwindSafe + std::panic::RefUnwindSafe>() {}
        check::<crate::Mic>();
        check::<Gyro>();
        check::<Accel>();
        check::<Imu>();
    }

    /// Runs `act`, which must return in under 10 ms.
    fn at_once<T>(act: impl FnOnce() -> T) -> T {
        let start = Instant::now();
        let result = act();
        assert!(start.elapsed() < Duration::from_millis(10), "waited {:?}", start.elapsed());
        result
    }

    /// Polls `try_latest` until a reading is there, for up to 1 s.
    fn arrived<T>(mut try_latest: impl FnMut() -> Result<Option<T>, Error>) -> T {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if let Some(reading) = try_latest().unwrap() {
                return reading;
            }
            assert!(Instant::now() < deadline, "no reading arrived");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn try_latest_is_none_when_nothing_is_queued() {
        let (usb, mut board) = memory_usb();

        let gyro = answered(&board, || Gyro::on(&usb)).0.unwrap();
        assert_eq!(at_once(|| gyro.try_latest()).unwrap(), None);
        send(&mut board, 0, None, Some([1000, 0, 0]));
        assert!(close(arrived(|| gyro.try_latest()).x.0, 8.75));
        assert_eq!(at_once(|| gyro.try_latest()).unwrap(), None, "handed out once");
        answered(&board, || drop(gyro));

        let accel = answered(&board, || Accel::on(&usb)).0.unwrap();
        assert_eq!(at_once(|| accel.try_latest()).unwrap(), None);
        send(&mut board, 0, Some([16384, 0, 0]), None);
        assert!(close(arrived(|| accel.try_latest()).x.0, 0.9994));
        answered(&board, || drop(accel));

        let imu = answered(&board, || Imu::on(&usb)).0.unwrap();
        assert_eq!(at_once(|| imu.try_latest()).unwrap(), None);
        send(&mut board, 0, Some([16384, 0, 0]), Some([1000, 0, 0]));
        let motion = arrived(|| imu.try_latest());
        assert!(close(motion.gyro.x.0, 8.75), "{motion:?}");
        answered(&board, || drop(imu));
    }

    #[test]
    fn a_sensor_outlives_its_link_value() {
        let (usb, mut board) = memory_usb();
        let gyro = answered(&board, || Gyro::on(&usb)).0.unwrap();
        drop(usb);

        send(&mut board, 0, None, Some([1000, 0, 0]));
        assert!(close(gyro.latest().unwrap().x.0, 8.75), "still reads");
        let ((), command) = answered(&board, || drop(gyro));
        assert_eq!(command, Command::ImuOff, "and still turns the gyroscope off");
    }

    #[test]
    fn a_sensor_and_its_link_fit_in_one_struct() {
        struct Rig {
            usb: Usb,
            gyro: Gyro,
            mic: Mic,
        }

        // Configured, turned on and returned with its link: no borrow left.
        fn set_up() -> (Rig, Memory) {
            let (usb, board) = memory_usb();
            let configured = || Gyro::rate(Rate::Hz208).range(GyroRange::Dps500).on(&usb);
            let gyro = answered(&board, configured).0.unwrap();
            let mic = answered(&board, || Mic::on(&usb)).0.unwrap();
            (Rig { usb, gyro, mic }, board)
        }

        let (rig, mut board) = set_up();
        send(&mut board, 0, None, Some([1000, 0, 0]));
        assert!(close(rig.gyro.latest().unwrap().x.0, 17.5));
        let again = Gyro::on(&rig.usb);
        assert!(matches!(again, Err(Error::Busy { held_by: Link::Usb })), "{again:?}");

        let Rig { usb, gyro, mic } = rig;
        drop(usb);
        answered(&board, || drop(gyro));
        answered(&board, || drop(mic));
    }

    #[test]
    fn timed_out_on_is_undone() {
        let (usb, mut board) = memory_usb();
        let gyro = answered(&board, || Gyro::on(&usb)).0.unwrap();
        let gyro_only = ImuConfig {
            accel: None,
            gyro: Some(GyroConfig { rate: Rate::Hz104, range: GyroRange::Dps250 }),
        };

        // The board does not answer the accelerometer's ImuOn in time: the
        // API puts the IMU back to the gyroscope alone.
        let accel = Accel::on(&usb);
        assert!(matches!(accel, Err(Error::Timeout { attempts: 1 })), "{accel:?}");
        assert!(matches!(
            board.command(QUIET),
            Some(Command::ImuOn(ImuConfig { accel: Some(_), .. }))
        ));
        assert_eq!(board.command(QUIET), Some(Command::ImuOn(gyro_only)));
        assert_eq!(board.command(QUIET), None);

        // With nothing else held, the undo is ImuOff.
        answered(&board, || drop(gyro));
        assert!(matches!(Gyro::on(&usb), Err(Error::Timeout { .. })));
        assert!(matches!(board.command(QUIET), Some(Command::ImuOn(_))));
        assert_eq!(board.command(QUIET), Some(Command::ImuOff));
    }
}
