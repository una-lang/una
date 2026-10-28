//! The `registry_yank` fuzz target: the reader of a yank, `ita-yank/1` (ADR 0122 rule 41,
//! `spec/30 §11` rule 11), on any text. Reading never panics, and a yank that reads names a full
//! version.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::tlog::state::parse_yank;

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
    if let Ok(yank) = parse_yank(&text) {
        assert!(!yank.name.is_empty());
        assert!(!yank.version.to_string().is_empty());
    }
});
