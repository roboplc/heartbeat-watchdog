#![deny(missing_docs)]
#![ doc = include_str!( concat!( env!( "CARGO_MANIFEST_DIR" ), "/", "README.md" ) ) ]
use core::{fmt, ops, time::Duration};
use std::{sync::Arc, thread, time::Instant};

use io::WatchdogIo;
use portable_atomic::{AtomicBool, Ordering};
use rtsc::policy_channel;

/// Watchdog I/O
pub mod io;

/// Errors
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// IO error (std)
    #[error("IO error: {0}")]
    Io(std::io::Error),
    /// Timeout
    #[error("Timed out")]
    Timeout,
    /// All other errors
    #[error("Failed: {0}")]
    Failed(String),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => Error::Timeout,
            _ => Error::Io(e),
        }
    }
}

impl Error {
    /// Create a new failed error
    pub fn failed<T: fmt::Display>(msg: T) -> Self {
        Error::Failed(msg.to_string())
    }
}

/// Result type
pub type Result<T> = std::result::Result<T, Error>;

type RawMutex = rtsc::pi::RawMutex;
type Condvar = rtsc::pi::Condvar;

/// State event
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum StateEvent {
    /// Watchdog switched to Fault state
    Fault(FaultKind),
    /// Watchdog switched to OK state
    Ok,
}

impl rtsc::data_policy::DataDeliveryPolicy for StateEvent {
    fn delivery_policy(&self) -> rtsc::data_policy::DeliveryPolicy {
        rtsc::data_policy::DeliveryPolicy::Latest
    }
}

impl From<StateEvent> for State {
    fn from(e: StateEvent) -> Self {
        match e {
            StateEvent::Ok => State::Ok,
            StateEvent::Fault(_) => State::Fault,
        }
    }
}

/// Watchdog state
#[repr(u8)]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum State {
    /// Fault state
    Fault = 0,
    /// OK state
    Ok = 1,
}

impl From<u8> for State {
    fn from(b: u8) -> Self {
        match b {
            0 => State::Fault,
            _ => State::Ok,
        }
    }
}

impl From<bool> for State {
    fn from(b: bool) -> Self {
        if b {
            State::Ok
        } else {
            State::Fault
        }
    }
}

impl From<State> for bool {
    fn from(s: State) -> bool {
        match s {
            State::Fault => false,
            State::Ok => true,
        }
    }
}

/// Edge
#[repr(u8)]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Edge {
    /// Rising edge
    Rising = b'+',
    /// Falling edge
    Falling = b'.',
}

impl ops::Not for Edge {
    type Output = Self;
    fn not(self) -> Self {
        match self {
            Edge::Rising => Edge::Falling,
            Edge::Falling => Edge::Rising,
        }
    }
}

impl From<u8> for Edge {
    fn from(b: u8) -> Self {
        match b {
            1 | b'+' => Edge::Rising,
            _ => Edge::Falling,
        }
    }
}

impl From<bool> for Edge {
    fn from(b: bool) -> Self {
        if b {
            Edge::Rising
        } else {
            Edge::Falling
        }
    }
}

impl From<Edge> for bool {
    fn from(e: Edge) -> bool {
        match e {
            Edge::Rising => true,
            Edge::Falling => false,
        }
    }
}

/// Heartbeat range
#[derive(Debug, Clone)]
pub enum Range {
    /// Upper bound (timeout)
    Timeout(Duration),
    /// Time window
    Window(Duration),
}

/// Fault state kind
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FaultKind {
    /// Initial state (watchdog is always started in "Fault")
    Initial,
    /// No heartbeat received in time
    Timeout,
    /// Heartbeat not in the time window
    Window,
    /// Out-of-order edge (e.g. for TCP/IP packets)
    OutOfOrder,
}

impl Range {
    fn timeout(&self) -> Duration {
        match self {
            Range::Timeout(d) | Range::Window(d) => *d,
        }
    }
}

/// Watchdog configuration
#[derive(Debug, Clone)]
pub struct WatchdogConfig<IC> {
    interval: Duration,
    range: Range,
    warmup: Duration,
    min_beats: u32,
    io_config: IC,
}

impl<IC> WatchdogConfig<IC> {
    /// Create a new watchdog configuration
    pub fn new(interval: Duration, io_config: IC) -> Self {
        Self {
            interval,
            range: Range::Timeout(interval + interval / 10),
            warmup: interval * 2,
            min_beats: 2,
            io_config,
        }
    }
    /// Set the range
    pub fn with_range(mut self, range: Range) -> Self {
        self.range = range;
        self
    }
    /// Set the warmup time (no heartbeat checked after startup/fault)
    pub fn with_warmup(mut self, warmup: Duration) -> Self {
        self.warmup = warmup;
        self
    }
    /// Set the minimum number of valid beats before switching to OK state
    pub fn with_min_beats(mut self, min_beats: u32) -> Self {
        self.min_beats = min_beats;
        self
    }
}

/// Watchdog
pub struct Watchdog<I: WatchdogIo<IC>, IC> {
    inner: Arc<WatchDogInner<I, IC>>,
}

impl<I: WatchdogIo<IC>, IC> Clone for Watchdog<I, IC> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

struct WatchDogInner<I: WatchdogIo<IC>, IC> {
    io: I,
    state: AtomicBool,
    config: WatchdogConfig<IC>,
    state_tx: policy_channel::Sender<StateEvent, RawMutex, Condvar>,
    state_rx: policy_channel::Receiver<StateEvent, RawMutex, Condvar>,
}

impl<I: WatchdogIo<IC>, IC> Watchdog<I, IC> {
    /// Create a new watchdog
    pub fn create(config: WatchdogConfig<IC>) -> Result<Self> {
        let (state_tx, state_rx) = rtsc::policy_channel::bounded(1);
        Ok(Self {
            inner: Arc::new(WatchDogInner {
                io: I::create(&config)?,
                state: AtomicBool::new(State::Fault.into()),
                config,
                state_tx,
                state_rx,
            }),
        })
    }
    /// Get the current state
    pub fn state(&self) -> State {
        self.inner.state.load(Ordering::Relaxed).into()
    }
    /// Get the state receiver channel
    pub fn state_rx(&self) -> policy_channel::Receiver<StateEvent, RawMutex, Condvar> {
        self.inner.state_rx.clone()
    }
    /// Run the watchdog
    pub fn run(&self) -> Result<()> {
        self.set_fault(FaultKind::Initial)?;
        let mut packets = 0;
        let mut next = Edge::Rising;
        let mut last_packet = Instant::now();
        loop {
            match self.inner.io.get(next) {
                Ok(edge) => {
                    if let Range::Window(v) = self.inner.config.range {
                        if last_packet.elapsed() < self.inner.config.interval - v {
                            packets = 0;
                            self.set_fault(FaultKind::Window)?;
                            last_packet = Instant::now();
                            continue;
                        }
                        last_packet = Instant::now();
                    }
                    if edge == next {
                        next = !next;
                        if self.state() == State::Fault {
                            packets += 1;
                            if packets >= self.inner.config.min_beats * 2 {
                                self.set_ok()?;
                            }
                        }
                        continue;
                    }
                    if self.state() == State::Ok {
                        packets = 0;
                        self.set_fault(FaultKind::OutOfOrder)?;
                        last_packet = Instant::now();
                    }
                }
                Err(Error::Timeout) => {
                    packets = 0;
                    self.set_fault(FaultKind::Timeout)?;
                    last_packet = Instant::now();
                }
                Err(e) => return Err(e),
            }
        }
    }
    fn set_ok(&self) -> Result<()> {
        if self.state() == State::Ok {
            return Ok(());
        }
        self.inner.state.store(true, Ordering::Relaxed);
        self.inner
            .state_tx
            .send(StateEvent::Ok)
            .map_err(Error::failed)?;
        Ok(())
    }
    fn set_fault(&self, kind: FaultKind) -> Result<()> {
        if self.state() == State::Fault && kind != FaultKind::Initial {
            return Ok(());
        }
        self.inner.state.store(false, Ordering::Relaxed);
        self.inner
            .state_tx
            .send(StateEvent::Fault(kind))
            .map_err(Error::failed)?;
        self.warmup()?;
        Ok(())
    }
    fn warmup(&self) -> Result<()> {
        thread::sleep(self.inner.config.warmup);
        self.inner.io.clear()?;
        Ok(())
    }
}

/// Heartbeat client trait
pub trait Heart {
    /// Send the current edge
    fn beat(&self) -> Result<()>;
}
