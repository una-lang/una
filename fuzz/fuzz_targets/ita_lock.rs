//! The `ita_lock` fuzz target (#997): the reader of `ita.lock` (ADR 0122 rules 16 and 18), on
//! any text: the input's bytes as UTF-8, a byte that is not replaced by U+FFFD. It must never
//! panic, it reads the same text the same way twice, and every range it gives, of an error or
//! of a package, lies within the text.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::ita::parse_ita_lock;

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
    let (lock, errors) = parse_ita_lock(&text);
    let (again, errors_again) = parse_ita_lock(&text);
    assert_eq!(lock.read, again.read);
    assert_eq!(errors, errors_again);
    for error in &errors {
        assert!(error.range.start <= error.range.end && error.range.end <= text.len());
    }
    for package in &lock.read.packages {
        assert!(text.get(package.range.clone()).is_some());
    }
});
