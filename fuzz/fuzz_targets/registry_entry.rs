//! The `registry_entry` fuzz target: the reader of a logged entry (ADR 0122 rules 36.1 and 41,
//! `spec/30 §14` rule 4), with its arms for release statements, owners' entries, yanks and
//! advisories, on any text. Reading never panics.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::client::read_entry;

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
    let _ = read_entry(&text);
});
