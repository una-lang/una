//! The confinement record (ADR 0124 rule 2): unset until the confinement code sets it.
#![cfg(feature = "solver")]

use una_sys::solver::{confined, record_confined};

#[test]
fn the_record_is_unset_until_recorded_and_then_stays() {
    assert!(!confined());
    record_confined();
    assert!(confined());
    record_confined();
    assert!(confined());
}
