use std::{thread, time::Duration};

use heartbeat_watchdog::{
    io::udp::{UdpHeart, UdpIo},
    Heart, Range, Watchdog, WatchdogConfig,
};
use rtsc::time::interval;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let heart = UdpHeart::create("127.0.0.1:9999")?;
    let watchdog_config = WatchdogConfig::new(Duration::from_millis(100))
        .with_range(Range::Window(Duration::from_millis(10)));
    let watchdog_io = UdpIo::create("127.0.0.1:9999", watchdog_config.io_timeout())?;
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
        heart.beat()?;
        if i > 0 && i % 100 == 0 {
            if i % 200 == 0 {
                println!("Timing out");
                thread::sleep(Duration::from_millis(200));
            } else {
                println!("Breaking the window");
                heart.beat()?;
            }
        }
    }
    Ok(())
}
