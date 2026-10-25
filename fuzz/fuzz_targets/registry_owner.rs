//! The `registry_owner` fuzz target: the reader of an owner's key event (ADR 0122 rules 38 and
//! 38.1, `spec/30 §14` rule 5), on any text. Reading never panics; a threshold that reads is at
//! least 1, and cosigners are at least their threshold.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::tlog::owner::{Event, parse_entry};

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
    if let Ok(entry) = parse_entry(&text) {
        match entry.event {
            Event::Threshold(k) => assert!(k >= 1),
            Event::Cosigners { threshold, keys } => {
                assert!(threshold >= 1 && threshold <= keys.len());
            }
            _ => {}
        }
    }
});
