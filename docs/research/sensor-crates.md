# Research: sensor crates and board facts (2026-11-12)

Input for the SRS (step 2), not policy. Four research agents + skeptics
checked these against primary sources (Seeed schematic V1.1, Seeed
Arduino cores, Zephyr devicetree, crate sources). The flashing skeptic
was stopped early; flashing is settled by `scripts/flash.sh` from
xiao-nrf52840-dead-simple-math.

## Board (confirmed by skeptic)

| what | value |
|---|---|
| IMU | LSM6DS3TR-C, I2C SDA P0.07, SCL P0.27, addr 0x6A (SA0=GND), INT1 P0.11, INT2 not wired |
| IMU power | P1.08 active high; powers VDD, VDDIO, CS and both I2C pull-ups. Drive high-drive; wait ≥10 ms before first I2C (35 ms before data valid) |
| Mic | MSM261D3526H1CPM (wiki links WMM7035DTTJ0, same pinout); PDM CLK P1.00, DATA P0.16, L/R=GND (mono) |
| Mic power | P1.10 active high; power-up time 6 ms typ / 20 ms max |
| USB 2886:0045 | the Sense's UF2 bootloader. An app with no USB stack does not enumerate |
| Memory | bootloader + S140 7.3.0 reserve flash up to 0x27000 (156K). The bootloader refuses writes ≥ 0xEA000 |

## IMU driver: no candidate is clean

| crate | coverage | problem |
|---|---|---|
| `lsm6ds3tr` 0.2.2 (MIT, blocking, e-h 1.0) | accel/gyro/temp in g/dps/°C, accel ODR+scale, tap/free-fall/wake-up/inactivity IRQs | `extern crate alloc`: every binary needs a `#[global_allocator]` = new dep + `unsafe`. Gyro ODR/scale types not exported (stuck 416 Hz / 250 dps; MR !7 unmerged). IRQ bugs (wake-up threshold overwritten, `disable_irqs` no-op). No FIFO. Maintainer idle since 2025-05 |
| `lsm6ds3trc` 0.1.0 (Apache-2.0, async, e-h-async 1.0) | accel/gyro/temp, ODR+scale for both, correct scales | `Transport` trait not exported: driver cannot be a field of our struct without a one-line fork/`[patch]`. No IRQs/FIFO. 26 downloads, 0 tests |
| `imu-sensors` 0.1.7 | async | `read_register` always returns 0 → `init()` fails on real hardware; gyro scale wrong; forces embassy-time `generic-queue-8` |
| ST official | none for this part | C driver `lsm6ds3tr-c-pid` (BSD-3) is a register reference |
| `lsm6ds33` 0.5.0 | — | rejects WHO_AM_I 0x6A, e-h 0.2 |

## HAL: embassy-nrf 0.11.0

- `Twim` implements blocking `embedded_hal::i2c::I2c` and async
  `embedded_hal_async::i2c::I2c`: every IMU candidate plugs in.
- `pdm::Pdm::new(pdm, irqs, clk, din, config)` (clk first; the two
  official examples disagree on order).
- PDM traps: default clock 1.032 MHz is outside the mic's range → set
  1.280 MHz (16 kHz). Mic datasheet says rising edge for L/R=GND;
  Seeed, Zephyr and embassy all use `LeftFalling`.
- `sample()` and `run_task_sampler` are not cancel-safe in 0.11.0 (stop
  loop condition inverted): no `select`/timeout around them; keep PDM
  buffers in storage that outlives the future.
- Twim: no copy-from-flash fallback beyond the caller's `tx_ram_buffer`.

Full agent output (session-local):
`/tmp/claude-1000/-home-ovsiankina-Projects-km-robota-projects-Architecture-xiao-nrf52840-sensors/61bedbdb-01a0-4b62-869f-f8b0b1c764bd/scratchpad/research-results.txt`
