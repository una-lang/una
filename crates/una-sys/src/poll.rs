//! The `kqueue` (macOS) or `epoll` (Linux) instance of the reactor (ADR 0117 rule 3, slice S7,
//! `docs/plan/0117-s7-design.md` §2). Only the system calls are here: the table of waits,
//! timers and wakes are safe code in `una-rt`. A registration is one-shot, so a descriptor that
//! stays ready never makes a waiter spin; a token is a plain integer, never read as an address;
//! the instance is close-on-exec; nothing here runs in a signal handler.
#![allow(unsafe_code)]

use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::time::Duration;

/// What a registration waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interest {
    /// The descriptor can be read, or has reached its end, or failed.
    Read,
    /// The descriptor can be written, or failed.
    Write,
}

#[cfg(target_os = "macos")]
type Raw = libc::kevent;
#[cfg(target_os = "linux")]
type Raw = libc::epoll_event;

/// A buffer for the events of one [`Poller::wait`].
pub struct Events {
    list: Vec<Raw>,
    len: usize,
}

impl Events {
    /// A buffer for at most `capacity` events of one wait, from 1 to 4096.
    pub fn with_capacity(capacity: usize) -> Events {
        // SAFETY: `kevent` and `epoll_event` are plain C structs of integers (and, for
        // `kevent`, one pointer that is never dereferenced), for which all zeros is valid.
        let zero: Raw = unsafe { std::mem::zeroed() };
        let list = vec![zero; capacity.clamp(1, 4096)];
        Events { list, len: 0 }
    }

    /// The tokens of the events of the last wait.
    pub fn tokens(&self) -> impl Iterator<Item = u64> + '_ {
        #[cfg(target_os = "macos")]
        let token = |event: &Raw| event.udata.addr() as u64;
        #[cfg(target_os = "linux")]
        let token = |event: &Raw| event.u64;
        self.list.iter().take(self.len).map(token)
    }
}

/// A `kqueue` or `epoll` instance, closed when dropped.
#[derive(Debug)]
pub struct Poller {
    fd: OwnedFd,
}

/// The result of a system call that returns -1 on failure.
fn check(result: libc::c_int) -> io::Result<usize> {
    usize::try_from(result).map_err(|_| io::Error::last_os_error())
}

impl Poller {
    /// A new instance, close-on-exec.
    pub fn new() -> io::Result<Poller> {
        // SAFETY: `epoll_create1` takes a flag and touches no memory of the process.
        #[cfg(target_os = "linux")]
        let raw = check(unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) })?;
        // SAFETY: `kqueue` takes no argument and touches no memory of the process.
        #[cfg(target_os = "macos")]
        let raw = check(unsafe { libc::kqueue() })?;
        let raw = libc::c_int::try_from(raw).map_err(|_| io::Error::from(io::ErrorKind::Other))?;
        // SAFETY: `raw` is a descriptor just opened, owned by nothing else.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        // A kqueue is not inherited by `fork` (kqueue(2)); it is made close-on-exec too, and one
        // whose flag cannot be set is closed and not used.
        // SAFETY: `fcntl` with `F_SETFD` sets a flag of an open descriptor; no memory is touched.
        #[cfg(target_os = "macos")]
        check(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) })?;
        Ok(Poller { fd })
    }

    /// Registers `fd` for one event of `interest`, which [`Poller::wait`] reports with `token`.
    pub fn add(&self, fd: BorrowedFd<'_>, interest: Interest, token: u64) -> io::Result<()> {
        self.change(fd, interest, Some(token))
    }

    /// Removes the registration of `fd` for `interest`; one already gone is not an error.
    pub fn delete(&self, fd: BorrowedFd<'_>, interest: Interest) -> io::Result<()> {
        match self.change(fd, interest, None) {
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => Ok(()),
            other => other,
        }
    }

    /// Adds a registration with `token`, or deletes one when `None`.
    #[cfg(target_os = "macos")]
    fn change(&self, fd: BorrowedFd<'_>, interest: Interest, token: Option<u64>) -> io::Result<()> {
        let change = libc::kevent {
            ident: usize::try_from(fd.as_raw_fd()).unwrap_or(usize::MAX),
            filter: match interest {
                Interest::Read => libc::EVFILT_READ,
                Interest::Write => libc::EVFILT_WRITE,
            },
            flags: token.map_or(libc::EV_DELETE, |_| libc::EV_ADD | libc::EV_ONESHOT),
            fflags: 0,
            data: 0,
            udata: std::ptr::without_provenance_mut(token.unwrap_or(0) as usize),
        };
        self.kevent(&[change], &mut [], None).map(drop)
    }

    /// One `kevent` call: applies the changes `ins`, then waits up to `wait` for events in `out`.
    #[cfg(target_os = "macos")]
    fn kevent(&self, ins: &[Raw], out: &mut [Raw], wait: Option<Duration>) -> io::Result<usize> {
        let spec = wait.map(|left| libc::timespec {
            tv_sec: libc::time_t::try_from(left.as_secs()).unwrap_or(libc::time_t::MAX),
            tv_nsec: libc::c_long::from(left.subsec_nanos()),
        });
        let at = spec.as_ref().map_or(std::ptr::null(), std::ptr::from_ref);
        let (n, m) = (ins.len().min(4096) as i32, out.len().min(4096) as i32);
        let fd = self.fd.as_raw_fd();
        // SAFETY: `ins` and `out` are valid slices of `kevent` of at least the lengths
        // given, `out` writable; `at` is null or points to a `timespec` that outlives the call.
        // With no room in `out`, a failed change is reported by the return value and `errno`.
        check(unsafe { libc::kevent(fd, ins.as_ptr(), n, out.as_mut_ptr(), m, at) })
    }

    /// Adds a registration with `token`, or deletes one when `None`.
    #[cfg(target_os = "linux")]
    fn change(&self, fd: BorrowedFd<'_>, interest: Interest, token: Option<u64>) -> io::Result<()> {
        let events = match interest {
            Interest::Read => libc::EPOLLIN | libc::EPOLLRDHUP,
            Interest::Write => libc::EPOLLOUT,
        };
        let events = (events | libc::EPOLLONESHOT) as u32;
        let mut event = libc::epoll_event {
            events,
            u64: token.unwrap_or(0),
        };
        let op = token.map_or(libc::EPOLL_CTL_DEL, |_| libc::EPOLL_CTL_ADD);
        // SAFETY: `event` is one valid `epoll_event` that the call reads (old kernels need it
        // non-null for `EPOLL_CTL_DEL` too); `fd` is open for the length of the call.
        check(unsafe { libc::epoll_ctl(self.fd.as_raw_fd(), op, fd.as_raw_fd(), &raw mut event) })
            .map(drop)
    }

    /// Waits for events until `timeout`, or without one when `None`, and returns how many
    /// [`Events::tokens`] gives. A signal that interrupts the wait gives 0 events.
    pub fn wait(&self, events: &mut Events, timeout: Option<Duration>) -> io::Result<usize> {
        events.len = 0;
        #[cfg(target_os = "macos")]
        let found = self.kevent(&[], &mut events.list, timeout);
        #[cfg(target_os = "linux")]
        let found = {
            let millis = timeout.map_or(-1, |left| {
                libc::c_int::try_from(left.as_nanos().div_ceil(1_000_000))
                    .unwrap_or(libc::c_int::MAX)
            });
            let room = events.list.len().min(4096) as libc::c_int;
            // SAFETY: the event list is at least `room` valid, writable `epoll_event`s.
            check(unsafe {
                libc::epoll_wait(self.fd.as_raw_fd(), events.list.as_mut_ptr(), room, millis)
            })
        };
        match found {
            Ok(found) => events.len = found.min(events.list.len()),
            Err(error) if error.raw_os_error() == Some(libc::EINTR) => {}
            Err(error) => return Err(error),
        }
        Ok(events.len)
    }
}

impl AsFd for Poller {
    /// The instance's descriptor, which may be registered in another poller.
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}
