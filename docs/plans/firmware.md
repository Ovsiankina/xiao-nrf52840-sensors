# firmware Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The binary flashed on the XIAO nRF52840 Sense: it takes commands on USB, BLE and UART, powers only the sensors asked for, and streams their readings back.

**Architecture:** One embassy task per link reads bytes into a `proto::frame::Decoder` and forwards `(Link, Command)` to the main task over a channel. The main task asks `sensors::Table` whether to grant, switches the sensor, and replies. One task per sensor reads it and writes `Message`s to the channel of the link that holds it. `sensors` and `imu` are hardware-free and tested on the host.

**Tech Stack:** `embassy-nrf` 0.11, `embassy-executor` 0.10, `embassy-usb` 0.6, `trouble-host` 0.8 + `nrf-sdc` 0.4, `xiao-proto`.

**Spec:** `docs/srs.md` §4.2, `docs/plans/2026-11-12-hld.md`, pins and traps in `docs/research/sensor-crates.md`.

## Global Constraints

- No `unsafe`, no new dependency; either one is a deferral (`CLAUDE.md`).
- Host: `cargo test -p xiao-firmware`. Board: `cargo build -p xiao-firmware --release --target thumbv7em-none-eabihf` must stay green after every task.
- One test per requirement, named after it: `fw_1_…` … `fw_6_…`.
- Pins: IMU SDA P0.07, SCL P0.27, addr 0x6A, power P1.08 (high drive, wait 35 ms). Mic CLK P1.00, DATA P0.16, power P1.10 (wait 20 ms). UART TX P1.11 (D6), RX P1.12 (D7).
- PDM: 1.280 MHz clock (16 kHz), never wrapped in `select` or a timeout; buffers `'static`.
- Board-only tasks (fw-03 to fw-06) cannot be tested on the host; their check is the board build plus the manual steps in fw-06.

## Review Focus

- Second link asks for a held sensor → `Refused { Busy { held_by } }`, the holder is undisturbed (fw-01).
- The same link asks again for a sensor it holds → granted, new config applied, not "busy" (fw-01).
- `ImuOn` with neither half selected → treated as `ImuOff` (fw-01).
- Chip does not answer WHO_AM_I 0x6A → error, no stream of zeros (fw-02).
- A link's outgoing channel is full → the reading is dropped and the counter still advances, so the API sees the gap (fw-05).

---

### Task fw-01: who holds which sensor

**Files:** `crates/firmware/src/sensors.rs`

**Produces:**

```rust
pub struct Table { /* private: Option<Link> per sensor, ImuConfig */ }
pub enum Decision { Grant, Refuse(Refusal) }
impl Table {
    pub const fn new() -> Self;
    pub fn request(&mut self, link: Link, cmd: &Command, link_speed: u32) -> Decision;
    pub fn holder(&self, sensor: Sensor) -> Option<Link>;
    pub fn imu_config(&self) -> ImuConfig;          // both halves None when off
    pub fn release_link(&mut self, link: Link);     // frees everything that link holds
}
pub const fn link_speed(link: Link, uart_baud: u32) -> u32;  // bytes/s: Usb 500_000, Uart baud/10, Ble 60_000
pub fn needs(cmd: &Command) -> u32;  // bytes/s: mic 34_000; imu = rate × 16 per enabled half (rounded up)
pub struct Counter(u32);  impl Counter { pub const fn new() -> Self; pub fn next(&mut self) -> u32; }  // wraps
```

`// DECISION:` the BLE figure (60_000) is a placeholder for the SRS hole "whether BLE can carry the mic"; measured in fw-06.

- [ ] Tests: `fw_1_busy_names_holding_link`, `fw_1_two_sensors_two_links`, `fw_2_imu_halves_share_one_link`, `fw_3_uart_default_is_921600` (`link_speed(Uart, UART_DEFAULT_BAUD) == 92_160`), `fw_4_mic_refused_on_slow_uart` (115200 → `TooSlow { needs: 34_000, link_has: 11_520 }`), `fw_5_counter_counts_and_wraps`, `fw_6_unrequested_half_stays_off`, `same_link_may_reconfigure`, `release_link_frees_its_sensors`. Implement, commit.

### Task fw-02: IMU registers

**Files:** `crates/firmware/src/imu.rs`

**Produces:**

```rust
pub struct Lsm6ds3<I2C> { /* private */ }
pub enum ImuError<E> { Bus(E), WrongChip(u8) }
impl<I2C: embedded_hal_async::i2c::I2c> Lsm6ds3<I2C> {
    pub fn new(i2c: I2C) -> Self;                       // address 0x6A
    pub async fn apply(&mut self, cfg: ImuConfig) -> Result<(), ImuError<I2C::Error>>;  // checks WHO_AM_I, sets BDU + IF_INC, CTRL1_XL, CTRL2_G; None = power-down
    pub async fn read(&mut self, cfg: ImuConfig) -> Result<([i16; 3], [i16; 3]), ImuError<I2C::Error>>;  // (accel, gyro) raw; one 12-byte read from 0x22
}
```

Registers: WHO_AM_I 0x0F (=0x6A), CTRL1_XL 0x10, CTRL2_G 0x11, CTRL3_C 0x12 (BDU bit 6, IF_INC bit 2), OUTX_L_G 0x22 (gyro x,y,z then accel x,y,z, little-endian). ODR field (bits 7:4): 12.5 Hz=1, 26=2, 52=3, 104=4, 208=5, 416=6, 833=7. FS_XL (bits 3:2): 2 g=0, 16 g=1, 4 g=2, 8 g=3. FS_G (bits 3:2): 250=0, 500=1, 1000=2, 2000=3; 125 dps = bit 1.

- [ ] Tests with an in-file fake I2C: `apply_writes_ctrl_registers` (Hz104/G4 + Hz104/Dps500 → CTRL1_XL 0x48, CTRL2_G 0x44), `off_half_is_powered_down` (0x00), `wrong_chip_is_an_error`, `read_splits_gyro_then_accel`. Run with `embassy_futures::block_on`. Implement, commit.

### Task fw-03: board and microphone

**Files:** `crates/firmware/src/board.rs`, `crates/firmware/src/mic.rs`

**Produces:** `board::Board::take(Peripherals) -> Board` with fields `imu: ImuPins` (twim peripheral, sda, scl, power), `mic: MicPins` (pdm, clk, data, power), `uart: UartPins`, `usb`, `ble` (the peripherals nrf-sdc/mpsl need); `board::Power` (wraps an `Output`, `on().await` waits the settle time, `off()`). `mic::Mic::new(MicPins, irqs)`, `mic.on().await`, `mic.off()`, `mic.block().await -> AudioBlock` payload of 96 samples.

- [ ] Implement, board build green, commit.

### Task fw-04: USB and UART links

**Files:** `crates/firmware/src/link.rs`, `link/usb.rs`, `link/uart.rs`

**Produces:** `link::Inbox` = `Channel<(Link, Command), 4>`; `link::Outbox` = `Channel<Message, 8>` per link; `usb::run(usb, &Inbox, &Outbox) -> !` (CDC-ACM, vendor id 0x2886, product "XIAO-Sense"), `uart::run(pins, baud, &Inbox, &Outbox) -> !`. On USB disconnect: post nothing, call back `on_down(Link::Usb)` so main releases its sensors.

- [ ] Implement, board build green, commit.

### Task fw-05: main loop

**Files:** `crates/firmware/src/main.rs`

**Produces:** spawns the link tasks, then loops on `Inbox`: `Table::request` → `Refused` or switch the sensor and reply `On`/`Off`; sensor tasks push `Audio` / `Imu` messages with `Counter` to the holder's `Outbox` using `try_send` (full = dropped).

- [ ] Implement, board build green, commit.

### Task fw-06: BLE link and bring-up

**Files:** `crates/firmware/src/link/ble.rs`

**Produces:** `ble::run(ble, &Inbox, &Outbox) -> !`: advertises as `BLE_NAME`, Nordic UART Service, write characteristic → `Decoder`, `Outbox` → notifications; on disconnect `on_down(Link::Ble)`.

- [ ] Implement, board build green, commit.
- [ ] Manual, on the board: `./scripts/flash.sh`, then run each `xiao-api` example; note the measured BLE throughput in `docs/srs.md`'s hole (owner edits) via `/issue question`.
