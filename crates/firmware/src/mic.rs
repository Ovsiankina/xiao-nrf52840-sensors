//! The PDM microphone: power, clock, and audio blocks.
//!
//! Traps (`docs/research/sensor-crates.md`): the PDM clock must be set
//! to 1.280 MHz (the default 1.032 MHz is outside the mic's range), and
//! the PDM driver's futures are not cancel-safe in embassy-nrf 0.11.0.
//! So [`Mic::record`] must never be wrapped in `select` or a timeout:
//! it stops itself when its callback says so.
//!
//! Depends on: `board`.

use embassy_nrf::interrupt::typelevel::{Binding, PDM as PdmIrq};
use embassy_nrf::pdm::{self, Edge, Frequency, OperationMode, Pdm, Ratio, SamplerState};
use embassy_nrf::peripherals::PDM;
use static_cell::StaticCell;
use xiao_proto::message::{AUDIO_BLOCK_BYTES, AUDIO_BLOCK_SAMPLES, AudioBlock};

use crate::board::{MicPins, Power};

/// Blocks thrown away each time the clock starts, before audio is valid.
// DECISION: 4 blocks (24 ms), the first whole number of 6 ms blocks
// past 20 ms. The datasheet's 200 µs wake-up assumes a powered mic;
// counting its 20 ms max power-up again from the first clock edge keeps
// start-up noise out of the first block sent.
const SETTLE_BLOCKS: u32 = 4;

/// One block of samples.
pub type Samples = [i16; AUDIO_BLOCK_SAMPLES];

/// The two blocks the PDM fills in turn, by DMA.
// DECISION: in a `StaticCell`, not on the stack of `record()`: if the
// sampling future is ever dropped mid-transfer, the PDM DMA still
// points at memory that lives forever (research file trap).
static BUFFERS: StaticCell<[Samples; 2]> = StaticCell::new();

/// The microphone: 16,000 mono samples/s in blocks of 96.
pub struct Mic {
    pdm: Pdm<'static>,
    power: Power,
    buffers: &'static mut [Samples; 2],
}

/// `samples` as the [`AudioBlock`] numbered `counter`: 96 little-endian
/// `i16`.
pub fn audio_block(counter: u32, samples: &Samples) -> AudioBlock {
    let mut bytes = [0u8; AUDIO_BLOCK_BYTES];
    for (pair, sample) in bytes.chunks_exact_mut(2).zip(samples.iter()) {
        pair.copy_from_slice(&sample.to_le_bytes());
    }
    AudioBlock { counter, pcm: heapless::Vec::from_array(bytes) }
}

impl Mic {
    /// Sets up the PDM peripheral; the mic stays powered off (FW-6).
    ///
    /// # Panics
    ///
    /// When called twice (the sample buffers are a single static).
    pub fn new(
        pins: MicPins,
        irqs: impl Binding<PdmIrq, pdm::InterruptHandler<PDM>> + 'static,
    ) -> Self {
        let config = pdm::Config {
            operation_mode: OperationMode::Mono,
            // DECISION: left channel on the falling edge, as Seeed, Zephyr
            // and embassy do for this board, although the mic datasheet
            // says rising for L/R = GND (research file).
            edge: Edge::LeftFalling,
            // 32 MHz / 25 = 1.280 MHz; ratio 80 → 16,000 samples/s (SRS 4.2).
            // DECISION: 1.280 MHz is inside the MSM261's 1.1–4.0 MHz range;
            // the wiki's alternative part (WMM7035) asks 1.3 MHz min, so a
            // board carrying it would run 1.5 % under spec. The only other
            // 16 kHz setting (1.024 MHz, ratio 64) is further below.
            frequency: Frequency::_1280k,
            ratio: Ratio::Ratio80,
            // Gain stays at the default 0 dB.
            ..pdm::Config::default()
        };
        Self {
            pdm: Pdm::new(pins.pdm, irqs, pins.clk, pins.data, config),
            power: pins.power,
            buffers: BUFFERS.init([[0; AUDIO_BLOCK_SAMPLES]; 2]),
        }
    }

    /// Powers the mic and waits its power-up time. Does nothing if it
    /// is already on. The clock runs only inside [`Mic::record`].
    pub async fn on(&mut self) {
        self.power.on().await;
    }

    /// Cuts the mic's power. Call it only between two `record`s.
    pub fn off(&mut self) {
        self.power.off();
    }

    /// Starts the clock and calls `each` every 96 samples (6 ms), with
    /// the block, without a gap between blocks, until `each` returns
    /// `false`; then stops the clock. The first blocks after the clock
    /// starts are not valid audio: `each` gets `None` for them.
    ///
    /// Not cancel-safe: never put it in `select` or a timeout.
    // DECISION: the double-buffered `run_task_sampler`, not one
    // `Pdm::sample` per block: `sample` points the DMA back at its dummy
    // buffer before the next call, so at least one sample was lost
    // between every two blocks (16 kHz in name only). Here the next
    // buffer is armed while the current one fills: 16,000 samples/s.
    // `each` runs in the task, between two buffers: it must take less
    // than one block (6 ms), and only queues a message.
    // DECISION: an error (the PDM already running) is returned and the
    // caller turns the mic off; it cannot happen when every `record`
    // ran to its end, which stops the PDM.
    pub async fn record(
        &mut self,
        mut each: impl FnMut(Option<&Samples>) -> bool,
    ) -> Result<(), pdm::Error> {
        let mut settle = SETTLE_BLOCKS;
        self.pdm
            .run_task_sampler(self.buffers, |samples| {
                let block = if settle > 0 {
                    settle -= 1;
                    None
                } else {
                    Some(samples)
                };
                if each(block) { SamplerState::Sampled } else { SamplerState::Stopped }
            })
            .await
    }
}
