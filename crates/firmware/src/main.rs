//! The firmware binary: starts the links, then serves commands.
//!
//! Depends on: every module of `xiao_firmware`.
#![cfg_attr(target_os = "none", no_std, no_main)]

#[cfg(target_os = "none")]
mod board;
#[cfg(target_os = "none")]
mod link;
#[cfg(target_os = "none")]
mod mic;

#[cfg(target_os = "none")]
use {nrf_mpsl as _, panic_halt as _, xiao_firmware as _};

#[cfg(target_os = "none")]
#[embassy_executor::main]
async fn main(spawner: embassy_executor::Spawner) {
    let peripherals = embassy_nrf::init(embassy_nrf::config::Config::default());
    // Skeleton: nothing is started yet.
    let _ = (spawner, peripherals);
}

/// Off the board there is nothing to run; this keeps `cargo test
/// --workspace` building on the host.
#[cfg(not(target_os = "none"))]
fn main() {}
