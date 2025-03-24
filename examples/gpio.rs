use std::{thread, time::Duration};

use heartbeat_watchdog::{
    io::gpio::{Gpio, GpioConfig, GpioHeart},
    Heart, Range, Watchdog, WatchdogConfig,
};
use rtsc::time::interval;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let heart = GpioHeart::create("/dev/gpiochip0", 17).unwrap();
    let watchdog_config = WatchdogConfig::new(Duration::from_millis(100))
        .with_range(Range::Window(Duration::from_millis(10)));
    let watchdog_io = Gpio::create(
        &GpioConfig::new("/dev/gpiochip0", 27, Duration::from_millis(2)),
        watchdog_config.io_timeout(),
    )?;
    let watchdog = Watchdog::new(watchdog_config, watchdog_io);
    let state_rx = watchdog.state_rx();
    thread::spawn(move || {
        for e in state_rx {
            println!("{:?}", e);
        }
    });
    thread::spawn(move || {
        watchdog.run().unwrap();
    });
    for (i, _) in interval(Duration::from_millis(100)).enumerate() {
        heart.beat().unwrap();
        if i > 0 && i % 10 == 0 {
            if i % 20 == 0 {
                println!("Timing out");
                thread::sleep(Duration::from_millis(200));
            } else {
                println!("Breaking the window");
                thread::sleep(Duration::from_millis(50));
                heart.beat().unwrap();
            }
        }
    }
    Ok(())
}
