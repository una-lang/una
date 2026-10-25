//! The `registry_promise` fuzz target: the reader of a registry's promise, `ita-promise/1`
//! (ADR 0122 rule 36.2, `spec/30 §15` rule 5), on any text: its text parser directly, which a
//! signature check would otherwise keep the fuzzer from reaching, and the whole check against a
//! fixed log key. Reading never panics; a text that reads names a source and a leaf hash, and a
//! promise that verifies is of the source and leaf asked for.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::registry::submit::{check_promise, parse_promise};
use una_project::tlog::note::Key;

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
    if let Ok(read) = parse_promise(&text) {
        assert!(read.leaf.starts_with("sha256:"));
        assert!(!read.source.is_empty());
    }
    let log = Key {
        name: "acme.example/log".to_owned(),
        public: [7; 32],
    };
    let leaf = "sha256:abababababababababababababababababababababababababababababababab";
    if let Ok(promise) = check_promise(&text, &log, "acme", leaf) {
        assert_eq!(promise.source, "acme");
        assert_eq!(promise.leaf, leaf);
    }
});
