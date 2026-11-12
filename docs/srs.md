# SRS: xiao-nrf52840-sensors

_Status: POC-ready. Holes are marked `TODO(srs)`. One section per C2
container. Requirements are numbered `<MOD>-<n>`, one testable sentence
each; step-4 tests are named after them._

## 1. Purpose

A firmware for the Seeed XIAO nRF52840 Sense plus a small Rust API that
talks to it, so a developer can switch on and read the board's motion
and sound sensors without writing embedded code.

## 2. Scope and non-goals

In scope: the two on-board sensors, in three categories: accelerometer
and gyroscope (one IMU chip, LSM6DS3TR-C) and the PDM microphone. Three
links between API and firmware: USB, BLE, UART.

Non-goals:

- No new hardware; only what is soldered on the board.
- No sensor shared by two links at once (see FW-1).
- No new drivers where an existing crate does the job: reuse, re-expose,
  flatten into builders.

## 3. System overview

See `docs/workspace.dsl` (C1, C2) and `docs/c4/`. The developer calls
the **API**; the API sends commands to the **firmware** over a link;
the firmware powers, configures and reads the sensors and streams the
readings back. Flashing is manual, with `scripts/flash.sh`.

Board and crate facts: `docs/research/sensor-crates.md`.

## 4. Modules

### 4.1 api

#### Responsibility

What the developer calls: pick sensors, configure them, read them.
Runs off the board and talks to the firmware.

#### Requirements

- **API-1**: The developer picks the link by passing an open link to
  the sensor, e.g. `Mic::on(Ble::open())`, `Imu::Gyro::on(Usb::open())`.
- **API-2**: When the BLE link drops, the API retries the connection up
  to `attempts` times (`u8`, configurable), then returns a timeout
  error.
- **API-3**: Readings arrive as a stream; the API also offers "latest
  reading" on top of it.
- **API-4**: The API reports dropped readings, detected from the
  counter in each stream message (FW-5).

#### Interfaces (intent, not signatures)

- One type per link: `Usb`, `Ble`, `Uart`, each with `open()`.
- One type per sensor: `Mic`, `Imu` (with `Gyro` and `Accel` inside
  it), each turned on with a link.

#### Data (hot structs, layout constraints)

The message format is `xiao-proto` (§4.3).

#### Open holes

- TODO(srs): where the API runs (Linux PC only, or also phone/other).
- TODO(srs): retry behaviour for USB and UART when the link drops
  (API-2 covers BLE only).
- TODO(srs): default value of `attempts`, and the wait between tries.
- TODO(srs): configuration each sensor exposes (data rate, range, gain).

### 4.2 firmware

#### Responsibility

Runs on the XIAO; powers and reads the sensors the API asks for and
sends the readings back.

#### Requirements

- **FW-1**: Each sensor is held by one link at a time; a request for it
  from another link is refused with "busy", naming the link that holds
  it (USB, BLE or UART). Different sensors may be on different links at
  the same time.
- **FW-2**: The IMU is one sensor: the accelerometer and the gyroscope
  are always held by the same link.
- **FW-3**: The UART link runs at 921600 baud by default
  (configurable).
- **FW-4**: When a sensor's stream does not fit the link's speed (e.g.
  the mic on a UART set to 115200), the firmware refuses to turn it on
  and says why.
- **FW-5**: The firmware pushes readings as a stream, at the sensor's
  own rate; each stream message carries a counter.
- **FW-6**: A sensor nobody asked for stays powered off; within the
  IMU, the accelerometer or the gyroscope can be on alone.

#### Interfaces (intent, not signatures)

- Three links in: USB, BLE, UART. Same commands on each.
- Sensors out: IMU on I2C, microphone on PDM, one power pin each.

#### Data (hot structs, layout constraints)

- Microphone: 16,000 samples/s, 2 bytes each = 32,000 bytes/s.
- The message format is `xiao-proto` (§4.3).

#### Open holes

- TODO(srs): whether BLE can carry the mic stream (32,000 bytes/s);
  measure on hardware.
- TODO(srs): what frees a sensor when its link dies without saying so
  (BLE out of range, USB unplugged).
- TODO(srs): which IMU driver crate; none is clean
  (`docs/research/sensor-crates.md`).
- TODO(srs): which HAL and BLE stack (the board carries SoftDevice
  S140 7.3.0).
- TODO(srs): how the UART baud is configured (build time or command).

### 4.3 proto

_Added by the agent on the owner's "take the wheel" (2026-11-12);
design in `docs/plans/2026-11-12-hld.md`._

#### Responsibility

The messages the API and the firmware exchange, and how they become
bytes on a link. Built into both, so they cannot disagree.

#### Requirements

- **PROTO-1**: Every command and every message decodes to exactly what
  was encoded.
- **PROTO-2**: An encoded frame is at most 244 bytes (one BLE packet),
  including a full audio block of 96 samples.
- **PROTO-3**: After garbage, a half frame or an over-long frame, the
  decoder decodes the next good frame.
- **PROTO-4**: The count-to-unit scale factors are the ones in ST's
  LSM6DS3TR-C driver.

#### Interfaces (intent, not signatures)

- `message`: `Command` (API → firmware), `Message` (firmware → API),
  and the names both use: `Sensor`, `Link`, `Rate`, ranges, `Refusal`.
- `frame`: `encode` one value into a buffer; a `Decoder` fed one byte
  at a time.

#### Data (hot structs, layout constraints)

Wire format: postcard, COBS-framed, a zero byte ends each frame. Audio
samples travel as raw little-endian bytes. Layout in memory is not
pinned: only the wire bytes cross the boundary.

#### Open holes

None.

## 5. Cross-cutting

### 5.1 Errors

Every refusal says why: "busy" names the holding link (FW-1), "too
slow" names the link speed (FW-4), timeout after `attempts` (API-2).

### 5.2 Performance

TODO(srs).

### 5.3 Safety and `unsafe`

No `unsafe` by agents (`CLAUDE.md`). TODO(srs): the IMU driver and BLE
stack choices may force some; decide per site.

## 6. Glossary

- **Link**: the connection between API and firmware: USB, BLE or UART.
- **Sensor**: the IMU (accelerometer + gyroscope) or the microphone.
- **Stream**: readings pushed by the firmware without being asked each
  time.
