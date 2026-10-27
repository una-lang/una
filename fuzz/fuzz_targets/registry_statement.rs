//! The `registry_statement` fuzz target: the reader of a release statement (ADR 0122 rule 37,
//! `spec/30 §11` rule 6) and the check of rule 26 between two statements, on any text. Reading
//! never panics; a statement that reads is within its own authority and trust.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::tlog::release::{parse_statement, within};

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
    if let Ok(statement) = parse_statement(&text) {
        assert!(within(&statement, &statement).is_ok());
        assert!(statement.needs.iter().all(|(source, _, _)| *source == statement.source));
    }
});
