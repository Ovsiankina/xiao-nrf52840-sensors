# api Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The crate a developer calls to switch the XIAO's sensors on over a link and read them, exactly as shown in the HLD's "API surface".

**Architecture:** Each link (`Usb`, `Ble`, `Uart`) owns a background reader that decodes frames and routes each message to the sensor it belongs to through channels. Sensors (`Mic`, `Gyro`, `Accel`, `Imu`) send one command, wait for `On` or `Refused`, then iterate over their channel. All link types share one private trait, `Transport` (write bytes, read bytes), so everything above it is tested on the host with an in-memory transport.

**Tech Stack:** `xiao-proto`, `serialport` 4.10 (USB, UART), `btleplug` 0.13 + `tokio` (BLE), `std::sync::mpsc`.

**Spec:** `docs/srs.md` §4.1, `docs/plans/2026-11-12-hld.md` ("API surface" is the contract's source).

## Global Constraints

- Blocking public API; no `async` in any `pub` signature.
- Outputs are newtypes, never bare numbers.
- No `unsafe`, no new dependency. Commit message starts with the task ID.
- One test per requirement, named after it: `api_1_…`, `api_2_…`, `api_3_…`, `api_4_…`.
- Host tests only use the in-memory transport; nothing in `cargo test` needs a board.

## Review Focus

- The firmware answers `Refused` → the developer gets `Error::Busy` / `Error::TooSlow` with the same values (api-04).
- The link dies mid-stream → the iterator yields one `Err`, then ends; it never blocks forever (api-03).
- A counter gap of N → `dropped()` grows by N; a counter that wraps at `u32::MAX` is not a drop (api-03).
- A sensor value dropped → the `Off` command is sent exactly once (api-04).
- No reply to a command within 2 s → `Error::Timeout`, not a hang (api-02).

---

### Task api-01: units and errors

**Files:** `crates/api/src/error.rs`, `crates/api/src/lib.rs` (re-exports)

**Produces:**

```rust
pub struct G(pub f32); pub struct Dps(pub f32); pub struct Hertz(pub u32);
pub struct Baud(pub u32); pub struct BytesPerSec(pub u32); pub struct Dropped(pub u64);
pub enum Error {
    Busy { held_by: Link }, TooSlow { needs: BytesPerSec, link_has: BytesPerSec },
    Timeout { attempts: u8 }, NotFound, Disconnected, Io(std::io::Error),
}   // impl Display, std::error::Error
pub use xiao_proto::message::{AccelRange, GyroRange, Link, Rate};
```

- [ ] Test `error_messages_name_the_cause` (Busy names the link; TooSlow shows both numbers). Implement, commit.

### Task api-02: link core

**Files:** `crates/api/src/link.rs`

**Produces:** private `trait Transport: Send { fn write(&mut self, bytes: &[u8]) -> io::Result<()>; fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>; }`; private `struct Connection` (reader thread + per-sensor `mpsc` routes) with `fn request(&self, cmd: Command) -> Result<(), Error>` (2 s reply timeout) and `fn subscribe(&self, sensor: Sensor) -> Receiver<Result<Message, Error>>`; `pub(crate) fn memory_pair()` test transport; a sealed public `trait AsLink` implemented by `Usb`, `Ble`, `Uart`.

- [ ] Tests: `api_1_sensor_goes_on_over_the_link_it_was_given`, `request_times_out_without_reply`, `refusal_becomes_error`. Implement, commit.

### Task api-03: stream

**Files:** `crates/api/src/stream.rs`

**Produces:** private `struct Stream<T>` wrapping a `Receiver`: `next()`, `latest()` (drains, returns newest; blocks for the first), `dropped() -> Dropped` from counter gaps (wrapping subtraction).

- [ ] Tests: `api_3_stream_and_latest`, `api_4_counter_gap_is_reported`, `counter_wrap_is_not_a_drop`, `dead_link_ends_the_stream_with_one_error`. Implement, commit.

### Task api-04: sensors

**Files:** `crates/api/src/mic.rs`, `crates/api/src/imu.rs`

**Produces:**

```rust
pub struct Mic<'l>;  impl Mic<'_> { pub fn on(link: &impl AsLink) -> Result<Mic<'_>, Error>; pub fn latest(&self) -> Result<AudioBlock, Error>; pub fn dropped(&self) -> Dropped; }
impl Iterator for &Mic<'_> { type Item = Result<AudioBlock, Error>; }   // Drop sends MicOff
pub struct AudioBlock;  impl AudioBlock { pub fn samples(&self) -> &[i16]; pub fn rate(&self) -> Hertz; }
pub struct Acceleration { pub x: G, pub y: G, pub z: G }
pub struct Rotation { pub x: Dps, pub y: Dps, pub z: Dps }
pub struct Motion { pub accel: Acceleration, pub gyro: Rotation }
pub struct Gyro<'l>;  // on(&link), rate(Rate) -> GyroBuilder, range(GyroRange) -> GyroBuilder, latest() -> Rotation, dropped(), Iterator<Item = Result<Rotation, Error>>
pub struct Accel<'l>; // same shape, AccelRange, Acceleration
pub struct Imu<'l>;   // on(&link), builder with accel_range/gyro_range/rate, Motion
```

Raw counts become `G` / `Dps` with `AccelRange::g_per_count` / `GyroRange::dps_per_count` of the config that was sent.

- [ ] Tests (in-memory firmware stand-in): `mic_iterates_audio_blocks`, `gyro_counts_become_dps` (raw 1000 at Dps500 → `Dps(17.5)`), `accel_counts_become_g` (raw 16384 at G2 → ≈`G(0.9994)`), `drop_sends_off_once`, `busy_names_the_holding_link`. Implement, commit.

### Task api-05: real links

**Files:** `crates/api/src/link/usb.rs`, `uart.rs`, `ble.rs`, `crates/api/examples/*.rs`

**Produces:** `Usb::open()` (first serial port with USB vendor id `0x2886`); `Uart::port(&str) -> UartBuilder` with `.baud(Baud)` (default `Baud(UART_DEFAULT_BAUD)`) and `.open()`; `Ble::open()` (name `BLE_NAME`), `Ble::named(&str) -> BleBuilder` with `.attempts(u8)` (default 3) and `.open()`. BLE reconnect is one private function `retry<T>(attempts: u8, connect: impl FnMut() -> Result<T, Error>) -> Result<T, Error>` returning `Error::Timeout { attempts }` when all fail. BLE service: Nordic UART Service UUIDs (`6e400001-…` service, `…0002` write, `…0003` notify). Examples: `mic_usb.rs`, `gyro_ble.rs`, `imu_uart.rs`, `two_links.rs`, each the matching HLD snippet as a runnable `main`.

- [ ] Tests: `api_2_retries_then_times_out` (connect fails 5 times with `attempts = 5` → called 5 times, `Timeout { attempts: 5 }`), `retry_stops_at_first_success`. Implement, `cargo build --examples -p xiao-api`, commit.
