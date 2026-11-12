//! Gyroscope over BLE, configured; the accelerometer stays off.
//!
//! Run with `cargo run -p xiao-api --example gyro_ble`.

use xiao_api::{Ble, Error, Gyro, GyroRange, Rate, Rotation};

fn main() -> Result<(), Error> {
    // The default name, "XIAO-Sense"; `Ble::named("desk-xiao")` finds another board.
    let ble = Ble::attempts(5).open()?;
    let gyro = Gyro::rate(Rate::Hz104).range(GyroRange::Dps500).on(&ble)?;

    let Rotation { x, y, z } = gyro.latest()?;
    println!("turning at {x:?} {y:?} {z:?}");
    Ok(())
}
