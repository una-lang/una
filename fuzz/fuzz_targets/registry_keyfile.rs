//! The `registry_keyfile` fuzz target: the reader of a publisher's key file, `ita-keyfile/1`
//! (ADR 0122 rule 38, `spec/30 §15` rule 4), on any text. Reading never panics, and a key that
//! reads encodes back to the same file.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::registry::keyfile::SigningKey;

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
    if let Ok(key) = SigningKey::parse(&text) {
        assert_eq!(SigningKey::parse(&key.encode()).map(|k| k.key().clone()).ok(), Some(key.key().clone()));
    }
});
