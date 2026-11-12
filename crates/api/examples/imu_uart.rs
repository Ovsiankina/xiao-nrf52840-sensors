//! Both halves of the IMU over the UART pins.

use xiao_api::{Baud, Error, Imu, Uart};

fn main() -> Result<(), Error> {
    let uart = Uart::port("/dev/ttyUSB0").baud(Baud(921_600)).open()?;
    let imu = Imu::on(&uart)?;

    for motion in &imu {
        let motion = motion?;
        println!("{:?} {:?}", motion.accel, motion.gyro);
    }
    Ok(())
}
