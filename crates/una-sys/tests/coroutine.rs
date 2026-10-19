//! The coroutines of `una-sys` (ADR 0117 rule 2, the security review of S1): switching back and
//! forth, nesting, many at once, a recursion stopped by the guard page, and the aborts of a
//! dropped suspended coroutine and of a panicking body, the last three in child processes.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used)]

use std::cell::Cell;
use std::rc::Rc;

use una_sys::coroutine::{Coroutine, Resumed, Stack, inside, suspend};

fn stack() -> Stack {
    Stack::new(256 * 1024).unwrap()
}

#[test]
fn a_body_suspends_and_resumes_and_its_values_cross() {
    assert!(!inside() && !suspend());
    let seen = Rc::new(Cell::new(0));
    let inner = Rc::clone(&seen);
    let mut coroutine = Coroutine::new(stack(), move || {
        for step in 1..=3 {
            assert!(inside());
            inner.set(step);
            assert!(suspend());
        }
        inner.set(10);
    });
    for step in 1..=3 {
        assert_eq!(coroutine.resume(), Resumed::Suspended);
        assert_eq!(seen.get(), step);
        assert!(!inside());
    }
    assert_eq!(coroutine.resume(), Resumed::Finished);
    assert_eq!(seen.get(), 10);
    assert_eq!(coroutine.resume(), Resumed::Finished);
    let reused = coroutine.into_stack().ok().unwrap();
    let mut again = Coroutine::new(reused, || {
        let floats = [1.5f64, 2.5];
        suspend();
        assert_eq!(floats.iter().sum::<f64>(), 4.0);
    });
    assert_eq!(again.resume(), Resumed::Suspended);
    assert_eq!(again.resume(), Resumed::Finished);
}

#[test]
fn suspend_leaves_the_innermost_coroutine_only() {
    let log = Rc::new(std::cell::RefCell::new(Vec::new()));
    let outer_log = Rc::clone(&log);
    let mut outer = Coroutine::new(stack(), move || {
        let inner_log = Rc::clone(&outer_log);
        let mut inner = Coroutine::new(stack(), move || {
            inner_log.borrow_mut().push("inner 1");
            suspend();
            inner_log.borrow_mut().push("inner 2");
        });
        assert_eq!(inner.resume(), Resumed::Suspended);
        outer_log.borrow_mut().push("outer");
        suspend();
        assert_eq!(inner.resume(), Resumed::Finished);
    });
    assert_eq!(outer.resume(), Resumed::Suspended);
    assert_eq!(outer.resume(), Resumed::Finished);
    assert_eq!(*log.borrow(), ["inner 1", "outer", "inner 2"]);
}

#[test]
fn a_body_never_started_is_dropped_and_its_stack_given_back() {
    let held = Rc::new(());
    let captured = Rc::clone(&held);
    let coroutine = Coroutine::new(stack(), move || drop(captured));
    assert_eq!(Rc::strong_count(&held), 2);
    assert!(coroutine.into_stack().is_ok());
    assert_eq!(Rc::strong_count(&held), 1);
    let mut suspended = Coroutine::new(stack(), || {
        suspend();
    });
    suspended.resume();
    let mut suspended = suspended.into_stack().err().unwrap();
    assert_eq!(suspended.resume(), Resumed::Finished);
}

/// The resident pages of the usable part of `stack`, by `mincore`.
#[allow(unsafe_code)]
fn resident(stack: &Stack) -> usize {
    // SAFETY: `sysconf` reads a constant of the system.
    let page = usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).unwrap();
    let start = std::ptr::with_exposed_provenance_mut::<libc::c_void>(stack.top() - stack.usable());
    let mut pages = vec![0u8; stack.usable().div_ceil(page)];
    // SAFETY: the range is mapped by `stack`, and `pages` holds one byte per page of it.
    let done = unsafe { libc::mincore(start, stack.usable(), pages.as_mut_ptr().cast()) };
    assert_eq!(done, 0);
    pages.iter().filter(|page| **page & 1 != 0).count()
}

#[test]
fn a_discarded_stack_holds_no_resident_page_and_runs_again() {
    // The pool gives a stack back to the system before it keeps it, so pooled stacks never hold
    // memory outside `--max-memory` (review of S1, finding 3).
    let mut coroutine = Coroutine::new(Stack::new(1024 * 1024).unwrap(), || {
        let touched = std::hint::black_box([7u8; 256 * 1024]);
        std::hint::black_box(&touched);
    });
    assert_eq!(coroutine.resume(), Resumed::Finished);
    let mut stack = coroutine.into_stack().ok().unwrap();
    assert!(resident(&stack) >= 4, "{}", resident(&stack));
    stack.discard().unwrap();
    assert_eq!(resident(&stack), 0);
    let mut again = Coroutine::new(stack, || {
        std::hint::black_box([1u8; 256 * 1024]);
    });
    assert_eq!(again.resume(), Resumed::Finished);
}

#[test]
fn ten_thousand_coroutines_wait_at_once() {
    let total = Rc::new(Cell::new(0usize));
    let mut all: Vec<Coroutine> = (0..10_000)
        .map(|index| {
            let total = Rc::clone(&total);
            Coroutine::new(Stack::new(16 * 1024).unwrap(), move || {
                suspend();
                total.set(total.get() + index);
            })
        })
        .collect();
    for coroutine in &mut all {
        assert_eq!(coroutine.resume(), Resumed::Suspended);
    }
    for coroutine in &mut all {
        assert_eq!(coroutine.resume(), Resumed::Finished);
    }
    assert_eq!(total.get(), 10_000 * 9_999 / 2);
}

/// Runs the ignored test `name` in a child process: how it ended.
fn child(name: &str) -> std::process::ExitStatus {
    let exe = std::env::current_exe().unwrap();
    std::process::Command::new(exe)
        .args(["--ignored", "--exact", "--test-threads=1"])
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
}

fn signal(name: &str) -> Option<i32> {
    std::os::unix::process::ExitStatusExt::signal(&child(name))
}

#[test]
fn the_guard_page_stops_a_recursion_and_drops_and_panics_abort() {
    assert!(matches!(
        signal("child_overflows"),
        Some(libc::SIGSEGV | libc::SIGBUS)
    ));
    assert_eq!(
        signal("child_drops_a_suspended_coroutine"),
        Some(libc::SIGABRT)
    );
    assert_eq!(signal("child_panics_in_a_body"), Some(libc::SIGABRT));
}

fn deep(n: u64) -> u64 {
    let pad = std::hint::black_box([n; 64]);
    if n == 0 { 0 } else { deep(n - 1) + pad[7] }
}

#[test]
#[ignore = "run in a child process"]
fn child_overflows() {
    let mut coroutine = Coroutine::new(Stack::new(64 * 1024).unwrap(), || {
        std::hint::black_box(deep(1_000_000));
    });
    coroutine.resume();
}

#[test]
#[ignore = "run in a child process"]
fn child_drops_a_suspended_coroutine() {
    let mut coroutine = Coroutine::new(stack(), || {
        suspend();
    });
    coroutine.resume();
    drop(coroutine);
}

#[test]
#[ignore = "run in a child process"]
fn child_panics_in_a_body() {
    let mut coroutine = Coroutine::new(stack(), || panic!("a test body panics"));
    coroutine.resume();
}
