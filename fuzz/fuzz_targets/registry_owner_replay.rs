//! The `registry_owner_replay` fuzz target: the replay of an owner's chain (ADR 0122
//! Amendment 14, `spec/30 §14` rule 5) over fuzzed entries. The input is cut at each NUL byte;
//! each part's first byte chooses which of four fixed test keys sign it (a bit per key) and the
//! rest is its text, read as an `ita-owner/1` or `ita-key/1` note. The input's first byte
//! chooses the gap between log indices (1, 2 or 3), so that a claim's `since` can name a
//! position between two entries. Every entry is signed twice, its signature lines in one order
//! and in the reverse order.
//!
//! What it asserts: replaying never panics nor loops; its snapshots are in order and name every
//! entry; the result does not depend on the order of the signature lines; and the chain is
//! [`ReplayError::Unsupported`] exactly when it holds an entry of a kind this release does not
//! replay and no entry before the first such one is unacceptable, at that entry's index (an
//! unacceptable entry before it is [`ReplayError::Unacceptable`] at its own, earlier, index).
#![no_main]

use ed25519_dalek::{Signer, SigningKey};
use libfuzzer_sys::fuzz_target;
use una_project::tlog::note::{Key, Note, encode_base64, parse_note};
use una_project::tlog::owner::{Entry, Logged, ReplayError, parse_entry, replay, unreplayed};

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

/// The note of `text` signed by the keys of `mask`, its signature lines in key order or in the
/// reverse order.
fn signed(text: &str, mask: u8, keys: &[(SigningKey, Key)], reversed: bool) -> Option<Note> {
    let mut lines: Vec<String> = keys
        .iter()
        .enumerate()
        .filter(|(bit, _)| mask >> bit & 1 == 1)
        .map(|(_, (signing, key))| {
            let mut line = key.id().to_vec();
            line.extend_from_slice(&signing.sign(text.as_bytes()).to_bytes());
            format!("\u{2014} {} {}\n", key.name, encode_base64(&line))
        })
        .collect();
    if reversed {
        lines.reverse();
    }
    parse_note(&format!("{text}\n{}", lines.concat())).ok()
}

/// The chain of `entries` at `indices`, each with its note of `notes`.
fn logged<'a>(indices: &[u64], notes: Vec<&'a Note>, entries: &'a [Entry]) -> Vec<Logged<'a>> {
    indices
        .iter()
        .zip(notes)
        .zip(entries)
        .map(|((index, note), entry)| Logged {
            index: *index,
            note,
            entry,
        })
        .collect()
}

fuzz_target!(|bytes: &[u8]| {
    let Some((mode, rest)) = bytes.split_first() else {
        return;
    };
    let gap = u64::from(mode % 3) + 1;
    let keys = [pair(1, "a"), pair(2, "b"), pair(3, "c"), pair(4, "d")];
    let mut forward: Vec<(u64, Note)> = Vec::new();
    let mut backward: Vec<Note> = Vec::new();
    let mut entries: Vec<Entry> = Vec::new();
    for (index, part) in rest.split(|b| *b == 0).take(32).enumerate() {
        let Some((mask, text)) = part.split_first() else {
            continue;
        };
        let text = String::from_utf8_lossy(text).into_owned();
        let Ok(entry) = parse_entry(&text) else {
            continue;
        };
        let (Some(one), Some(other)) = (
            signed(&text, *mask, &keys, false),
            signed(&text, *mask, &keys, true),
        ) else {
            continue;
        };
        forward.push((index as u64 * gap, one));
        backward.push(other);
        entries.push(entry);
    }
    let indices: Vec<u64> = forward.iter().map(|(index, _)| *index).collect();
    let first = logged(
        &indices,
        forward.iter().map(|(_, note)| note).collect(),
        &entries,
    );
    let second = logged(&indices, backward.iter().collect(), &entries);
    let owner = entries.first().map_or("acme", |e| e.owner.as_str());
    let source = entries.first().map_or("acme", |e| e.source.as_str());
    let result = replay(owner, source, &first);
    assert_eq!(
        result,
        replay(owner, source, &second),
        "signature-line order"
    );
    let same = entries
        .iter()
        .all(|e| e.owner == owner && e.source == source);
    let unsupported = first
        .iter()
        .find(|l| unreplayed(&l.entry.event).is_some())
        .map(|l| l.index);
    match (&result, unsupported) {
        (Ok(history), None) => {
            assert!(history.snapshots.windows(2).all(|w| w[0].from < w[1].from));
            assert_eq!(history.entries, indices);
        }
        (Ok(_), Some(_)) => panic!("a chain with an unreplayed entry replayed"),
        (Err(ReplayError::Unsupported(u)), Some(at)) => assert_eq!(u.index, at),
        (Err(ReplayError::Unsupported(_)), None) => panic!("unsupported without such an entry"),
        (Err(ReplayError::Unacceptable(bad)), at) => {
            assert!(
                at.is_none_or(|at| bad.index < at),
                "read past an unreplayed entry"
            );
        }
        (Err(ReplayError::Contested(_)), at) => {
            assert!(at.is_none(), "contested past an unreplayed entry");
        }
        (Err(ReplayError::Refused(_)), _) => assert!(!same, "refused one chain in order"),
    }
});
