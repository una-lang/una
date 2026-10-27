//! The `registry_bundle` fuzz target: the reader of a bundle, `ita-bundle/1` (ADR 0122 rule 33,
//! `spec/30 §15` rule 9), on any bytes. Reading never panics, and every entry it gives lies
//! within the bytes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::registry::bundle::read;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(read) = read(bytes) {
        for range in read.entries.values() {
            assert!(range.start <= range.end && range.end <= bytes.len());
        }
    }
});
