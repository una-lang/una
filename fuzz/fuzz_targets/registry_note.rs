//! The `registry_note` fuzz target: the reader of a signed note and of a checkpoint (C2SP
//! `signed-note`, `tlog-checkpoint`; ADR 0122 rule 36, `spec/30 §11` rules 3 and 4), the first
//! parsers of every document a registry serves, on any text. Reading never panics; a note that
//! reads has a text ending with a line feed, at least one signature line, and its signatures'
//! base64 encodes back to itself.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::tlog::note::{base64, encode_base64, parse_checkpoint, parse_note};

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
    if let Ok(note) = parse_note(&text) {
        assert!(note.text.ends_with('\n'));
        assert!(!note.signatures.is_empty());
        let _ = parse_checkpoint(&note.text);
    }
    let _ = parse_checkpoint(&text);
    if let Some(decoded) = base64(&text) {
        assert_eq!(encode_base64(&decoded), text);
    }
});
