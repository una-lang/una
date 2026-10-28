//! The `registry_root_chain` fuzz target: the reader and the walk of root metadata (ADR 0122
//! rule 35, `spec/30 §11` rule 8), past the signatures: each fuzzed text, cut at NUL bytes, is
//! signed by fixed test keys before it is read, so the fields are reached. Reading and walking
//! never panic, and a chain that is accepted has rising versions and thresholds within its
//! keys.
#![no_main]

use ed25519_dalek::{Signer, SigningKey};
use libfuzzer_sys::fuzz_target;
use una_project::tlog::note::{Key, encode_base64};
use una_project::tlog::root::{digest, trust_first, walk};

fuzz_target!(|bytes: &[u8]| {
    let keys: Vec<(SigningKey, Key)> = (1u8..=3)
        .map(|seed| {
            let signing = SigningKey::from_bytes(&[seed; 32]);
            let public = signing.verifying_key().to_bytes();
            (
                signing,
                Key {
                    name: format!("root.{seed}"),
                    public,
                },
            )
        })
        .collect();
    let documents: Vec<String> = bytes
        .split(|b| *b == 0)
        .take(8)
        .map(|text| {
            let text = String::from_utf8_lossy(text).into_owned();
            let mut document = format!("{text}\n");
            for (signing, key) in &keys {
                let mut line = key.id().to_vec();
                line.extend_from_slice(&signing.sign(text.as_bytes()).to_bytes());
                document.push_str(&format!("\u{2014} {} {}\n", key.name, encode_base64(&line)));
            }
            document
        })
        .collect();
    let Some((first, next)) = documents.split_first() else {
        return;
    };
    let Ok(root) = trust_first(first, &digest(first)) else {
        return;
    };
    let next: Vec<&str> = next.iter().map(String::as_str).collect();
    if let Ok(last) = walk(&root, &next, 0) {
        assert!(last.version >= root.version);
        for set in [&last.root, &last.witnesses, &last.security, &last.itauna] {
            assert!(1 <= set.threshold && set.threshold <= set.keys.len());
        }
    }
});
