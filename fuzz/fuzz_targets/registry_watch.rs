//! The `registry_watch` fuzz target: the readers of a watch position, `ita-watch/1`, and of a
//! submission's record, `ita-submitted/1` (ADR 0122 rules 36 and 36.2, `spec/30 §15` rules 5
//! and 8), on any text. Reading never panics, and what reads encodes back to the same text.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::registry::watch::{Position, Submitted};

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
    if let Ok(position) = Position::parse(&text, "acme", "acme") {
        assert_eq!(position.encode(), text);
    }
    if let Ok(record) = Submitted::parse("ab", &text) {
        assert_eq!(record.encode(), text);
    }
});
