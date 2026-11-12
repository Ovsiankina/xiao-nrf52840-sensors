//! The firmware binary: starts the links, then serves commands.
//!
//! One task per link turns bytes into commands for the one `Inbox`. The
//! main task decides each command with [`sensors::Table`], has the
//! sensor's task switch it, and that task replies `On`/`Off` before it
//! sends any reading. A sensor task owns its sensor and pushes readings,
//! each with a [`sensors::Counter`], to its holder's `Outbox`.
//!
//! Depends on: every module of `xiao_firmware`.
#![cfg_attr(target_os = "none", no_std, no_main)]

#[cfg(target_os = "none")]
mod board;
#[cfg(any(target_os = "none", test))]
mod gate;
#[cfg(target_os = "none")]
mod link;
#[cfg(target_os = "none")]
mod mic;
#[cfg(any(target_os = "none", test))]
mod pace;

#[cfg(target_os = "none")]
use {nrf_mpsl as _, panic_halt as _};

#[cfg(target_os = "none")]
#[embassy_executor::main]
async fn main(spawner: embassy_executor::Spawner) {
    let board = board::Board::take(embassy_nrf::init(board::config()));
    // The MPSL takes over the clocks, so it starts before USB, which
    // needs the crystal.
    let ble = board.ble;
    let mpsl = serve::start_mpsl(ble.mpsl);
    // Held until the end of `main`, which never comes: the crystal keeps
    // running for USB whatever the radio does.
    let _hfclk = match mpsl {
        Some(mpsl) => {
            spawner.spawn(serve::mpsl_task(mpsl).unwrap());
            // DECISION: no BLE without the crystal held: USB, the link
            // that powers the board, comes first. `request_hfclk` only
            // fails when a second guard is asked for, which never happens.
            match mpsl.request_hfclk().await {
                Ok(hfclk) => {
                    spawner.spawn(serve::ble_task(mpsl, ble.sdc, ble.rng).unwrap());
                    Some(hfclk)
                }
                Err(_) => None,
            }
        }
        None => None,
    };
    let mic = mic::Mic::new(board.mic, board::Irqs);
    spawner.spawn(serve::usb_task(board.usb).unwrap());
    spawner.spawn(serve::uart_task(board.uart).unwrap());
    spawner.spawn(serve::mic_task(mic).unwrap());
    spawner.spawn(serve::imu_task(board.imu).unwrap());
    serve::commands().await
}

/// The tasks and the main loop. Board only.
#[cfg(target_os = "none")]
mod serve {
    use embassy_futures::select::{Either, select, select_array};
    use embassy_nrf::gpio::{Input, Pull};
    use embassy_nrf::peripherals::{RNG, USBD};
    use embassy_nrf::{Peri, twim};
    use nrf_mpsl::{MultiprotocolServiceLayer, raw as mpsl_raw};
    use static_cell::StaticCell;
    use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
    use embassy_sync::channel::Channel;
    use embassy_sync::signal::Signal;
    use embassy_time::{Duration, Instant, Timer, with_timeout};
    use xiao_firmware::imu::Lsm6ds3;
    use xiao_firmware::sensors::{Counter, Decision, Table, link_speed};
    use xiao_proto::message::{
        Command, ImuConfig, ImuReading, Link, Message, Sensor, UART_DEFAULT_BAUD,
    };

    use crate::board::{ImuPins, Irqs, UartPins};
    use crate::link::{Inbox, Outbox, Session, ble, uart, usb};
    use crate::mic::{Mic, audio_block};
    use crate::pace::Pace;

    /// The UART link's speed, in baud (FW-3).
    // DECISION: the default, fixed at build time; the SRS leaves open how
    // it is configured, and nothing sets it yet.
    const UART_BAUD: u32 = UART_DEFAULT_BAUD;

    /// The longest a reply waits for room in a full `Outbox`.
    // DECISION: a reply is never dropped while its link drains, but a dead
    // link's full `Outbox` must not stall the sensor task and, through it,
    // main. 500 ms is a quarter of the API's 2 s reply timeout.
    const REPLY_WAIT: Duration = Duration::from_millis(500);

    /// The longest one IMU call (`apply`, `read`) may hold the I2C bus.
    // DECISION: 10 ms, over 10 times the longest call at 400 kHz (`apply`,
    // five short transfers, under 1 ms). Only a stuck bus (a line held
    // low) runs out; the chip is then treated as failed.
    const I2C_TIMEOUT: Duration = Duration::from_millis(10);

    /// Commands from every link.
    static INBOX: Inbox = Channel::new();
    /// What goes out on each link, indexed by [`slot`].
    static OUTBOX: [Outbox; 3] = [const { Channel::new() }; 3];
    /// Raised by a link task when its link goes down, with the link's new
    /// [`Session`], indexed by [`slot`].
    // DECISION: one `Signal` per link, not a channel of links: `on_down`
    // is synchronous, and a signal raised twice before main sees it still
    // means "release this link" once, with the latest session, so no
    // link-down is ever lost.
    static DOWN: [Signal<ThreadModeRawMutex, Session>; 3] = [const { Signal::new() }; 3];
    /// The links, in [`slot`] order.
    const LINKS: [Link; 3] = [Link::Usb, Link::Ble, Link::Uart];

    /// A link's index into [`OUTBOX`] and [`DOWN`].
    const fn slot(link: Link) -> usize {
        match link {
            Link::Usb => 0,
            Link::Ble => 1,
            Link::Uart => 2,
        }
    }

    /// The `Outbox` of `link`.
    fn outbox(link: Link) -> &'static Outbox {
        &OUTBOX[slot(link)]
    }

    /// Queues `message` for `link`, waiting at most [`REPLY_WAIT`] for
    /// room. Replies go through here; readings use `try_send`.
    async fn reply(link: Link, message: Message) {
        // A reply that still finds no room is dropped: the API times out.
        let _ = with_timeout(REPLY_WAIT, outbox(link).send(message)).await;
    }

    /// What main asks a sensor task to do.
    #[derive(Clone, Copy)]
    struct Switch<C> {
        /// `Some((holder, settings))`: on, streaming to `holder`. `None`: off.
        to: Option<(Link, C)>,
        /// Who gets `On`/`Off` once it is done; `None` when the link that
        /// held the sensor went down.
        reply: Option<Link>,
    }

    /// Main's line to one sensor task.
    // DECISION: two signals, and main waits for `done` after every
    // switch: switches are applied one at a time, in order, so none is
    // overwritten before its task saw it, and main learns whether it
    // worked before it takes the next command.
    struct Control<C> {
        switch: Signal<ThreadModeRawMutex, Switch<C>>,
        /// `true` once the switch is applied, `false` when it failed and
        /// the sensor is off.
        done: Signal<ThreadModeRawMutex, bool>,
    }

    impl<C: Send> Control<C> {
        const fn new() -> Self {
            Self { switch: Signal::new(), done: Signal::new() }
        }

        /// Hands `switch` to the task and waits until it is applied.
        async fn apply(&self, switch: Switch<C>) -> bool {
            self.switch.signal(switch);
            self.done.wait().await
        }
    }

    static MIC: Control<()> = Control::new();
    static IMU: Control<ImuConfig> = Control::new();

    /// The USB link (fw-04).
    #[embassy_executor::task]
    pub async fn usb_task(usb: Peri<'static, USBD>) -> ! {
        let down = |link, session| DOWN[slot(link)].signal(session);
        usb::run(usb, &INBOX, outbox(Link::Usb), down).await
    }

    /// The MPSL, once started; the BLE controller and `main` share it.
    static MPSL: StaticCell<MultiprotocolServiceLayer<'static>> = StaticCell::new();

    /// Starts the MPSL, which from then on owns the clocks, the radio,
    /// RTC0 and TIMER0. `None` when it does not start; BLE stays off and
    /// the crystal `embassy_nrf::init` started keeps running.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the MPSL's bindgen constants are u32; each value fits its field"
    )]
    pub fn start_mpsl(
        p: nrf_mpsl::Peripherals<'static>,
    ) -> Option<&'static MultiprotocolServiceLayer<'static>> {
        // DECISION: the low-frequency clock stays on the internal RC
        // oscillator, as `embassy_nrf::init` started it for the time
        // driver, calibrated as Nordic recommends (every 4 s, at least
        // every 8 s). It works on every board, crystal or not; BLE allows
        // up to 500 ppm.
        let lfclk = mpsl_raw::mpsl_clock_lfclk_cfg_t {
            source: mpsl_raw::MPSL_CLOCK_LF_SRC_RC as u8,
            rc_ctiv: mpsl_raw::MPSL_RECOMMENDED_RC_CTIV as u8,
            rc_temp_ctiv: mpsl_raw::MPSL_RECOMMENDED_RC_TEMP_CTIV as u8,
            accuracy_ppm: mpsl_raw::MPSL_WORST_CLOCK_ACCURACY_PPM as u16,
            skip_wait_lfclk_started: false,
        };
        let mpsl = MultiprotocolServiceLayer::new(p, Irqs, lfclk).ok()?;
        Some(MPSL.init(mpsl))
    }

    /// The MPSL's low-priority work.
    #[embassy_executor::task]
    pub async fn mpsl_task(mpsl: &'static MultiprotocolServiceLayer<'static>) -> ! {
        mpsl.run().await
    }

    /// The BLE link (fw-06).
    #[embassy_executor::task]
    pub async fn ble_task(
        mpsl: &'static MultiprotocolServiceLayer<'static>,
        sdc: nrf_sdc::Peripherals<'static>,
        rng: Peri<'static, RNG>,
    ) -> ! {
        let radio = ble::Radio { mpsl, sdc, rng };
        let down = |link, session| DOWN[slot(link)].signal(session);
        ble::run(radio, &INBOX, outbox(Link::Ble), down).await
    }

    /// The UART link (fw-04); it never goes down.
    #[embassy_executor::task]
    pub async fn uart_task(pins: UartPins) -> ! {
        uart::run(pins, UART_BAUD, &INBOX, outbox(Link::Uart)).await
    }

    /// The main loop: decides every command and frees the sensors of a
    /// link that went down. Never returns.
    pub async fn commands() -> ! {
        let mut table = Table::new();
        // Each link's current session, indexed by [`slot`].
        let mut sessions: [Session; 3] = [0; 3];
        loop {
            let down = select_array(DOWN.each_ref().map(Signal::wait));
            // DECISION: a link-down is taken before any queued command
            // (`select` polls its first future first). A command from a
            // new session on that link, queued while main was busy, must
            // not be granted and then undone by the old session's down.
            // A command the closed session queued before its down carries
            // the old session and is dropped unanswered: granted after
            // the release, it would power and hold a sensor nobody asks
            // for, and the next session's gate would open on its `On`
            // (FW-6). The link task raises the down before its next
            // session posts anything, so a command's session is never
            // newer than main's.
            match select(down, INBOX.receive()).await {
                Either::First((session, i)) => {
                    sessions[i] = session;
                    release(&mut table, LINKS[i]).await;
                }
                Either::Second((link, session, cmd)) => {
                    if session == sessions[slot(link)] {
                        command(&mut table, link, &cmd).await;
                    }
                }
            }
        }
    }

    /// Decides `cmd` from `link` and answers it: `Refused`, or the
    /// sensor switched and `On`/`Off` (sent by the sensor's task).
    async fn command(table: &mut Table, link: Link, cmd: &Command) {
        let speed = link_speed(link, UART_BAUD);
        let sensor = match cmd {
            Command::MicOn | Command::MicOff => Sensor::Mic,
            Command::ImuOn(_) | Command::ImuOff => Sensor::Imu,
        };
        if let Decision::Refuse(reason) = table.request(link, cmd, speed) {
            reply(link, Message::Refused { sensor, reason }).await;
            return;
        }
        // Granted: the table already holds the new state; make it real.
        // An `ImuOn` with no half comes back with no holder: off, `Off`.
        let reply = Some(link);
        match sensor {
            Sensor::Mic => {
                let to = table.holder(Sensor::Mic).map(|holder| (holder, ()));
                MIC.apply(Switch { to, reply }).await;
            }
            Sensor::Imu => {
                let to = table.holder(Sensor::Imu).map(|holder| (holder, table.imu_config()));
                if !IMU.apply(Switch { to, reply }).await {
                    // The chip did not answer: it is off, so free it too.
                    // Granted: `link` holds the IMU, and an off needs 0 B/s.
                    let _ = table.request(link, &Command::ImuOff, speed);
                }
            }
        }
    }

    /// Frees every sensor `link` held and powers it off; nobody is told.
    async fn release(table: &mut Table, link: Link) {
        let mic = table.holder(Sensor::Mic) == Some(link);
        let imu = table.holder(Sensor::Imu) == Some(link);
        table.release_link(link);
        if mic {
            MIC.apply(Switch { to: None, reply: None }).await;
        }
        if imu {
            IMU.apply(Switch { to: None, reply: None }).await;
        }
    }

    /// Owns the microphone: switches it between two recordings and
    /// streams its blocks to the holder.
    #[embassy_executor::task]
    pub async fn mic_task(mut mic: Mic) -> ! {
        // The holder and the stream's counter, while the mic is on.
        let mut stream: Option<(Link, Counter)> = None;
        loop {
            if let Some((holder, counter)) = &mut stream {
                let holder = *holder;
                // Records until a switch is waiting; `record` is never
                // cancelled.
                let recorded = mic
                    .record(|block| {
                        if MIC.switch.signaled() {
                            return false;
                        }
                        if let Some(samples) = block {
                            // The counter advances even when the block
                            // is dropped.
                            let block = audio_block(counter.next(), samples);
                            let _ = outbox(holder).try_send(Message::Audio(block));
                        }
                        true
                    })
                    .await;
                if recorded.is_err() {
                    // DECISION: a PDM that will not record is turned off;
                    // the table still names the holder, whose API times
                    // out on the silent stream. Unreachable while every
                    // `record` runs to its end. The next switch sets the
                    // stream again.
                    mic.off();
                }
            }
            let switch = MIC.switch.wait().await;
            let answer = if let Some((holder, ())) = switch.to {
                mic.on().await;
                // DECISION: a fresh counter on every granted `MicOn`,
                // even from the holder: the API only re-sends it for a
                // new `Mic`, whose stream expects 0.
                stream = Some((holder, Counter::new()));
                Message::On(Sensor::Mic)
            } else {
                mic.off();
                stream = None;
                Message::Off(Sensor::Mic)
            };
            if let Some(link) = switch.reply {
                reply(link, answer).await;
            }
            MIC.done.signal(true);
        }
    }

    /// The IMU while it streams.
    struct ImuStream {
        holder: Link,
        config: ImuConfig,
        counter: Counter,
        /// Which halves each chip sample carries.
        pace: Pace,
        /// When the chip was last read.
        last: Instant,
    }

    /// The IMU with both halves off.
    const IMU_OFF: ImuConfig = ImuConfig { accel: None, gyro: None };

    /// Owns the IMU: switches it and streams its readings to the holder,
    /// paced by the chip's data-ready on INT1.
    #[embassy_executor::task]
    pub async fn imu_task(pins: ImuPins) -> ! {
        // DECISION: 2 bytes, the longest write `imu` sends (register +
        // value). Its constant arrays may sit in flash, which the TWIM's
        // DMA cannot read, so Twim copies them through this buffer.
        let mut tx_ram_buffer = [0; 2];
        let mut config = twim::Config::default();
        // DECISION: 400 kHz, the chip's fast mode: a 12-byte read takes
        // ~0.4 ms, well inside 833 Hz's 1.2 ms. The board's pull-ups
        // (powered with the chip) are used; the internal ones stay off.
        config.frequency = twim::Frequency::K400;
        let twim = twim::Twim::new(pins.twim, Irqs, pins.sda, pins.scl, config, &mut tx_ram_buffer);
        let mut imu = Lsm6ds3::new(twim);
        let mut power = pins.power;
        // DECISION: pulled down, so the pin does not float while the
        // chip is unpowered; the chip drives it push-pull when on.
        let mut int1 = Input::new(pins.int1, Pull::Down);
        let mut stream: Option<ImuStream> = None;
        loop {
            let switch = match &stream {
                None => Some(IMU.switch.wait().await),
                // Waiting is cut short by a switch; an I2C transfer only
                // by `I2C_TIMEOUT` (`bus`).
                Some(s) => {
                    // DECISION: the chip paces the reads (data-ready on
                    // INT1, latched until the read), so no chip sample is
                    // sent twice or skipped when its oscillator is off
                    // nominal. If INT1 stays low a quarter period past
                    // due, the chip is read anyway: a broken INT1 slows
                    // the stream instead of stopping it.
                    // INT1 is only looked at from 3/4 of a period after
                    // the last read (`Pace::floor_us`): a pin stuck high
                    // reads at most at 4/3 the rate, not back to back.
                    let floor = s.last + Duration::from_micros(s.pace.floor_us());
                    let late = s.last + Duration::from_micros(s.pace.period_us() * 5 / 4);
                    let int1 = &mut int1;
                    let sample = async move {
                        Timer::at(floor).await;
                        select(int1.wait_for_high(), Timer::at(late)).await
                    };
                    match select(IMU.switch.wait(), sample).await {
                        Either::First(switch) => Some(switch),
                        Either::Second(_) => None,
                    }
                }
            };
            if let Some(switch) = switch {
                let before = stream.take().map_or(IMU_OFF, |s| s.config);
                let (ok, answer) =
                    imu_switch(&mut imu, &mut power, before, switch.to, &mut stream).await;
                if let Some(link) = switch.reply {
                    reply(link, answer).await;
                }
                IMU.done.signal(ok);
                continue;
            }
            if let Some(s) = &mut stream {
                s.last = Instant::now();
                let (accel_on, gyro_on) = s.pace.tick();
                // Read every sample, carried or not: the read clears the
                // data-ready latch.
                let Some(read) = bus(imu.read()).await else {
                    // DECISION: a read that hangs is a failed chip, as in
                    // `imu_switch`: power off, stream over, and `Off` to
                    // the holder, whose API ends its streams on it. The
                    // table still names the holder, as for the mic; its
                    // next `ImuOn` powers the chip again.
                    power.off();
                    let holder = s.holder;
                    stream = None;
                    reply(holder, Message::Off(Sensor::Imu)).await;
                    continue;
                };
                if accel_on || gyro_on {
                    let counter = s.counter.next();
                    // DECISION: a failed read sends nothing; the counter
                    // has advanced, so the API sees the gap.
                    if let Ok((accel, gyro)) = read {
                        let reading = ImuReading {
                            counter,
                            accel: accel_on.then_some(accel),
                            gyro: gyro_on.then_some(gyro),
                        };
                        let _ = outbox(s.holder).try_send(Message::Imu(reading));
                    }
                }
            }
        }
    }

    /// Runs one IMU call on the I2C bus for at most [`I2C_TIMEOUT`].
    /// `None` when it ran out; the TWIM is then stopped.
    async fn bus<F: core::future::Future>(call: F) -> Option<F::Output> {
        let done = with_timeout(I2C_TIMEOUT, call).await.ok();
        if done.is_none() {
            // DECISION: the TWIM driver does not stop a transfer whose
            // future is dropped, so it is stopped here (TWISPI0 is TWIM0,
            // `board::ImuPins`), through `embassy_nrf::pac` as in
            // `link::ble`: a bus that came back would otherwise let its
            // DMA write into memory the next call reuses. The next call
            // sets the transfer up from scratch.
            embassy_nrf::pac::TWIM0.tasks_stop().write_value(1);
        }
        done
    }

    /// Switches the IMU from `before` to `to` and sets `stream`: on
    /// (power, registers, a fresh stream) or off (power cut). Returns
    /// whether it worked and the reply.
    async fn imu_switch<I: embedded_hal_async::i2c::I2c>(
        imu: &mut Lsm6ds3<I>,
        power: &mut crate::board::Power,
        before: ImuConfig,
        to: Option<(Link, ImuConfig)>,
        stream: &mut Option<ImuStream>,
    ) -> (bool, Message) {
        // A config with no half on is off (the table never sends one as
        // on); `Pace::new` is `None` for it.
        let target =
            to.and_then(|(holder, config)| Some((holder, config, Pace::new(before, config)?)));
        let Some((holder, config, pace)) = target else {
            // DECISION: off = power cut, no register write: the chip
            // forgets everything, and the next `on` writes it all again
            // after the settle time.
            power.off();
            *stream = None;
            return (true, Message::Off(Sensor::Imu));
        };
        power.on().await;
        if !matches!(bus(imu.apply(config)).await, Some(Ok(()))) {
            // DECISION: no chip (or a stuck bus), no stream of zeros:
            // power off and say `Off`, the truth. The API was waiting for
            // `On`, so it times out.
            power.off();
            *stream = None;
            return (false, Message::Off(Sensor::Imu));
        }
        // DECISION: a fresh counter on every granted `ImuOn`, even a
        // reconfiguration by the holder: the API sends one for every new
        // value, whose stream expects 0, and a stream already running
        // reads a counter that goes back as a restart, not as drops
        // (`xiao-api` `stream.rs`). Same rule as the mic.
        let counter = Counter::new();
        *stream = Some(ImuStream { holder, config, counter, pace, last: Instant::now() });
        (true, Message::On(Sensor::Imu))
    }
}

/// Off the board there is nothing to run; this keeps `cargo test
/// --workspace` building on the host.
#[cfg(not(target_os = "none"))]
fn main() {}
