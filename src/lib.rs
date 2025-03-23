#![cfg_attr(not(feature = "std"), no_std)]
#![deny(missing_docs)]
#![ doc = include_str!( concat!( env!( "CARGO_MANIFEST_DIR" ), "/", "README.md" ) ) ]
use core::{future::Future, ops, time::Duration};
#[cfg(feature = "embassy")]
use embassy_time::Instant;
#[cfg(feature = "std")]
use std::{sync::Arc, time::Instant};

use io::{WatchdogIo, WatchdogIoAsync};
use portable_atomic::{AtomicBool, Ordering};
#[cfg(feature = "std")]
use rtsc::{policy_channel, policy_channel_async};

/// Watchdog I/O
pub mod io;

/// Errors
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[cfg(feature = "std")]
    /// IO error (std)
    #[error("IO error: {0}")]
    Io(std::io::Error),
    /// Timeout
    #[error("Timed out")]
    Timeout,
    /// All other errors
    #[cfg(feature = "std")]
    #[error("Failed: {0}")]
    Failed(String),
    /// All other errors (no std)
    #[cfg(not(feature = "std"))]
    #[error("Failed")]
    Failed,
}

#[cfg(feature = "std")]
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => Error::Timeout,
            _ => Error::Io(e),
        }
    }
}

impl Error {
    #[cfg(feature = "std")]
    /// Create a new failed error
    pub fn failed<T: core::fmt::Display>(msg: T) -> Self {
        Error::Failed(msg.to_string())
    }
    #[cfg(not(feature = "std"))]
    /// Create a new failed error
    pub fn failed() -> Self {
        Error::Failed
    }
}

/// Result type
pub type Result<T> = core::result::Result<T, Error>;

#[cfg(feature = "std")]
type RawMutex = rtsc::pi::RawMutex;
#[cfg(feature = "std")]
type Condvar = rtsc::pi::Condvar;

/// State event
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum StateEvent {
    /// Watchdog switched to Fault state
    Fault(FaultKind),
    /// Watchdog switched to OK state
    Ok,
}

#[cfg(feature = "std")]
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
    /// Get the relative I/O timeout duration
    #[allow(dead_code)]
    pub fn timeout(&self) -> Duration {
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
    /// Get the interval
    pub fn interval(&self) -> Duration {
        self.interval
    }
    /// Get the range
    pub fn range(&self) -> &Range {
        &self.range
    }
    /// Get the warmup time
    pub fn warmup(&self) -> Duration {
        self.warmup
    }
    /// Get the minimum number of valid beats
    pub fn min_beats(&self) -> u32 {
        self.min_beats
    }
    /// Get the I/O configuration
    pub fn io_config(&self) -> &IC {
        &self.io_config
    }
}

/// Watchdog
pub struct Watchdog<I: WatchdogIo<IC>, IC> {
    #[cfg(feature = "std")]
    inner: Arc<WatchDogInner<I, IC>>,
    #[cfg(not(feature = "std"))]
    inner: WatchDogInner<I, IC>,
}

#[cfg(feature = "std")]
impl<I: WatchdogIo<IC>, IC> Clone for Watchdog<I, IC> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

struct WatchDogProcessor<'a, IC> {
    packets: u32,
    next: Edge,
    last_packet: Instant,
    config: &'a WatchdogConfig<IC>,
}

impl<'a, IC> WatchDogProcessor<'a, IC> {
    fn new(config: &'a WatchdogConfig<IC>) -> Self {
        Self {
            packets: 0,
            next: Edge::Rising,
            last_packet: Instant::now(),
            config,
        }
    }
    fn process(&mut self, res: Result<Edge>, current_state: State) -> Result<Option<StateEvent>> {
        #[cfg(feature = "std")]
        let elapsed_ms = u64::try_from(self.last_packet.elapsed().as_micros()).unwrap();
        #[cfg(feature = "embassy")]
        let elapsed_ms = self.last_packet.elapsed().as_micros();
        self.last_packet = Instant::now();
        match res {
            Ok(edge) => {
                if let Range::Window(v) = self.config.range {
                    if elapsed_ms
                        < u64::try_from(self.config.interval.as_micros() - v.as_micros()).unwrap()
                    {
                        self.packets = 0;
                        return Ok(Some(StateEvent::Fault(FaultKind::Window)));
                    }
                }
                if edge == self.next {
                    self.next = !self.next;
                    if current_state == State::Fault {
                        self.packets += 1;
                        if self.packets >= self.config.min_beats * 2 {
                            return Ok(Some(StateEvent::Ok));
                        }
                    }
                    return Ok(None);
                }
                if self.packets > 1 {
                    self.packets = 0;
                    return Ok(Some(StateEvent::Fault(FaultKind::OutOfOrder)));
                }
                Ok(None)
            }
            Err(Error::Timeout) => {
                self.packets = 0;
                Ok(Some(StateEvent::Fault(FaultKind::Timeout)))
            }
            Err(e) => Err(e),
        }
    }
}

struct WatchDogInner<I: WatchdogIo<IC>, IC> {
    io: I,
    state: AtomicBool,
    config: WatchdogConfig<IC>,
    #[cfg(feature = "std")]
    state_tx: policy_channel::Sender<StateEvent, RawMutex, Condvar>,
    #[cfg(feature = "std")]
    state_rx: policy_channel::Receiver<StateEvent, RawMutex, Condvar>,
}

impl<I: WatchdogIo<IC>, IC> Watchdog<I, IC> {
    /// Create a new watchdog
    pub fn create(config: WatchdogConfig<IC>) -> Result<Self> {
        #[cfg(feature = "std")]
        let (state_tx, state_rx) = rtsc::policy_channel::bounded(1);
        Ok(Self {
            inner: WatchDogInner {
                io: I::create(&config)?,
                state: AtomicBool::new(State::Fault.into()),
                config,
                #[cfg(feature = "std")]
                state_tx,
                #[cfg(feature = "std")]
                state_rx,
            }
            .into(),
        })
    }
    /// Get the current state
    pub fn state(&self) -> State {
        self.inner.state.load(Ordering::Relaxed).into()
    }
    /// Get a reference to the state atomic
    pub fn state_ref(&self) -> &AtomicBool {
        &self.inner.state
    }
    /// Get the state receiver channel
    #[cfg(feature = "std")]
    pub fn state_rx(&self) -> policy_channel::Receiver<StateEvent, RawMutex, Condvar> {
        self.inner.state_rx.clone()
    }
    /// Run the watchdog
    pub fn run(&self) -> Result<()> {
        self.set_fault(FaultKind::Initial)?;
        let mut p = WatchDogProcessor::new(&self.inner.config);
        loop {
            match p.process(self.inner.io.get(p.next), self.state()) {
                Ok(Some(event)) => match event {
                    StateEvent::Ok => self.set_ok()?,
                    StateEvent::Fault(kind) => self.set_fault(kind)?,
                },
                Ok(None) => (),
                Err(e) => return Err(e),
            }
        }
    }
    fn set_ok(&self) -> Result<()> {
        if self.state() == State::Ok {
            return Ok(());
        }
        self.inner.state.store(true, Ordering::Relaxed);
        #[cfg(feature = "std")]
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
        #[cfg(feature = "std")]
        self.inner
            .state_tx
            .send(StateEvent::Fault(kind))
            .map_err(Error::failed)?;
        self.warmup()?;
        Ok(())
    }
    fn warmup(&self) -> Result<()> {
        #[cfg(feature = "std")]
        std::thread::sleep(self.inner.config.warmup);
        self.inner.io.clear()?;
        Ok(())
    }
}

/// Watchdog
pub struct WatchdogAsync<I: WatchdogIoAsync<IC>, IC> {
    #[cfg(feature = "std")]
    inner: Arc<WatchDogInnerAsync<I, IC>>,
    #[cfg(not(feature = "std"))]
    inner: WatchDogInnerAsync<I, IC>,
}

struct WatchDogInnerAsync<I: WatchdogIoAsync<IC>, IC> {
    io: I,
    state: AtomicBool,
    config: WatchdogConfig<IC>,
    #[cfg(feature = "std")]
    state_tx: policy_channel_async::Sender<StateEvent>,
    #[cfg(feature = "std")]
    state_rx: policy_channel_async::Receiver<StateEvent>,
}

impl<I: WatchdogIoAsync<IC>, IC> WatchdogAsync<I, IC> {
    /// Create a new watchdog
    pub async fn create(config: WatchdogConfig<IC>) -> Result<Self> {
        #[cfg(feature = "std")]
        let (state_tx, state_rx) = rtsc::policy_channel_async::bounded(1);
        Ok(Self {
            inner: WatchDogInnerAsync {
                io: I::create(&config).await?,
                state: AtomicBool::new(State::Fault.into()),
                config,
                #[cfg(feature = "std")]
                state_tx,
                #[cfg(feature = "std")]
                state_rx,
            }
            .into(),
        })
    }
    /// Get the current state
    pub fn state(&self) -> State {
        self.inner.state.load(Ordering::Relaxed).into()
    }
    #[cfg(feature = "std")]
    /// Get the state receiver channel
    pub fn state_rx(&self) -> policy_channel_async::Receiver<StateEvent> {
        self.inner.state_rx.clone()
    }
    /// Get a reference to the state atomic
    pub fn state_ref(&self) -> &AtomicBool {
        &self.inner.state
    }
    /// Run the watchdog
    pub async fn run(&self) -> Result<()> {
        self.set_fault(FaultKind::Initial).await?;
        let mut p = WatchDogProcessor::new(&self.inner.config);
        loop {
            match p.process(self.inner.io.get(p.next).await, self.state()) {
                Ok(Some(event)) => match event {
                    StateEvent::Ok => self.set_ok().await?,
                    StateEvent::Fault(kind) => self.set_fault(kind).await?,
                },
                Ok(None) => (),
                Err(e) => return Err(e),
            }
        }
    }
    async fn set_ok(&self) -> Result<()> {
        if self.state() == State::Ok {
            return Ok(());
        }
        self.inner.state.store(true, Ordering::Relaxed);
        #[cfg(feature = "std")]
        self.inner
            .state_tx
            .send(StateEvent::Ok)
            .await
            .map_err(Error::failed)?;
        Ok(())
    }
    async fn set_fault(&self, kind: FaultKind) -> Result<()> {
        if self.state() == State::Fault && kind != FaultKind::Initial {
            return Ok(());
        }
        self.inner.state.store(false, Ordering::Relaxed);
        #[cfg(feature = "std")]
        self.inner
            .state_tx
            .send(StateEvent::Fault(kind))
            .await
            .map_err(Error::failed)?;
        self.warmup().await?;
        Ok(())
    }
    async fn warmup(&self) -> Result<()> {
        #[cfg(feature = "std")]
        async_io::Timer::after(self.inner.config.warmup).await;
        #[cfg(feature = "embassy")]
        embassy_time::Timer::after(embassy_time::Duration::from_micros(
            self.inner.config.warmup.as_micros().try_into().unwrap(),
        ))
        .await;
        self.inner.io.clear_async().await?;
        Ok(())
    }
}

/// Heartbeat client trait
pub trait Heart {
    /// Send the current edge
    fn beat(&self) -> Result<()>;
}

/// Heartbeat async client trait
pub trait HeartAsync {
    /// Send the current edge asynchronouslyyc
    fn beat_async(&self) -> impl Future<Output = Result<()>>;
}
