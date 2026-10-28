//! The `registry_owner` fuzz target: the reader of an owner's entries, `ita-owner/1` and
//! `ita-key/1` (ADR 0122 rules 38 and 38.1, `spec/30 §14` rule 5), on any text. Reading never
//! panics; a threshold that reads is between 1 and the most keys, a registration holds at least
//! its threshold of keys, and cosigners are at least their threshold.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::tlog::owner::{Event, MAX_KEYS, parse_entry};

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
    if let Ok(entry) = parse_entry(&text) {
        assert!((1..=MAX_KEYS).contains(&entry.threshold));
        match entry.event {
            Event::Register { keys, cosigners } => {
                assert!(entry.threshold <= keys.len() && keys.len() <= MAX_KEYS);
                assert!(entry.previous.is_none());
                if let Some((k, list)) = cosigners {
                    assert!(k >= 1 && k <= list.len());
                }
            }
            Event::Cosigners(Some((k, list))) => assert!(k >= 1 && k <= list.len()),
            _ => assert!(entry.previous.is_some()),
        }
    }
});
