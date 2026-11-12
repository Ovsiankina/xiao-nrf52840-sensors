# Deferred

_Written by agents when a task hits a contract break (see
`CLAUDE.md`), and by me at step 9. Each entry re-enters the cycle as
a change (`PROCEDURE.md`). Three open entries on one module = `needs
contract rework`.
Never delete a row; set status `done` with the date._

| date | module | step | what was needed | why a contract break | smallest unblocking change | status |
|---|---|---|---|---|---|---|
| 2026-11-12 | firmware | 4 | BLE bytes-per-second figure for FW-4 | SRS hole: "whether BLE can carry the mic stream" | measure on the board, replace the 60 000 placeholder in `sensors::link_speed` | open |
| 2026-11-12 | api | 4 | default `attempts` and wait between tries; retry for USB and UART | SRS holes under 4.1 | owner states the values in `docs/srs.md`; code uses 3 attempts, 1 s, BLE only | open |

- [question] api: sensors borrow their link (`Mic<'l>`), so a sensor cannot sit in a struct next to its link or be returned with it from a setup function. Changing it (sensor holds a shared handle, no lifetime) is a contract change. (step-6 review, 2026-11-12)
- [question] api: `Iterator` exists only on `&Mic` / `&Gyro` / `&Accel` / `&Imu`, so `gyro.next()` does not compile (`(&gyro).next()` does). An inherent `next()` would be a new pub item. (step-6 review, 2026-11-12)
