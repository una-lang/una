//! The `registry_mirrors` fuzz target: the reader of Itaoca's `mirrors.una` (ADR 0122 rule
//! 40, `spec/30 §14`), on any text. Reading never panics, and every mirror it keeps is an
//! `https://` location or a local directory, never a plain `http://` one.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::mirrors::parse;
use una_project::transport::Location;

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
    let (mirrors, _) = parse(&text);
    for list in mirrors.by_source.values() {
        for mirror in list {
            assert!(!mirror.url.starts_with("http://"));
            assert!(matches!(
                mirror.location,
                Location::Https(_) | Location::Directory(_)
            ));
        }
    }
});
