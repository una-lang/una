//! The `ita_lock` fuzz target (#997): the reader of `ita.lock` (ADR 0122 rules 16 and 18), on
//! any bytes, through both of its entries. The written entry takes the input's bytes as UTF-8,
//! a byte that is not replaced by U+FFFD; the decoded entry takes the bytes decoded once by
//! `SourceFile::new`, as every command reads the lock (#2252, #2282). Neither must panic, each
//! reads the same input the same way twice, and every range it gives, of an error or of a
//! package, lies within the text its ranges index. When the bytes are UTF-8, both entries give
//! the same messages, and, without a `\r\n` (where the two texts are the same), the same lock.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::ita::parse_ita_lock;
use una_syntax::SourceFile;

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

    // The decoded entry.
    let file = SourceFile::new("ita.lock", bytes);
    let decoded = file.text();
    let (lock_decoded, errors_decoded) = parse_ita_lock(&file);
    let (again, errors_again) = parse_ita_lock(&file);
    assert_eq!(lock_decoded.read, again.read);
    assert_eq!(errors_decoded, errors_again);
    for error in &errors_decoded {
        assert!(decoded.get(error.range.clone()).is_some());
    }
    for package in &lock_decoded.read.packages {
        assert!(decoded.get(package.range.clone()).is_some());
    }
    if std::str::from_utf8(bytes).is_ok() {
        let messages = |errors: &[una_project::ProjectError]| -> Vec<String> {
            errors.iter().map(|error| error.message.clone()).collect()
        };
        assert_eq!(messages(&errors), messages(&errors_decoded));
        if !text.contains("\r\n") {
            assert_eq!(lock.read, lock_decoded.read);
            assert_eq!(errors, errors_decoded);
        }
    }
});
