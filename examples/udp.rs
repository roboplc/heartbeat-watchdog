use std::{thread, time::Duration};

use heartbeat_watchdog::{
    io::udp::{UdpHeart, UdpIo},
    Heart, Range, Watchdog, WatchdogConfig,
};
use rtsc::time::interval;

fn main() {
    let heart = UdpHeart::create("127.0.0.1:9999").unwrap();
    let watchdog: Watchdog<UdpIo, _> = Watchdog::create(
        WatchdogConfig::new(Duration::from_millis(100), "127.0.0.1:9999")
            .with_range(Range::Window(Duration::from_millis(10))),
    )
    .unwrap();
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
                heart.beat().unwrap();
            }
        }
    }
}
