//! The `registry_advisory` fuzz target: the reader of an advisory, `ita-advisory/1` (ADR 0122
//! rule 41, `spec/30 §11` rule 11), on any text. Reading never panics; an advisory that reads
//! names each affected version once, on its line, and a fix later than every one of them.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::tlog::state::parse_advisory;

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
    if let Ok(advisory) = parse_advisory(&text) {
        for (at, version) in advisory.affected.iter().enumerate() {
            assert_eq!(version.line(), advisory.line);
            assert!(!advisory.affected[..at].contains(version));
        }
        if let Some((fix, _)) = &advisory.fixed {
            assert!(advisory.affected.iter().all(|version| version < fix));
        }
    }
});
