<h2>
  Heartbeat watchdog
  <a href="https://crates.io/crates/heartbeat-watchdog"><img alt="crates.io page" src="https://img.shields.io/crates/v/heartbeat-watchdog.svg"></img></a>
  <a href="https://docs.rs/heartbeat-watchdog"><img alt="docs.rs page" src="https://docs.rs/heartbeat-watchdog/badge.svg"></img></a>
</h2>


A versatile watchdog and heartbeat traits for various monitoring purposes in
mission-critical systems (processes, single threads etc).

The crate is a part of the [RoboPLC](https://www.roboplc.com) project and works
on Linux only. No other platforms support is planned, except bare-metal.

## Communication

The crate provides out-of-the-box:

- `UDP` socket heartbeat/watchdog
- `GPIO` heartbeat/watchdog (Linux only)

More communication methods can be added by implementing `io::WatchdogIo` and `Heart` traits.

## Error detection

The following heartbeat errors are detected:

- `Timeout` - no heartbeat received within the specified time
- `Window` - heartbeat has been received out of the time window
- `OutOfOrder` - heartbeat has been received out of order (e.g. for TCP/IP communication)

## TODO

- `nostd` support
