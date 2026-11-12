# proto Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One `no_std` crate holding every message the API and the firmware exchange, and the framing that puts them on a byte link.

**Architecture:** `message` is plain serde data. `frame` encodes one message as postcard + COBS (zero byte ends a frame) and decodes a byte stream one byte at a time, resyncing on the next zero after garbage.

**Tech Stack:** `serde`, `postcard` 1.1, `heapless` 0.9. No `alloc`.

**Spec:** `docs/srs.md` (FW-1, FW-4, FW-5 name what the messages must carry), `docs/plans/2026-11-12-hld.md`.

## Global Constraints

- `no_std`, no `alloc`, no `unsafe`, no new dependency.
- Commit message starts with the task ID (`proto-01: ...`).
- `docs/preferences.md` before any code.
- Tests on the host: `cargo test -p xiao-proto`.

## Review Focus

- Garbage or a half frame before a good frame: the good frame still decodes (proto-02).
- A frame longer than `MAX_FRAME`: decoder reports `TooLong`, then recovers on the next zero (proto-02).
- Audio samples must not grow on the wire: they travel as raw little-endian bytes, not as varints (proto-01).

---

### Task proto-01: messages

**Files:** `crates/proto/src/message.rs`

**Produces** (all `Serialize + Deserialize + Debug + Clone + PartialEq`; the small ones also `Copy + Eq`):

```rust
pub enum Sensor { Mic, Imu }
pub enum Link { Usb, Ble, Uart }
pub enum Rate { Hz12_5, Hz26, Hz52, Hz104, Hz208, Hz416, Hz833 }   // impl: fn hertz(self) -> f32
pub enum AccelRange { G2, G4, G8, G16 }        // fn g_per_count(self) -> f32: 0.000061, 0.000122, 0.000244, 0.000488
pub enum GyroRange { Dps125, Dps250, Dps500, Dps1000, Dps2000 }  // fn dps_per_count(self) -> f32: 0.004375, 0.00875, 0.0175, 0.035, 0.07
pub struct AccelConfig { pub rate: Rate, pub range: AccelRange }   // Default: Hz104, G2
pub struct GyroConfig { pub rate: Rate, pub range: GyroRange }     // Default: Hz104, Dps250
pub struct ImuConfig { pub accel: Option<AccelConfig>, pub gyro: Option<GyroConfig> }
pub enum Command { MicOn, MicOff, ImuOn(ImuConfig), ImuOff }
pub enum Refusal { Busy { held_by: Link }, TooSlow { needs: u32, link_has: u32 } }  // bytes per second
pub struct AudioBlock { pub counter: u32, pub pcm: heapless::Vec<u8, AUDIO_BLOCK_BYTES> }  // i16 little-endian
pub struct ImuReading { pub counter: u32, pub accel: Option<[i16; 3]>, pub gyro: Option<[i16; 3]> }  // raw counts
pub enum Message { On(Sensor), Off(Sensor), Refused { sensor: Sensor, reason: Refusal }, Audio(AudioBlock), Imu(ImuReading) }
pub const MIC_SAMPLE_RATE_HZ: u32 = 16_000;
pub const AUDIO_BLOCK_SAMPLES: usize = 96;
pub const AUDIO_BLOCK_BYTES: usize = 192;
pub const BLE_NAME: &str = "XIAO-Sense";
pub const UART_DEFAULT_BAUD: u32 = 921_600;
```

- [x] Tests: `scale_factors_match_st_driver` (the ten values above), `defaults_are_104_hz`, `audio_block_holds_96_samples`.
- [x] Implement, `cargo test -p xiao-proto`, commit.

### Task proto-02: framing

**Files:** `crates/proto/src/frame.rs`

**Produces:**

```rust
pub const MAX_FRAME: usize = 256;
pub enum FrameError { TooLong, Malformed }
pub fn encode<'b, T: Serialize>(value: &T, buf: &'b mut [u8; MAX_FRAME]) -> Result<&'b [u8], FrameError>;  // ends with 0x00
pub struct Decoder { /* private: [u8; MAX_FRAME], len, overflowed */ }
impl Decoder {
    pub const fn new() -> Self;
    /// Feed one byte. `None` until a frame ends; then the decoded value or why it failed.
    pub fn feed<T: DeserializeOwned>(&mut self, byte: u8) -> Option<Result<T, FrameError>>;
}
```

- [x] Tests: `round_trip_every_command`, `round_trip_full_audio_block` (encoded length ≤ 244, the BLE payload), `garbage_then_frame_decodes`, `too_long_then_recovers`, `two_frames_back_to_back`.
- [x] Implement with `postcard::to_slice_cobs` / `postcard::from_bytes_cobs`, `cargo test -p xiao-proto`, commit.
