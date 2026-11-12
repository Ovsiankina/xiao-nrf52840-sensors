//! Gyroscope over BLE, configured; the accelerometer stays off.

use xiao_api::{Ble, Error, Gyro, GyroRange, Rate, Rotation};

fn main() -> Result<(), Error> {
    let ble = Ble::attempts(5).open()?; // default name "XIAO-Sense"
    let gyro = Gyro::rate(Rate::Hz104).range(GyroRange::Dps500).on(&ble)?;

    let Rotation { x, y, z } = gyro.latest()?;
    println!("turning at {x:?} {y:?} {z:?}");
    Ok(())
}
