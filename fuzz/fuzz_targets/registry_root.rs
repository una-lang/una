//! The `registry_root` fuzz target: the reader of root metadata and its chain (ADR 0122 rule
//! 35, `spec/30 §11` rule 8), on any text, pinned by its own digest so the parser is reached.
//! Reading never panics, and a document that is accepted has thresholds within its keys and
//! revokes, by `witnesses-revoked`, no witness of its own and none twice. The bytes after a
//! NUL, when there is one, are a next version walked from the first: a chain walked never
//! lists as a witness a key an earlier version revoked.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::tlog::owner::fingerprint;
use una_project::tlog::root::{digest, trust_first, walk};

fuzz_target!(|bytes: &[u8]| {
    let (first, next) = match bytes.iter().position(|b| *b == 0) {
        Some(at) => {
            let (first, rest) = bytes.split_at(at);
            (first, rest.get(1..))
        }
        None => (bytes, None),
    };
    let text = String::from_utf8_lossy(first);
    if let Ok(root) = trust_first(&text, &digest(&text)) {
        for keys in [&root.root, &root.witnesses, &root.security, &root.itauna] {
            assert!(1 <= keys.threshold && keys.threshold <= keys.keys.len());
        }
        // A revoked witness is never one of the version's witnesses, nor revoked twice.
        for (at, revoked) in root.witnesses_revoked.iter().enumerate() {
            assert_eq!(revoked.len(), 64);
            assert!(revoked.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
            assert!(root.witnesses.keys.iter().all(|key| fingerprint(&key.public) != *revoked));
            assert!(root.witnesses_revoked.iter().take(at).all(|other| other != revoked));
        }
        let _ = walk(&root, &[&text], 0);
        if let Some(next) = next {
            let next = String::from_utf8_lossy(next);
            if let Ok(last) = walk(&root, &[&next], 0) {
                for key in &last.witnesses.keys {
                    assert!(!root.witnesses_revoked.contains(&fingerprint(&key.public)));
                }
            }
        }
    }
});
