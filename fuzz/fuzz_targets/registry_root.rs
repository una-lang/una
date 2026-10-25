//! The `registry_root` fuzz target: the reader of root metadata and its chain (ADR 0122 rule
//! 35, `spec/30 §11` rule 8), on any text, pinned by its own digest so the parser is reached.
//! Reading never panics, and a document that is accepted has thresholds within its keys.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::tlog::root::{digest, trust_first, walk};

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
    if let Ok(root) = trust_first(&text, &digest(&text)) {
        for keys in [&root.root, &root.witnesses, &root.security, &root.itauna] {
            assert!(1 <= keys.threshold && keys.threshold <= keys.keys.len());
        }
        let _ = walk(&root, &[&text], 0);
    }
});
