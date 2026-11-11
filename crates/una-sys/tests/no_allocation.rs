//! The handler's work allocates nothing (ADR 0116 rule 18), and a real signal keeps `errno`:
//! under a counting allocator, in a process of its own. This file holds one test only, since
//! the handlers it installs and the sticky count of stop requests are the process's.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every method forwards to the system allocator, with the same arguments.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::SeqCst);
        // SAFETY: the caller's contract for `alloc` is passed on unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller's contract for `dealloc` is passed on unchanged.
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[allow(unsafe_code)]
fn errno_location() -> *mut libc::c_int {
    // SAFETY: each returns the address of the calling thread's `errno`.
    #[cfg(target_os = "macos")]
    unsafe {
        libc::__error()
    }
    // SAFETY: as above.
    #[cfg(target_os = "linux")]
    unsafe {
        libc::__errno_location()
    }
}

#[test]
#[allow(unsafe_code)]
fn the_handlers_work_allocates_nothing_keeps_errno_and_wakes_the_pipe() {
    una_sys::signals::install().unwrap();
    // A second call gives the first's outcome.
    una_sys::signals::install().unwrap();
    let word = una_sys::attention::register();
    let before = word.load();
    let allocations = ALLOCATIONS.load(Ordering::SeqCst);
    // The handler's work, then a real signal: two requests; the third ends the process, as the
    // test `escalation` checks.
    una_sys::signals::handle(libc::SIGTERM);
    // SAFETY: `errno_location` points to this thread's `errno`; `raise` delivers SIGTERM to
    // this thread, whose handler runs before `raise` returns.
    let kept = unsafe {
        *errno_location() = 1234;
        libc::raise(libc::SIGTERM);
        *errno_location()
    };
    assert_eq!(ALLOCATIONS.load(Ordering::SeqCst), allocations);
    assert_eq!(kept, 1234);
    assert!(word.changed_since(before).is_some());
    assert_eq!(una_sys::attention::stop_requests(), 2);
    // The read end sits at `min(512, limit - 2)` or above (#1757): under a soft limit of open
    // files below 514, as a job that launchd starts has (256 on macOS), it sits lower than
    // 512. The test `wake_placement` checks the exact descriptor under each limit (#1932).
    let limit = una_sys::io::descriptor_limit().unwrap();
    let start = i32::try_from(limit.saturating_sub(2)).map_or(512, |room| room.min(512));
    let fd = una_sys::signals::wake_fd().unwrap();
    assert!(fd >= start, "the read end {fd} under a limit of {limit}");
    let mut byte = [0u8; 1];
    // SAFETY: `byte` is a valid one-byte buffer; the pipe is non-blocking.
    let read = unsafe { libc::read(fd, byte.as_mut_ptr().cast(), 1) };
    assert_eq!(read, 1);
}
