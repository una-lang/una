//! The `registry_map` fuzz target: the reader of a name map proof (ADR 0122 rule 36.1,
//! `spec/30 §14` rule 2) and its verification, on any bytes. Reading never panics; a proof
//! that reads is in its one encoding, and verifying it against any root never panics.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::tlog::map::{encode_proof, parse_proof, verify};

fuzz_target!(|bytes: &[u8]| {
    if let Ok((head, proof)) = parse_proof(bytes) {
        assert_eq!(encode_proof(head.as_ref(), &proof).as_bytes(), bytes);
        let _ = verify(&[0u8; 32], "acme/ledger", head.as_ref(), &proof);
    }
});
