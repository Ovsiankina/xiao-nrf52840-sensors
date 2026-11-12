//! Two sensors on two links at once, and what "busy" looks like.
//!
//! Run with `cargo run -p xiao-api --example two_links`.

use xiao_api::{Accel, Ble, Error, Mic, Usb};

fn main() -> Result<(), Error> {
    let usb = Usb::open()?;
    let ble = Ble::open()?;

    let _mic = Mic::on(&ble)?;
    let accel = Accel::on(&usb)?;
    println!("{:?}", accel.latest()?);

    match Mic::on(&usb) {
        Err(Error::Busy { held_by }) => println!("the mic is held by {held_by:?}"),
        other => println!("unexpected: {:?}", other.map(|_| ())),
    }
    Ok(())
}
