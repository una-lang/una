//! The `registry_owner_replay` fuzz target: the replay of an owner's chain (ADR 0122 rules 38
//! and 38.1, `spec/30 §14` rule 5) over fuzzed entries, cut at each NUL byte, each read as an
//! `ita-owner/1` or `ita-key/1` note and signed by fixed test keys, so the replay is reached past
//! the signatures. Replaying never panics nor loops, and its snapshots are in order.
#![no_main]

use ed25519_dalek::{Signer, SigningKey};
use libfuzzer_sys::fuzz_target;
use una_project::tlog::note::{Key, encode_base64, parse_note};
use una_project::tlog::owner::{Logged, parse_entry, replay};

fn pair(seed: u8, name: &str) -> (SigningKey, Key) {
    let signing = SigningKey::from_bytes(&[seed; 32]);
    let public = signing.verifying_key().to_bytes();
    (
        signing,
        Key {
            name: name.to_owned(),
            public,
        },
    )
}

fuzz_target!(|bytes: &[u8]| {
    let keys = [pair(1, "a"), pair(2, "b"), pair(3, "c"), pair(4, "log")];
    let log = keys[3].1.clone();
    let mut notes = Vec::new();
    let mut entries = Vec::new();
    for (index, text) in bytes.split(|b| *b == 0).take(32).enumerate() {
        let text = String::from_utf8_lossy(text).into_owned();
        let Ok(entry) = parse_entry(&text) else {
            continue;
        };
        let mut document = format!("{text}\n");
        for (signing, key) in &keys {
            let mut line = key.id().to_vec();
            line.extend_from_slice(&signing.sign(text.as_bytes()).to_bytes());
            document.push_str(&format!("\u{2014} {} {}\n", key.name, encode_base64(&line)));
        }
        if let Ok(note) = parse_note(&document) {
            notes.push((index as u64, note));
            entries.push(entry);
        }
    }
    let logged: Vec<Logged<'_>> = notes
        .iter()
        .zip(&entries)
        .map(|((index, note), entry)| Logged {
            index: *index,
            note,
            entry,
        })
        .collect();
    let owner = entries.first().map_or("acme", |e| e.owner.as_str()).to_owned();
    let source = entries.first().map_or("acme", |e| e.source.as_str()).to_owned();
    let mut effect = |index: u64| -> Result<Option<u64>, String> { Ok(Some(index + 2)) };
    if let Ok(history) = replay(&owner, &source, &logged, &log, &mut effect) {
        assert!(history.snapshots.windows(2).all(|w| w[0].from <= w[1].from));
    }
});
