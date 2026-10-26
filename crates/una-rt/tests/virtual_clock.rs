//! The virtual clock of `--virtual-time` (ADR 0116 rule 19, `spec/26 §8`) in a process that
//! runs more than one run: the clock is set once per process, and each virtual run restarts it
//! at 0. A test binary of its own, so that no run on the real clock shares its process.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used)]

use std::sync::Mutex;

use una_rt::host::clock_now;
use una_rt::runtime::{clock, start};
use una_rt::values::{int_value, release};

/// The readings of `Clock.now` the runs made, in order.
static READINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Reads `Clock.now` as the compiled code does, and records it.
fn read() {
    let word = clock_now();
    let value = int_value(word);
    release(word);
    READINGS.lock().unwrap().push(value.to_string());
}

#[test]
fn each_virtual_run_in_one_process_starts_at_zero() {
    let meta = r#"{"grants":[],"uses":[]}"#;
    let args = ["--virtual-time".to_owned()];
    // The first run reads 0, then waits half a second of virtual time, at once.
    let first = start(&args, meta, || {
        read();
        clock().wait_until(500);
        read();
        0
    });
    assert_eq!(first, 0);
    // The second run starts at 0 again, not where the first one left the clock.
    let second = start(&args, meta, || {
        read();
        0
    });
    assert_eq!(second, 0);
    assert_eq!(*READINGS.lock().unwrap(), ["0", "500", "0"]);
}
