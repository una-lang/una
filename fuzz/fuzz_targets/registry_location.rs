//! The `registry_location` fuzz target: the reader of a registry's location (ADR 0122 rule 40,
//! `spec/30 §14` rule 1) and of a hash as the lock writes it, on any text. Reading never
//! panics; a location that reads is `https://` or a directory from a `file://` URL with no
//! `%`, and a hash that reads writes back to the same text.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::client::{hash_text, parse_hash_text};
use una_project::transport::Location;

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
    match Location::parse(&text) {
        Ok(Location::Https(url)) => assert!(url.starts_with("https://")),
        Ok(Location::Directory(_)) => {
            assert!(text.starts_with("file:///") && !text.contains('%'));
        }
        Err(_) => {}
    }
    if let Some(hash) = parse_hash_text(&text) {
        assert_eq!(hash_text(&hash), text);
    }
});
