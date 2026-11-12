//! The accelerometer alone, over USB: print a reading every half second.
//!
//! Lay the board flat and z reads about +1 g; tilt it and the 1 g moves
//! to another axis.
//!
//! Run with `cargo run -p xiao-api --example accel`.

use std::time::Duration;

use xiao_api::{Accel, Acceleration, Error, Usb};

fn main() -> Result<(), Error> {
    let usb = Usb::open()?;
    let accel = Accel::on(&usb)?;

    loop {
        let Acceleration { x, y, z } = accel.latest()?;
        println!("x {:+.2} g   y {:+.2} g   z {:+.2} g", x.0, y.0, z.0);
        std::thread::sleep(Duration::from_millis(500));
    }
}
