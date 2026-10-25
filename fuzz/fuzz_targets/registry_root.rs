//! The `registry_root` fuzz target: the reader of root metadata and its chain (ADR 0122 rule
//! 35, `spec/30 §11` rule 8), on any text, pinned by its own digest so the parser is reached.
//! Reading never panics, and a document that is accepted has thresholds within its keys and
//! revokes, by `witnesses-revoked`, no witness of its own and none twice.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::tlog::root::{digest, trust_first, walk};

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
    if let Ok(root) = trust_first(&text, &digest(&text)) {
        for keys in [&root.root, &root.witnesses, &root.security, &root.itauna] {
            assert!(1 <= keys.threshold && keys.threshold <= keys.keys.len());
        }
        // A revoked witness is never one of the version's witnesses, nor revoked twice.
        for (at, name) in root.witnesses_revoked.iter().enumerate() {
            assert!(root.witnesses.keys.iter().all(|key| key.name != *name));
            assert!(root.witnesses_revoked.iter().take(at).all(|other| other != name));
        }
        let _ = walk(&root, &[&text], 0);
    }
});
