//! What the serve engine needs from the machine: sockets, readiness, a clock and a way to wake a
//! sleeping thread. The engine runs the same code over real sockets in both engines and over
//! scripted ones in the conformance tests (ADR 0123 rules 20 and 21), so none of it names a
//! system call.

use std::io;
use std::net::TcpStream;
use std::os::fd::{AsFd, BorrowedFd};
use std::time::Instant;

/// A connection's byte stream, non-blocking: a read or write that cannot proceed fails with
/// [`io::ErrorKind::WouldBlock`].
pub trait Socket: Send {
    /// Reads at most `buf.len()` bytes; `Ok(0)` is the end of the peer's stream.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    /// Writes a prefix of `buf` and returns its length.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize>;
    /// Shuts down the sending side, so the peer reads what was sent instead of a reset.
    fn shutdown_write(&mut self);
    /// The descriptor to register for readiness, when the socket has one.
    fn fd(&self) -> Option<BorrowedFd<'_>> {
        None
    }
}

impl Socket for TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        io::Read::read(self, buf)
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        io::Write::write(self, buf)
    }

    fn shutdown_write(&mut self) {
        let _ = TcpStream::shutdown(self, std::net::Shutdown::Write);
    }

    fn fd(&self) -> Option<BorrowedFd<'_>> {
        Some(self.as_fd())
    }
}

/// Where a serve thread learns that a socket can be read or written. The thread tells it, after
/// each event it handles, what the connection still waits for; the implementation registers
/// that interest once, one-shot, and forgets a socket before the engine drops it.
pub trait Readiness: Send {
    /// The connection `token` waits to read and/or to write; both false means neither. An error
    /// means the interest is not registered, so nothing would ever report the socket: the
    /// thread closes the connection at once (#2279).
    fn set(&mut self, token: u64, socket: &dyn Socket, read: bool, write: bool) -> io::Result<()>;
    /// The connection `token` is closing.
    fn forget(&mut self, token: u64, socket: &dyn Socket);
}

/// The run's clock.
pub trait Clock: Send + Sync {
    /// Nanoseconds of the monotonic clock since the run began.
    fn now(&self) -> u64;
    /// Seconds since 1970-01-01T00:00:00Z, for `Date`.
    fn unix_seconds(&self) -> u64;
}

/// Wakes a serve thread that sleeps in its poller.
pub trait Waker: Send + Sync {
    /// Wakes the thread; may be called from any thread.
    fn wake(&self);
}

/// The monotonic clock of a real run.
#[derive(Debug)]
pub struct SystemClock {
    start: Instant,
    unix_at_start: u64,
}

impl SystemClock {
    /// A clock whose zero is now.
    pub fn new() -> SystemClock {
        let unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        SystemClock {
            start: Instant::now(),
            unix_at_start: unix,
        }
    }
}

impl Default for SystemClock {
    fn default() -> SystemClock {
        SystemClock::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    fn unix_seconds(&self) -> u64 {
        self.unix_at_start
            .saturating_add(self.start.elapsed().as_secs())
    }
}
