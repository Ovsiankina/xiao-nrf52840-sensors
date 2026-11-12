//! The board's wiring: which pins the sensors are on and how each one
//! is powered on and off.
//!
//! Pins and timings come from `docs/research/sensor-crates.md` (Seeed
//! schematic V1.1, Seeed Arduino cores, Zephyr devicetree).
//!
//! Depends on: nothing in this workspace.

use embassy_nrf::gpio::{Level, Output, OutputDrive, Pin};
use embassy_nrf::interrupt::{self, InterruptExt, Priority};
use embassy_nrf::peripherals::{
    P0_07, P0_11, P0_16, P0_27, P1_00, P1_11, P1_12, PDM, PPI_CH0, PPI_CH1, PPI_GROUP0, RNG,
    TIMER1, TWISPI0, UARTE0, USBD,
};
use embassy_nrf::{Peri, Peripherals, bind_interrupts, buffered_uarte, pdm, twim, usb};
use embassy_time::{Duration, Timer};

/// IMU power settle time: the LSM6DS3TR-C needs ≥ 10 ms before the
/// first I2C access and 35 ms to boot.
// DECISION: wait the full 35 ms before the registers are written. This
// covers the boot only: each half then needs its own turn-on time after
// its rate is set, which the IMU task waits per half (`pace`).
const IMU_SETTLE: Duration = Duration::from_millis(35);

/// Mic power settle time: MSM261 power-up time is 6 ms typ, 20 ms max.
const MIC_SETTLE: Duration = Duration::from_millis(20);

/// Interrupt priority for every peripheral the application drives.
// DECISION: P2. The MPSL (BLE, fw-06) takes P0 for RADIO/RTC0/TIMER0
// and P4 for its low-priority work; application interrupts must stay
// off P0, and P2 keeps them above the executor-thread work.
pub const APP_PRIORITY: Priority = Priority::P2;

bind_interrupts!(
    /// The interrupt bindings of the whole firmware.
    ///
    /// Each interrupt may be bound only once in the binary, so every
    /// task adds its lines here. `CLOCK_POWER` is shared: USB VBUS
    /// detection and the MPSL clock handler both run on it. The RNG has
    /// no line: the BLE link drives it blocking (`link::ble`).
    pub struct Irqs {
        TWISPI0 => twim::InterruptHandler<TWISPI0>;
        PDM => pdm::InterruptHandler<PDM>;
        UARTE0 => buffered_uarte::InterruptHandler<UARTE0>;
        USBD => usb::InterruptHandler<USBD>;
        CLOCK_POWER => usb::vbus_detect::InterruptHandler, nrf_mpsl::ClockInterruptHandler;
        // The MPSL: its low-priority work, and the radio timing it owns.
        EGU0_SWI0 => nrf_mpsl::LowPrioInterruptHandler;
        RADIO => nrf_mpsl::HighPrioInterruptHandler;
        TIMER0 => nrf_mpsl::HighPrioInterruptHandler;
        RTC0 => nrf_mpsl::HighPrioInterruptHandler;
    }
);

/// The `embassy_nrf::init` configuration for this board.
// DECISION: the GPIOTE and time-driver interrupts default to P0, which
// collides with the MPSL; move them to `APP_PRIORITY` like the rest.
// DECISION: the 32 MHz crystal runs from boot: the USB peripheral needs
// it (the internal RC is too imprecise for USB). Once the MPSL owns the
// clocks it may stop the crystal between radio events, so `main` holds
// an `Hfclk` from `request_hfclk` for USB to keep working.
pub fn config() -> embassy_nrf::config::Config {
    let mut config = embassy_nrf::config::Config::default();
    config.hfclk_source = embassy_nrf::config::HfclkSource::ExternalXtal;
    config.gpiote_interrupt_priority = APP_PRIORITY;
    config.time_interrupt_priority = APP_PRIORITY;
    config
}

/// A sensor's power pin: high = powered.
///
/// Not cancel-safe: dropping `on()` mid-wait leaves the pin high with
/// the settle time not waited, and the next `on()` will not wait it.
pub struct Power {
    pin: Output<'static>,
    settle: Duration,
}

impl Power {
    /// Takes `pin` as an output, driven low (sensor off).
    pub fn new(pin: Peri<'static, impl Pin>, drive: OutputDrive, settle: Duration) -> Self {
        Self { pin: Output::new(pin, Level::Low, drive), settle }
    }

    /// Powers the sensor and waits its settle time. Returns at once if
    /// it is already powered.
    pub async fn on(&mut self) {
        // DECISION: a second `on` does not wait again, so a link that
        // reconfigures a running sensor does not stall its stream.
        if self.pin.is_set_high() {
            return;
        }
        self.pin.set_high();
        Timer::after(self.settle).await;
    }

    /// Cuts the sensor's power.
    pub fn off(&mut self) {
        self.pin.set_low();
    }
}

/// The IMU (LSM6DS3TR-C) on I2C. Address 0x6A is in `imu`.
pub struct ImuPins {
    /// The I2C peripheral (TWIM0; its instance is shared with SPIM0).
    pub twim: Peri<'static, TWISPI0>,
    /// I2C data, P0.07.
    pub sda: Peri<'static, P0_07>,
    /// I2C clock, P0.27.
    pub scl: Peri<'static, P0_27>,
    /// The chip's INT1 (data-ready, `imu::Lsm6ds3::apply`), P0.11.
    pub int1: Peri<'static, P0_11>,
    /// P1.08: powers the chip and both I2C pull-ups.
    pub power: Power,
}

/// The PDM microphone (MSM261D3526H1CPM, mono, L/R = GND).
pub struct MicPins {
    /// The PDM peripheral.
    pub pdm: Peri<'static, PDM>,
    /// PDM clock out, P1.00.
    pub clk: Peri<'static, P1_00>,
    /// PDM data in, P0.16.
    pub data: Peri<'static, P0_16>,
    /// P1.10: powers the microphone.
    pub power: Power,
}

/// The UART link on the header pins.
// VERIFIED: TX = D6 = P1.11, RX = D7 = P1.12, against the Seeed wiki pin
// map for the XIAO nRF52840 Sense, Seeed's Arduino `variant.cpp`
// ("D6 is P1.11 (TX)", "D7 is P1.12 (RX)") and the schematic pad names
// `P1.11_TX_D6`, `P1.12_RX_D7`.
pub struct UartPins {
    /// The UART peripheral.
    pub uarte: Peri<'static, UARTE0>,
    /// Counts received bytes for the buffered UART (`link::uart`).
    pub timer: Peri<'static, TIMER1>,
    /// PPI channel: each received byte counts on `timer`.
    pub ppi_ch1: Peri<'static, PPI_CH0>,
    /// PPI channel: a full RX buffer restarts reception.
    pub ppi_ch2: Peri<'static, PPI_CH1>,
    /// PPI group the buffered UART switches `ppi_ch2` with.
    pub ppi_group: Peri<'static, PPI_GROUP0>,
    /// Transmit, P1.11 (D6).
    pub tx: Peri<'static, P1_11>,
    /// Receive, P1.12 (D7).
    pub rx: Peri<'static, P1_12>,
}

/// What the BLE stack (nrf-sdc on top of nrf-mpsl) takes for itself.
pub struct BlePins {
    /// RTC0, TIMER0, TEMP, PPI channels 19, 30, 31.
    pub mpsl: nrf_mpsl::Peripherals<'static>,
    /// PPI channels 17, 18, 20 to 29.
    pub sdc: nrf_sdc::Peripherals<'static>,
    /// The controller's random numbers.
    pub rng: Peri<'static, RNG>,
}

/// Every peripheral the firmware uses, grouped by what uses it.
pub struct Board {
    /// The IMU.
    pub imu: ImuPins,
    /// The microphone.
    pub mic: MicPins,
    /// The UART link.
    pub uart: UartPins,
    /// The USB device peripheral.
    pub usb: Peri<'static, USBD>,
    /// The BLE stack's peripherals.
    pub ble: BlePins,
}

impl Board {
    /// Splits the chip's peripherals into the board's parts. Both
    /// sensors start powered off (FW-6).
    pub fn take(p: Peripherals) -> Self {
        for irq in [
            interrupt::TWISPI0,
            interrupt::PDM,
            interrupt::UARTE0,
            interrupt::USBD,
            // USB VBUS detection; the MPSL moves it to P4 when it starts.
            interrupt::CLOCK_POWER,
        ] {
            irq.set_priority(APP_PRIORITY);
        }
        Self {
            imu: ImuPins {
                twim: p.TWISPI0,
                sda: p.P0_07,
                scl: p.P0_27,
                int1: p.P0_11,
                // High drive: the pin feeds the chip, VDDIO, CS and both
                // pull-ups (research file).
                power: Power::new(p.P1_08, OutputDrive::HighDrive, IMU_SETTLE),
            },
            mic: MicPins {
                pdm: p.PDM,
                clk: p.P1_00,
                data: p.P0_16,
                // DECISION: high drive for the mic too. Standard drive
                // is specified to 0.5 mA and the mic draws ~0.7 mA.
                power: Power::new(p.P1_10, OutputDrive::HighDrive, MIC_SETTLE),
            },
            uart: UartPins {
                uarte: p.UARTE0,
                timer: p.TIMER1,
                ppi_ch1: p.PPI_CH0,
                ppi_ch2: p.PPI_CH1,
                ppi_group: p.PPI_GROUP0,
                tx: p.P1_11,
                rx: p.P1_12,
            },
            usb: p.USBD,
            ble: BlePins {
                mpsl: nrf_mpsl::Peripherals::new(
                    p.RTC0, p.TIMER0, p.TEMP, p.PPI_CH19, p.PPI_CH30, p.PPI_CH31,
                ),
                sdc: nrf_sdc::Peripherals::new(
                    p.PPI_CH17, p.PPI_CH18, p.PPI_CH20, p.PPI_CH21, p.PPI_CH22, p.PPI_CH23,
                    p.PPI_CH24, p.PPI_CH25, p.PPI_CH26, p.PPI_CH27, p.PPI_CH28, p.PPI_CH29,
                ),
                rng: p.RNG,
            },
        }
    }
}
