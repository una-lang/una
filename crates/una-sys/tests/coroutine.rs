//! The coroutines of `una-sys` (ADR 0117 rule 2, the security review of S1): switching back and
//! forth, nesting, many at once, discarded stacks, and, in child processes, a recursion stopped
//! by the guard page and the aborts of a dropped or leaked suspended coroutine and of a panicking
//! body.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used)]

use std::cell::Cell;
use std::rc::Rc;

use una_sys::coroutine::{Coroutine, Resumed, Stack, inside, scope, suspend};

fn stack() -> Stack {
    Stack::new(256 * 1024).unwrap()
}

#[test]
fn a_body_suspends_and_resumes_and_its_values_cross() {
    assert!(!inside() && !suspend());
    let seen = Rc::new(Cell::new(0));
    let inner = Rc::clone(&seen);
    scope(|s| {
        let mut coroutine = s.spawn(stack(), move || {
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
        let reused = coroutine.into_stack().unwrap();
        let mut again = s.spawn(reused, || {
            let floats = [1.5f64, 2.5];
            suspend();
            assert_eq!(floats.iter().sum::<f64>(), 4.0);
        });
        assert_eq!(again.resume(), Resumed::Suspended);
        assert_eq!(again.resume(), Resumed::Finished);
    });
}

#[test]
fn suspend_leaves_the_innermost_coroutine_only() {
    let log = Rc::new(std::cell::RefCell::new(Vec::new()));
    let outer_log = Rc::clone(&log);
    scope(|s| {
        let mut outer = s.spawn(stack(), move || {
            let inner_log = Rc::clone(&outer_log);
            scope(|s| {
                let mut inner = s.spawn(stack(), move || {
                    inner_log.borrow_mut().push("inner 1");
                    suspend();
                    inner_log.borrow_mut().push("inner 2");
                });
                assert_eq!(inner.resume(), Resumed::Suspended);
                outer_log.borrow_mut().push("outer");
                suspend();
                assert_eq!(inner.resume(), Resumed::Finished);
            });
        });
        assert_eq!(outer.resume(), Resumed::Suspended);
        assert_eq!(outer.resume(), Resumed::Finished);
    });
    assert_eq!(*log.borrow(), ["inner 1", "outer", "inner 2"]);
}

#[test]
fn a_body_never_started_is_dropped_and_its_stack_given_back() {
    let held = Rc::new(());
    let captured = Rc::clone(&held);
    scope(|s| {
        let coroutine = s.spawn(stack(), move || drop(captured));
        assert_eq!(Rc::strong_count(&held), 2);
        assert!(coroutine.into_stack().is_ok());
        assert_eq!(Rc::strong_count(&held), 1);
        let mut suspended = s.spawn(stack(), || {
            suspend();
        });
        suspended.resume();
        let mut suspended = suspended.into_stack().unwrap_err();
        assert_eq!(suspended.resume(), Resumed::Finished);
    });
}

/// The resident pages of the usable part of `stack`, by `mincore`.
#[allow(unsafe_code)]
fn resident(stack: &Stack) -> usize {
    let page = una_sys::coroutine::page_size();
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
    scope(|s| {
        let mut coroutine = s.spawn(Stack::new(1024 * 1024).unwrap(), || {
            let touched = std::hint::black_box([7u8; 256 * 1024]);
            std::hint::black_box(&touched);
        });
        assert_eq!(coroutine.resume(), Resumed::Finished);
        let mut stack = coroutine.into_stack().unwrap();
        assert!(resident(&stack) >= 4, "{}", resident(&stack));
        stack.discard().unwrap();
        assert_eq!(resident(&stack), 0);
        let mut again = s.spawn(stack, || {
            std::hint::black_box([1u8; 256 * 1024]);
        });
        assert_eq!(again.resume(), Resumed::Finished);
    });
}

/// On Linux, a stack's mapping takes no transparent huge pages: its `VmFlags` in
/// `/proc/self/smaps` say `nh` (review of S1, N3).
#[cfg(target_os = "linux")]
#[test]
fn a_stack_takes_no_huge_pages() {
    let stack = Stack::new(4 * 1024 * 1024).unwrap();
    let start = format!("{:x}-", stack.top() - stack.usable());
    let smaps = std::fs::read_to_string("/proc/self/smaps").unwrap();
    // `VmFlags` is the last line of each mapping's entry, so the first after its header is its.
    let mapping = smaps
        .lines()
        .skip_while(|line| !line.starts_with(&start))
        .find(|line| line.starts_with("VmFlags:"))
        .unwrap();
    assert!(
        mapping.split_whitespace().any(|flag| flag == "nh"),
        "{mapping}"
    );
}

#[test]
fn ten_thousand_coroutines_wait_at_once() {
    let total = Rc::new(Cell::new(0usize));
    scope(|s| {
        let mut all: Vec<Coroutine<'_>> = (0..10_000)
            .map(|index| {
                let total = Rc::clone(&total);
                s.spawn(Stack::new(16 * 1024).unwrap(), move || {
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
    });
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
fn the_guard_page_stops_a_recursion_and_drops_leaks_and_panics_abort() {
    assert!(matches!(
        signal("child_overflows"),
        Some(libc::SIGSEGV | libc::SIGBUS)
    ));
    for name in [
        "child_drops_a_suspended_coroutine",
        "child_leaks_a_suspended_coroutine",
        "child_lends_a_thread_local_then_leaks_its_coroutine",
        "child_panics_in_a_body",
    ] {
        assert_eq!(signal(name), Some(libc::SIGABRT), "{name}");
    }
}

fn deep(n: u64) -> u64 {
    let pad = std::hint::black_box([n; 64]);
    if n == 0 { 0 } else { deep(n - 1) + pad[7] }
}

#[test]
#[ignore = "run in a child process"]
fn child_overflows() {
    scope(|s| {
        let mut coroutine = s.spawn(Stack::new(64 * 1024).unwrap(), || {
            std::hint::black_box(deep(1_000_000));
        });
        coroutine.resume();
    });
}

#[test]
#[ignore = "run in a child process"]
fn child_drops_a_suspended_coroutine() {
    scope(|s| {
        let mut coroutine = s.spawn(stack(), || {
            suspend();
        });
        coroutine.resume();
        drop(coroutine);
    });
}

#[test]
#[ignore = "run in a child process"]
fn child_leaks_a_suspended_coroutine() {
    // A forgotten suspended coroutine keeps its scope's count up: the scope aborts rather than
    // return.
    scope(|s| {
        let mut coroutine = s.spawn(stack(), || {
            suspend();
        });
        coroutine.resume();
        std::mem::forget(coroutine);
    });
}

thread_local! {
    static DATA: Vec<u64> = vec![0xAAAA_AAAA; 4];
}

#[test]
#[ignore = "run in a child process"]
fn child_lends_a_thread_local_then_leaks_its_coroutine() {
    // Review of S1, N1: a body lends a thread-local to a scoped thread and suspends; its
    // coroutine is leaked. The scope aborts before its thread can exit and free the
    // thread-local, so the scoped thread never reads freed memory.
    let thread = std::thread::spawn(|| {
        scope(|s| {
            let mut coroutine = s.spawn(stack(), || {
                DATA.with(|data| {
                    std::thread::scope(|threads| {
                        threads.spawn(move || std::hint::black_box(data.first().copied()));
                        suspend();
                    });
                });
            });
            coroutine.resume();
            std::mem::forget(coroutine);
        });
    });
    let _ = thread.join();
}

#[test]
#[ignore = "run in a child process"]
fn child_panics_in_a_body() {
    scope(|s| {
        let mut coroutine = s.spawn(stack(), || panic!("a test body panics"));
        coroutine.resume();
    });
}

#[test]
fn sanitizer_interface_is_absent_outside_asan_builds() {
    // #1145: the switch is announced to AddressSanitizer only in builds with `--cfg una_asan`,
    // which only scripts/sanitize.sh sets, and that script never runs this test. Every other
    // build, the ordinary tests and every release, has no reference to the sanitizer's
    // interface; scripts/check-release.sh checks the packed binary and runtime for the symbols.
    assert!(!cfg!(una_asan));
}
