//! The `registry_promise` fuzz target: the reader of a registry's promise, `ita-promise/1`
//! (ADR 0122 rule 36.2, `spec/30 §15` rule 5), against a fixed log key, on any text. Reading
//! never panics, and a promise that verifies is of the source and leaf asked for.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::registry::submit::check_promise;
use una_project::tlog::note::Key;

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
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
