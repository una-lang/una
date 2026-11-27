//! The `registry_mirrors` fuzz target: the reader of Itaoca's `mirrors.una` (ADR 0122 rule
//! 40, `spec/30 §14`), on any bytes, through both of its entries: the bytes as UTF-8, a byte
//! that is not replaced by U+FFFD, and the bytes decoded once by `SourceFile::new` (#2282).
//! Reading never panics, every range lies within the text it indexes, and every mirror it
//! keeps is an `https://` location or a local directory, never a plain `http://` one. When the
//! bytes are UTF-8, both entries keep the same mirrors and give the same messages.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::mirrors::{Mirrors, parse};
use una_project::transport::Location;
use una_syntax::SourceFile;

fn only_secure(mirrors: &Mirrors) {
    for list in mirrors.by_source.values() {
        for mirror in list {
            assert!(!mirror.url.starts_with("http://"));
            assert!(matches!(
                mirror.location,
                Location::Https(_) | Location::Directory(_)
            ));
        }
    }
}

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
    let (mirrors, errors) = parse(&text);
    only_secure(&mirrors);
    for error in &errors {
        assert!(error.range.start <= error.range.end && error.range.end <= text.len());
    }

    // The decoded entry.
    let file = SourceFile::new("mirrors.una", bytes);
    let (mirrors_decoded, errors_decoded) = parse(&file);
    only_secure(&mirrors_decoded);
    for error in &errors_decoded {
        assert!(file.text().get(error.range.clone()).is_some());
    }
    if std::str::from_utf8(bytes).is_ok() {
        assert_eq!(mirrors.by_source, mirrors_decoded.by_source);
        let messages = |errors: &[una_project::ProjectError]| -> Vec<String> {
            errors.iter().map(|error| error.message.clone()).collect()
        };
        assert_eq!(messages(&errors), messages(&errors_decoded));
    }
});
