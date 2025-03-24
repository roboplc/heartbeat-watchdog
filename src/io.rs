use core::future::Future;

use crate::{Edge, Result};

/// Generic watchdog I/O trait
#[allow(clippy::module_name_repetitions)]
pub trait WatchdogIo {
    /// gets the next edge, the expected edge can be used to detect changes in case of an analogue
    /// source (e.g. GPIO)
    fn get(&self, _expected: Edge) -> Result<Edge>;
    /// clears the watchdog I/O, e.g. a socket buffer in case of TCP/IP
    fn clear(&self) -> Result<()>;
}

/// Generic watchdog I/O trait
#[allow(clippy::module_name_repetitions)]
pub trait WatchdogIoAsync {
    /// gets the next edge asynchronously, the expected edge can be used to detect changes in case
    fn get(&self, _expected: Edge) -> impl Future<Output = Result<Edge>> + Send;
    /// clears the watchdog I/O asynchronously
    fn clear(&self) -> impl Future<Output = Result<()>> + Send;
}

#[cfg(feature = "gpio")]
/// GPIO communication
pub mod gpio {

    use crate::{Edge, Error, Result};
    use std::{
        path::{Path, PathBuf},
        time::{Duration, Instant},
    };

    use gpio_cdev::{Chip, LineHandle, LineRequestFlags};
    use portable_atomic::AtomicBool;
    use rtsc::time::interval;

    use super::WatchdogIo;

    #[allow(clippy::module_name_repetitions)]
    /// GPIO client
    pub struct GpioHeart {
        handle: LineHandle,
        next: AtomicBool,
    }

    impl GpioHeart {
        /// creates a new GPIO client
        pub fn create<P: AsRef<Path>>(chip: P, offset: u32) -> Result<Self> {
            let mut chip = Chip::new(chip).map_err(Error::failed)?;
            let line = chip.get_line(offset).map_err(Error::failed)?;
            let handle = line
                .request(LineRequestFlags::OUTPUT, 0, "gpio-heartbeat")
                .map_err(Error::failed)?;
            Ok(Self {
                handle,
                next: AtomicBool::new(true),
            })
        }
    }

    impl crate::Heart for GpioHeart {
        fn beat(&self) -> Result<()> {
            self.handle
                .set_value(
                    self.next
                        .fetch_xor(true, std::sync::atomic::Ordering::Relaxed)
                        .into(),
                )
                .map_err(Error::failed)
        }
    }

    #[allow(clippy::module_name_repetitions)]
    /// GPIO watchdog I/O configuration
    #[derive(Debug, Clone)]
    pub struct GpioConfig {
        chip: PathBuf,
        offset: u32,
        pull_interval: Duration,
    }

    impl GpioConfig {
        /// creates a new GPIO watchdog I/O configuration
        pub fn new<P: AsRef<Path>>(chip: P, offset: u32, pull_interval: Duration) -> Self {
            Self {
                chip: chip.as_ref().to_path_buf(),
                offset,
                pull_interval
            }
        }
    }

    /// GPIO watchdog I/O
    pub struct Gpio {
        handle: LineHandle,
        timeout: Duration,
        pull_interval: Duration,
    }

    impl Gpio {
        /// creates a new GPIO watchdog I/O
        pub fn create(config: &GpioConfig, timeout: Duration) -> Result<Self> {
            let mut chip = Chip::new(&config.chip).map_err(Error::failed)?;
            let line = chip.get_line(config.offset).map_err(Error::failed)?;
            let handle = line
                .request(LineRequestFlags::INPUT, 0, "gpio-watchdog")
                .map_err(Error::failed)?;
            Ok(Self {
                handle,
                timeout,
                pull_interval: config.pull_interval,
            })
        }
    }

    impl WatchdogIo for Gpio {
        fn get(&self, expected: crate::Edge) -> Result<crate::Edge> {
            let now = Instant::now();
            for _ in interval(self.pull_interval) {
                if now.elapsed() > self.timeout {
                    break;
                }
                let edge: Edge = self.handle.get_value().map_err(Error::failed)?.into();
                if edge == expected {
                    return Ok(edge);
                }
            }
            Err(Error::Timeout)
        }

        fn clear(&self) -> Result<()> {
            Ok(())
        }
    }
}

/// UDP communication
#[cfg(feature = "std")]
pub mod udp {
    use crate::{Edge, Error, Heart, Result};
    use core::time::Duration;
    use std::{
        net::{ToSocketAddrs, UdpSocket},
        thread,
    };

    use portable_atomic::{AtomicBool, Ordering};

    use super::WatchdogIo;

    /// UDP client
    #[allow(clippy::module_name_repetitions)]
    pub struct UdpHeart {
        socket: UdpSocket,
        next: AtomicBool,
    }

    impl UdpHeart {
        /// creates a new UDP client
        pub fn create<A: ToSocketAddrs>(addr: A) -> Result<Self> {
            let socket = UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, 0))?;
            socket.connect(addr)?;
            Ok(Self {
                socket,
                next: AtomicBool::new(true),
            })
        }
    }

    impl Heart for UdpHeart {
        fn beat(&self) -> Result<()> {
            self.socket
                .send(&[Edge::from(self.next.fetch_xor(true, Ordering::Relaxed)) as u8])
                .map_err(Error::from)?;
            Ok(())
        }
    }

    /// UDP watchdog I/O
    #[allow(clippy::module_name_repetitions)]
    pub struct UdpIo {
        socket: UdpSocket,
    }

    impl UdpIo {
        /// creates a new UDP watchdog I/O
        pub fn create<A: ToSocketAddrs>(addr: A, timeout: Duration) -> Result<Self>
        where
            Self: Sized,
        {
            let socket = UdpSocket::bind(addr)?;
            socket.set_read_timeout(Some(timeout))?;
            Ok(Self { socket })
        }
    }

    impl WatchdogIo for UdpIo {
        fn get(&self, _expected: Edge) -> Result<Edge> {
            let mut buf = [0];
            while self.socket.recv(&mut buf)? == 0 {}
            Ok(Edge::from(buf[0]))
        }

        fn clear(&self) -> Result<()> {
            self.socket.set_nonblocking(true)?;
            while self.socket.recv(&mut [0]).is_ok() {
                // should never happen, but just in case
                thread::yield_now();
            }
            self.socket.set_nonblocking(false)?;
            Ok(())
        }
    }
}
