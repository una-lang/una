//! The `registry_archive_check` fuzz target: the check of a release statement against an
//! archive (ADR 0122 rule 37, `spec/30 §14` rule 9), on a fuzzed statement and a fuzzed archive,
//! cut at the first NUL byte. Checking never panics, and a statement that agrees with an
//! archive gives its digest.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::tlog::release::{check_archive, parse_statement};
use una_project::tree::VerifiedTree;

fuzz_target!(|bytes: &[u8]| {
    let mut parts = bytes.splitn(2, |b| *b == 0);
    let statement = String::from_utf8_lossy(parts.next().unwrap_or_default()).into_owned();
    let archive = parts.next().unwrap_or_default();
    let (Ok(statement), Ok(tree)) = (parse_statement(&statement), VerifiedTree::from_archive(archive))
    else {
        return;
    };
    if check_archive(&statement, &tree, None).is_ok() {
        assert_eq!(tree.digest(), statement.digest);
    }
});
