//! The microphone, over USB: print "received" when you shout into it.
//!
//! One shout prints one line: after a loud block, the mic must stay
//! quiet for `REARM_BLOCKS` before it can print again.
//!
//! Run with `cargo run -p xiao-api --example mic`.

use xiao_api::{Error, Mic, Usb};

// DECISION: a quarter of full scale (32 767), picked without hardware;
// lower it if a light shout does not trigger, raise it if talking does.
const LOUD: u16 = 8_000;

/// Quiet blocks needed before the next shout counts: 50 × 6 ms = 0.3 s.
const REARM_BLOCKS: u32 = 50;

fn main() -> Result<(), Error> {
    let usb = Usb::open()?;
    let mic = Mic::on(&usb)?;

    let mut quiet_blocks = REARM_BLOCKS;
    for block in &mic {
        let block = block?;
        let peak = block
            .samples()
            .iter()
            .map(|s| s.unsigned_abs())
            .max()
            .unwrap_or(0);
        if peak >= LOUD {
            if quiet_blocks >= REARM_BLOCKS {
                println!("received");
            }
            quiet_blocks = 0;
        } else {
            quiet_blocks = quiet_blocks.saturating_add(1);
        }
    }
    Ok(())
}
