//! The `provenance` fuzz target (#997): the reader of a provenance document (ADR 0122 rules 17
//! and 30), which reads what a vendor directory or Itaoca holds, on any bytes. It must never
//! panic, and a document it reads has one encoding: encoding it again gives the same bytes,
//! which read back to the same document.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::provenance;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(read) = provenance::parse(bytes) {
        let encoded = read.encode();
        assert_eq!(encoded.as_slice(), bytes);
        assert_eq!(provenance::parse(&encoded).as_ref(), Ok(&read));
    }
});
