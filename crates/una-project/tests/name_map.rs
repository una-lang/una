//! The name map and per-name chains (ADR 0122 rule 36.1): proofs of presence and absence, a
//! map that hides an entry, a chain that skips one, and the proof's size.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::HashMap;

use una_project::tlog::map::{Head, Link, Map, verify, walk};
use una_project::tlog::merkle::leaf_hash;

fn head(index: u64, count: u64) -> Head {
    Head {
        index,
        leaf: leaf_hash(format!("entry {index}").as_bytes()),
        count,
    }
}

#[test]
fn presence_and_absence_are_proven_against_the_root() {
    let mut map = Map::default();
    for i in 0..50 {
        map.set(&format!("owner{i}/pkg"), &head(i * 10, i + 1));
    }
    let root = map.root();
    for i in 0..50 {
        let name = format!("owner{i}/pkg");
        let proof = map.prove(&name);
        assert!(
            verify(&root, &name, Some(&head(i * 10, i + 1)), &proof),
            "{name}"
        );
        // A stale head, or a lower count, does not verify: a map cannot hide an entry.
        assert!(!verify(&root, &name, Some(&head(i * 10, i)), &proof));
        assert!(!verify(&root, &name, None, &proof));
    }
    let absent = map.prove("nobody/pkg");
    assert!(verify(&root, "nobody/pkg", None, &absent));
    assert!(!verify(&root, "nobody/pkg", Some(&head(1, 1)), &absent));
    // A name whose chain exists cannot be shown absent.
    assert!(!verify(&root, "owner3/pkg", None, &absent));
}

#[test]
fn the_proof_grows_with_the_log_of_the_names() {
    let mut sizes = Vec::new();
    for bits in [6u32, 9, 12] {
        let mut map = Map::default();
        for i in 0..(1u64 << bits) {
            map.set(&format!("n{i}"), &head(i, 1));
        }
        let proof = map.prove("n1");
        assert!(verify(&map.root(), "n1", Some(&head(1, 1)), &proof));
        sizes.push(proof.siblings.len());
    }
    // About log2 M siblings: far from 256, and growing by a few per 8-fold growth.
    assert!(sizes[2] < 36, "{sizes:?}");
    assert!(sizes[0] <= sizes[1] && sizes[1] <= sizes[2], "{sizes:?}");
}

/// A name's chain of `count` entries at indices 10, 20, ...
fn chain(count: u64) -> HashMap<u64, Link> {
    let mut links = HashMap::new();
    let mut previous = None;
    for i in 1..=count {
        let index = i * 10;
        let leaf = leaf_hash(format!("entry {index}").as_bytes());
        links.insert(
            index,
            Link {
                index,
                leaf,
                previous,
            },
        );
        previous = Some((index, leaf));
    }
    links
}

#[test]
fn a_chain_is_walked_to_the_entry_held_and_counted() {
    let links = chain(5);
    let mut fetch = |index: u64| links.get(&index).cloned().ok_or("no entry".to_owned());
    let top = head(50, 5);
    assert_eq!(walk(&top, None, &mut fetch).unwrap().len(), 5);
    assert_eq!(walk(&top, Some(&head(30, 3)), &mut fetch).unwrap().len(), 2);
    // The map counts one entry more than the chain links: an omitted entry.
    assert!(
        walk(&head(50, 6), None, &mut fetch)
            .unwrap_err()
            .contains("missing")
    );
    // Or fewer: an entry the map does not count.
    assert!(walk(&head(50, 4), None, &mut fetch).is_err());
    // A held entry the chain never reaches, and a count that went down.
    assert!(walk(&top, Some(&head(35, 3)), &mut fetch).is_err());
    assert!(walk(&head(30, 3), Some(&head(50, 5)), &mut fetch).is_err());
}

#[test]
fn a_chain_that_skips_an_entry_is_refused() {
    let mut links = chain(5);
    // Entry 40 names 20 as its previous: entry 30, a yank say, is skipped.
    let skipped = links.get(&20).map(|l| (l.index, l.leaf));
    links.get_mut(&40).unwrap().previous = skipped;
    let mut fetch = |index: u64| links.get(&index).cloned().ok_or("no entry".to_owned());
    assert!(walk(&head(50, 5), None, &mut fetch).is_err());
    // A link whose leaf hash is not its successor's record.
    let mut links = chain(3);
    links.get_mut(&20).unwrap().leaf = leaf_hash(b"forged");
    let mut fetch = |index: u64| links.get(&index).cloned().ok_or("no entry".to_owned());
    assert!(walk(&head(30, 3), None, &mut fetch).is_err());
}
