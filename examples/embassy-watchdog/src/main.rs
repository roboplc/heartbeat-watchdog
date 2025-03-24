//! Watchdog example for STM32F103
//!
//! This example demonstrates the usage of the `heartbeat_watchdog` with an external heartbeat
//! signal, where the board acts as a watchdog.
//!
//! Pins:
//!
//! - PB12: Output LED, blinks every 1s to indicate the board is alive
//! - PB13: Fault LED, lights up when the watchdog detects a fault
//! - PB14: Input, the external heartbeat signal
#![no_std]
#![no_main]

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_time::{Duration, Instant, Timer};
use heartbeat_watchdog::{io::WatchdogIoAsync, WatchdogAsync, WatchdogConfig};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

struct WatchB14 {
    input: Input<'static>,
    timeout: Duration,
}

impl WatchdogIoAsync for WatchB14 {
    async fn get(
        &self,
        expected: heartbeat_watchdog::Edge,
    ) -> heartbeat_watchdog::Result<heartbeat_watchdog::Edge> {
        let now = Instant::now();
        loop {
            if now.elapsed() > self.timeout {
                break;
            }
            let edge: heartbeat_watchdog::Edge = bool::from(self.input.get_level()).into();
            if edge == expected {
                return Ok(edge);
            }
            embassy_time::Timer::after(Duration::from_micros(100)).await;
        }
        Err(heartbeat_watchdog::Error::Timeout)
    }

    async fn clear(&self) -> heartbeat_watchdog::Result<()> {
        Ok(())
    }
}

#[embassy_executor::task]
async fn run_watchdog(watchdog: WatchdogAsync<WatchB14>) {
    watchdog.run().await.unwrap();
}

static WATCHDOG_CHANNEL: StaticCell<heartbeat_watchdog::EmbassyStateChannel> = StaticCell::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());
    let mut op_led = Output::new(p.PB12, Level::Low, Speed::Low);
    let mut fault_led = Output::new(p.PB13, Level::Low, Speed::Low);
    fault_led.set_high();
    info!("Watchdog started");
    let w_input = Input::new(p.PB14, Pull::Down);
    let watchdog_config = WatchdogConfig::new(Duration::from_millis(10).into())
        .with_range(heartbeat_watchdog::Range::Window(
            Duration::from_millis(1).into(),
        ))
        .with_warmup(Duration::from_secs(2).into())
        .with_min_beats(200);
    let watchdog_io = WatchB14 {
        input: w_input,
        timeout: watchdog_config.io_timeout().try_into().unwrap(),
    };
    let watchdog_channel = WATCHDOG_CHANNEL.init(heartbeat_watchdog::EmbassyStateChannel::new());
    let mut watchdog = WatchdogAsync::new(watchdog_config, watchdog_io);
    watchdog.set_state_tx(watchdog_channel.sender());
    spawner.spawn(run_watchdog(watchdog)).unwrap();
    let mut n: usize = 0;
    loop {
        if let Ok(event) = watchdog_channel.try_receive() {
            match event {
                heartbeat_watchdog::StateEvent::Fault(kind) => {
                    warn!("Watchdog state FAULT: {:?}", kind);
                    fault_led.set_high();
                }
                heartbeat_watchdog::StateEvent::Ok => {
                    info!("Watchdog state OK");
                    fault_led.set_low();
                }
            }
        }
        Timer::after_millis(1).await;
        n += 1;
        if n % 1000 == 0 {
            op_led.toggle();
        }
    }
}
