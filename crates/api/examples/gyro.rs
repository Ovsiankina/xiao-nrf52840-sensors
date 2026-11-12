//! The gyroscope alone, over USB: print a reading every half second.
//!
//! At rest all three axes read close to 0; spin the board and the axis
//! you spin it around moves.
//!
//! Run with `cargo run -p xiao-api --example gyro`.

use std::time::Duration;

use xiao_api::{Error, Gyro, Rotation, Usb};

fn main() -> Result<(), Error> {
    let usb = Usb::open()?;
    let gyro = Gyro::on(&usb)?;

    loop {
        let Rotation { x, y, z } = gyro.latest()?;
        println!(
            "x {:+7.1} dps   y {:+7.1} dps   z {:+7.1} dps",
            x.0, y.0, z.0
        );
        std::thread::sleep(Duration::from_millis(500));
    }
}
