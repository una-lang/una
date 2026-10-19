//! The handler's work allocates nothing (ADR 0116 rule 18): it runs under a counting allocator,
//! in a process of its own, so no other test's requests count.

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

#[test]
fn the_handlers_work_allocates_nothing_and_wakes_the_pipe() {
    una_sys::signals::install().unwrap();
    let word = una_sys::attention::register();
    let before = word.load();
    let allocations = ALLOCATIONS.load(Ordering::SeqCst);
    // Two requests: the third would restore the default action and end this process.
    una_sys::signals::handle(libc::SIGTERM);
    una_sys::signals::handle(libc::SIGTERM);
    assert_eq!(ALLOCATIONS.load(Ordering::SeqCst), allocations);
    assert_ne!(word.load(), before);
    assert_eq!(una_sys::attention::stop_requests(), 2);
    let fd = una_sys::signals::wake_fd().unwrap();
    let mut byte = [0u8; 1];
    // SAFETY: `byte` is a valid one-byte buffer; the pipe is non-blocking.
    #[allow(unsafe_code)]
    let read = unsafe { libc::read(fd, byte.as_mut_ptr().cast(), 1) };
    assert_eq!(read, 1);
    una_sys::signals::drain_wake();
}
