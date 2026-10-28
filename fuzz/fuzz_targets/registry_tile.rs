//! The `registry_tile` fuzz target: the readers of a hash tile and of an entry bundle
//! (C2SP `tlog-tiles`, ADR 0122 rule 34, `spec/30 §14` rule 2), on any bytes, at the width the
//! first byte gives. Reading never panics, and what reads holds exactly that width.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::tlog::tile::{WIDTH, parse_entry_bundle, parse_hash_tile};

fuzz_target!(|bytes: &[u8]| {
    let Some((first, rest)) = bytes.split_first() else {
        return;
    };
    let width = u64::from(*first) % WIDTH + 1;
    let full = first & 0x80 != 0 && width < WIDTH;
    if let Ok(hashes) = parse_hash_tile(rest, width, full) {
        assert_eq!(hashes.len() as u64, width);
    }
    if let Ok(entries) = parse_entry_bundle(rest, width, full) {
        assert_eq!(entries.len() as u64, width);
    }
});
