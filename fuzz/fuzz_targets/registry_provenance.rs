//! The `registry_provenance` fuzz target: the reader of a stored provenance (ADR 0122 rules 17
//! and 30, `spec/30 §9` rule 7), on any bytes. Reading never panics, and a provenance that reads
//! is in its one encoding.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::provenance::parse;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(provenance) = parse(bytes) {
        assert_eq!(provenance.encode(), bytes);
    }
});
