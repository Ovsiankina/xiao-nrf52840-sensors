//! Microphone over USB: print the loudest sample of each block.
//!
//! Run with `cargo run -p xiao-api --example mic_usb`.

use xiao_api::{Error, Mic, Usb};

fn main() -> Result<(), Error> {
    let usb = Usb::open()?;
    let mic = Mic::on(&usb)?;

    for block in &mic {
        let block = block?;
        let peak = block.samples().iter().map(|s| s.unsigned_abs()).max();
        println!("{:?} peak {peak:?}, lost {:?}", block.rate(), mic.dropped());
    }
    Ok(())
}
