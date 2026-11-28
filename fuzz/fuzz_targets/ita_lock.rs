//! The `ita_lock` fuzz target (#997): the reader of `ita.lock` (ADR 0122 rules 16 and 18), on
//! any bytes, through both of its entries. The written entry takes the input's bytes as UTF-8,
//! a byte that is not replaced by U+FFFD; the decoded entry takes the bytes decoded once by
//! `SourceFile::new`, as every command reads the lock (#2252, #2282). Neither must panic, each
//! reads the same input the same way twice, and every range it gives, of an error or of a
//! package, lies within the text its ranges index. When the bytes are UTF-8, both entries give
//! the same messages, and, without a `\r\n` (where the two texts are the same), the same lock.
//!
//! The fuzz run caps inputs at 4096 bytes (`-max_len` in scripts/fuzz-targets.sh), far under the
//! 16 MiB bound, so these two legs never reach it; that the bound is measured on the bytes on
//! both entries is the unit test `bounds_are_measured_on_the_bytes_on_both_entries` of
//! una-syntax. A third leg reaches the bound's refusal on every input (#2311): a lock left
//! unread past its bound, with a length the input chooses, is always refused whole, at its
//! start, with one message that names the bound and, when the length is known and past it,
//! the length.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::ita::parse_ita_lock;
use una_syntax::ita::MAX_LOCK_BYTES;
use una_syntax::{SourceFile, Unread};

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

    // An unread lock, of a length the input chooses.
    let mut word = [0u8; 8];
    for (to, from) in word.iter_mut().zip(bytes) {
        *to = *from;
    }
    let len = usize::try_from(u64::from_le_bytes(word)).unwrap_or(usize::MAX);
    let unread = if len % 2 == 0 {
        Unread::Holds(len)
    } else {
        Unread::MoreThan(MAX_LOCK_BYTES)
    };
    let (lock, errors) = parse_ita_lock(&SourceFile::unread("ita.lock", unread));
    assert!(lock.read.packages.is_empty() && lock.read.registries.is_empty());
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].range, 0..0);
    assert!(errors[0].message.contains(&MAX_LOCK_BYTES.to_string()));
    if matches!(unread, Unread::Holds(len) if len > MAX_LOCK_BYTES) {
        assert!(errors[0].message.contains(&format!("holds {len} bytes")));
    } else {
        assert!(errors[0].message.contains("holds more than"));
    }
});
