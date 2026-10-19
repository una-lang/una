//! The verification of a registry's documents (ADR 0122 rules 35 to 39): Merkle proofs and their
//! scaling, strict Ed25519, signed notes, checkpoints, cosignatures and witness time.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::HashMap;

use pretty_assertions::assert_eq;
use ring::signature::{Ed25519KeyPair, KeyPair};
use una_project::tlog::ed25519::{Refusal, verify};
use una_project::tlog::merkle::{
    Hash, consistency_proof, inclusion_proof, leaf_hash, node_hash, root, verify_consistency,
    verify_inclusion,
};
use una_project::tlog::note::{
    Key, base64, encode_base64, parse_checkpoint, parse_note,
};
use una_project::tlog::witness::{
    Cosigned, Problem, checkpoint_time, fresh, logged_at, monotonic, waited,
};

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn leaves(n: usize) -> Vec<Hash> {
    (0..n).map(|i| leaf_hash(format!("entry {i}").as_bytes())).collect()
}

#[test]
fn inclusion_proofs_verify_for_every_leaf_and_no_other() {
    for n in 1..=33 {
        let leaves = leaves(n);
        let top = root(&leaves);
        for index in 0..n {
            let proof = inclusion_proof(index, &leaves);
            assert!(verify_inclusion(index as u64, n as u64, &leaves[index], &proof, &top));
            // Another leaf, or a changed hash of the proof, fails.
            let other = leaf_hash(b"other");
            assert!(!verify_inclusion(index as u64, n as u64, &other, &proof, &top));
            if let Some(first) = proof.first() {
                let mut bad = proof.clone();
                bad[0] = node_hash(first, first);
                assert!(!verify_inclusion(index as u64, n as u64, &leaves[index], &bad, &top));
            }
        }
        assert!(!verify_inclusion(n as u64, n as u64, &leaves[0], &[], &top));
    }
}

#[test]
fn consistency_proofs_verify_for_every_prefix_and_catch_a_fork() {
    for n in 1..=33 {
        let leaves = leaves(n);
        let top = root(&leaves);
        for m in 0..=n {
            let proof = consistency_proof(m, &leaves);
            let prefix = root(&leaves[..m]);
            assert!(
                verify_consistency(m as u64, n as u64, &prefix, &top, &proof),
                "{m} -> {n}"
            );
            if 0 < m && m < n {
                // A fork: the same size with one entry rewritten.
                let mut forked = leaves.clone();
                forked[m - 1] = leaf_hash(b"rewritten");
                let forked_prefix = root(&forked[..m]);
                assert!(!verify_consistency(m as u64, n as u64, &forked_prefix, &top, &proof));
            }
        }
        assert!(!verify_consistency(n as u64 + 1, n as u64, &top, &top, &[]));
    }
}

/// The root of a tree of `n` identical leaves of hash `leaf`, by the recursion of RFC 9162
/// with a memo, O(log n).
fn same_root(n: u64, leaf: &Hash, memo: &mut HashMap<u64, Hash>) -> Hash {
    if n == 1 {
        return *leaf;
    }
    if let Some(hash) = memo.get(&n) {
        return *hash;
    }
    let mut k = 1;
    while k << 1 < n {
        k <<= 1;
    }
    let hash = node_hash(&same_root(k, leaf, memo), &same_root(n - k, leaf, memo));
    memo.insert(n, hash);
    hash
}

/// The inclusion proof of `index` in a tree of `n` identical leaves.
fn same_proof(index: u64, n: u64, leaf: &Hash, memo: &mut HashMap<u64, Hash>) -> Vec<Hash> {
    if n <= 1 {
        return Vec::new();
    }
    let mut k = 1;
    while k << 1 < n {
        k <<= 1;
    }
    if index < k {
        let mut proof = same_proof(index, k, leaf, memo);
        proof.push(same_root(n - k, leaf, memo));
        proof
    } else {
        let mut proof = same_proof(index - k, n - k, leaf, memo);
        proof.push(same_root(k, leaf, memo));
        proof
    }
}

/// Proofs on logs of 2^10 to 2^30 leaves: the work, one hash per element of the proof, grows by
/// one hash per doubling (ADR 0122, Costs).
#[test]
fn inclusion_proofs_grow_by_one_hash_per_doubling() {
    let leaf = leaf_hash(b"x");
    let mut memo = HashMap::new();
    let mut lengths = Vec::new();
    for bits in 10..=30u32 {
        let n = 1u64 << bits;
        let index = n / 3;
        let proof = same_proof(index, n, &leaf, &mut memo);
        let top = same_root(n, &leaf, &mut memo);
        assert!(verify_inclusion(index, n, &leaf, &proof, &top));
        lengths.push(proof.len());
    }
    for pair in lengths.windows(2) {
        assert_eq!(pair[1], pair[0] + 1);
    }
    assert_eq!(lengths[0], 10);
}

#[test]
fn ed25519_follows_rfc_8032_strictly() {
    let key = hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
    let signature = hex("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b");
    assert_eq!(verify(&key, b"", &signature), Ok(()));
    assert_eq!(verify(&key, b"x", &signature), Err(Refusal::Equation));
    // S + L, the same signature with a non-canonical S, is refused before the equation.
    let mut s_plus_l = signature.clone();
    let order = hex("edd3f55c1a631258d69cf7a2def9de1400000000000000000000000000000010");
    let mut carry = 0u16;
    for i in 0..32 {
        let total = u16::from(s_plus_l[32 + i]) + u16::from(order[i]) + carry;
        s_plus_l[32 + i] = (total & 0xff) as u8;
        carry = total >> 8;
    }
    assert_eq!(verify(&key, b"", &s_plus_l), Err(Refusal::S));
    // A small-order key or R.
    let mut identity = [0u8; 32];
    identity[0] = 1;
    assert_eq!(verify(&identity, b"", &signature), Err(Refusal::Key));
    let mut small_r = signature.clone();
    small_r[..32].copy_from_slice(&identity);
    assert_eq!(verify(&key, b"", &small_r), Err(Refusal::R));
    assert_eq!(verify(&key[..31], b"", &signature), Err(Refusal::Length));
    assert_eq!(verify(&key, b"", &signature[..63]), Err(Refusal::Length));
}

fn keypair(seed: u8, name: &str) -> (Ed25519KeyPair, Key) {
    let pair = Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap();
    let public: [u8; 32] = pair.public_key().as_ref().try_into().unwrap();
    (
        pair,
        Key {
            name: name.to_owned(),
            public,
        },
    )
}

fn sign_line(pair: &Ed25519KeyPair, key: &Key, text: &str) -> String {
    let mut bytes = key.id().to_vec();
    bytes.extend_from_slice(pair.sign(text.as_bytes()).as_ref());
    format!("\u{2014} {} {}\n", key.name, encode_base64(&bytes))
}

fn cosign_line(pair: &Ed25519KeyPair, key: &Key, text: &str, time: u64) -> String {
    let message = format!("cosignature/v1\ntime {time}\n{text}");
    let mut bytes = key.cosignature_id().to_vec();
    bytes.extend_from_slice(&time.to_be_bytes());
    bytes.extend_from_slice(pair.sign(message.as_bytes()).as_ref());
    format!("\u{2014} {} {}\n", key.name, encode_base64(&bytes))
}

#[test]
fn signed_notes_and_checkpoints() {
    let (log_pair, log) = keypair(1, "itaty.test/log");
    let (_, other) = keypair(2, "itaty.test/log");
    let (w_pair, witness) = keypair(3, "witness.test");
    let top = root(&leaves(5));
    let text = format!("itaty.test/log\n5\n{}\n", encode_base64(&top));
    let document = format!(
        "{text}\n{}{}",
        sign_line(&log_pair, &log, &text),
        cosign_line(&w_pair, &witness, &text, 1_700_000_000)
    );
    let note = parse_note(&document).unwrap();
    assert!(note.signed_by(&log));
    assert!(!note.signed_by(&other), "another key of the same name");
    assert_eq!(note.signers(&[log.clone(), log.clone(), other]), 1);
    assert_eq!(note.cosignature(&witness), Some(1_700_000_000));
    assert_eq!(note.cosignature(&log), None);
    let checkpoint = parse_checkpoint(&note.text).unwrap();
    assert_eq!(checkpoint.size, 5);
    assert_eq!(checkpoint.root, top);
    // A changed text no longer verifies.
    let tampered = parse_note(&document.replacen("\n5\n", "\n6\n", 1)).unwrap();
    assert!(!tampered.signed_by(&log));
    assert_eq!(tampered.cosignature(&witness), None);
    for bad in [
        "a\n5\n",
        "itaty.test/log\n05\nAAAA\n",
        "itaty.test/log\n5\nnot base64\n",
        "itaty.test/log\n5\nAAAA\n",
    ] {
        assert!(parse_checkpoint(bad).is_err(), "{bad:?}");
    }
    assert!(parse_note("text\n").is_err());
    assert!(parse_note("text\n\n- name AAAAAA==\n").is_err());
    assert!(parse_note("te\u{7}xt\n\n\u{2014} name AAAAAAAA\n").is_err());
}

#[test]
fn base64_is_canonical() {
    for bytes in [&b""[..], b"A", b"AB", b"ABC", b"ABCD"] {
        assert_eq!(base64(&encode_base64(bytes)).unwrap(), bytes);
    }
    assert_eq!(base64("QQ=="), Some(b"A".to_vec()));
    assert_eq!(base64("QR=="), None, "unused bits set");
    assert_eq!(base64("QQ="), None);
    assert_eq!(base64("Q==="), None);
    assert_eq!(base64("QQ==QQ=="), None);
    assert_eq!(base64("QQ-_"), None);
}

fn c(witness: &str, size: u64, time: u64) -> Cosigned {
    Cosigned {
        witness: witness.to_owned(),
        size,
        time,
    }
}

#[test]
fn witness_time_resists_fewer_than_the_threshold() {
    // Three of five witnesses; one back-dates its cosignatures of the entry's first checkpoint.
    let seen = [
        c("a", 10, 1000),
        c("b", 10, 1001),
        c("c", 10, 1002),
        c("d", 10, 1003),
        c("liar", 10, 1),
        c("a", 20, 2000),
    ];
    assert_eq!(logged_at(5, &seen, 3), Ok(1001));
    // An entry the first checkpoint does not include is logged by the later ones only.
    assert_eq!(
        logged_at(15, &seen, 1),
        Ok(2000)
    );
    assert_eq!(
        logged_at(15, &seen, 3),
        Err(Problem::TooFew { found: 1, needed: 3 })
    );
    // One forward-dating witness cannot make a checkpoint look later.
    let checkpoint = [
        c("a", 30, 5000),
        c("b", 30, 5001),
        c("c", 30, 5002),
        c("liar", 30, 999_999),
    ];
    assert_eq!(checkpoint_time(&checkpoint, 3), Ok(5001));
    assert_eq!(waited(1001, 5001), 4000);
    assert_eq!(waited(5001, 1001), 0);
    assert_eq!(
        monotonic(&[c("a", 10, 100), c("a", 20, 50)]),
        Err(Problem::Decreasing {
            witness: "a".to_owned()
        })
    );
    assert_eq!(monotonic(&seen), Ok(()));
    assert_eq!(fresh(5001, 5001 + 3600, 86_400, 3600), Ok(true));
    assert_eq!(fresh(5001, 5001 + 90_000, 86_400, 3600), Ok(false));
    assert_eq!(fresh(10_000, 5000, 86_400, 3600), Err(5000));
}

const STATEMENT: &str = "ita-release/1
source itaty
name acme/ledger
version 0.3.1
previous 47755 sha256:aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04
digest sha256:3b1f3b1f3b1f3b1f3b1f3b1f3b1f3b1f3b1f3b1f3b1f3b1f3b1f3b1f3b1f3b1f
manifest sha256:77aa77aa77aa77aa77aa77aa77aa77aa77aa77aa77aa77aa77aa77aa77aa77aa
edition 0.4
needs itaty json 2.1.0
needs itaty text_utf8 1.0.3
authority Http[get], ledger.store.Store[find, put]
trust modules ledger.keys
";

#[test]
fn release_statements_parse_strictly() {
    use una_project::tlog::release::{parse_statement, within};
    let statement = parse_statement(STATEMENT).unwrap();
    assert_eq!(statement.name, "acme/ledger");
    assert_eq!(statement.previous.as_ref().unwrap().index, 47755);
    assert_eq!(statement.needs.len(), 2);
    assert_eq!(statement.authority.len(), 2);
    assert!(statement.trust.contains("ledger.keys"));
    let first = STATEMENT.replacen(
        "previous 47755 sha256:aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04",
        "previous none",
        1,
    );
    assert!(parse_statement(&first).unwrap().previous.is_none());
    for (from, to) in [
        ("ita-release/1", "ita-release/2"),
        ("needs itaty json", "needs other json"),
        ("needs itaty text_utf8 1.0.3\n", ""),
        ("Http[get]", "Http[get, get]"),
        ("trust modules ledger.keys\n", "trust modules ledger.keys\nextra\n"),
        ("version 0.3.1", "version 0.3"),
        ("digest sha256:3b1f", "digest sha256:XX1f"),
    ] {
        let changed = STATEMENT.replacen(from, to, 1);
        if from == "needs itaty text_utf8 1.0.3\n" {
            // One need fewer is still a statement.
            assert!(parse_statement(&changed).is_ok());
            continue;
        }
        assert!(parse_statement(&changed).is_err(), "{from} -> {to}");
    }
    // Rule 26: authority and trust may only shrink within an identity.
    let wider = parse_statement(&STATEMENT.replacen("Http[get]", "Http[get, post]", 1)).unwrap();
    assert!(within(&statement, &wider).is_ok());
    assert!(within(&wider, &statement).unwrap_err().contains("Http[post]"));
    let mailer = parse_statement(&STATEMENT.replacen(
        "authority Http[get], ledger.store.Store[find, put]",
        "authority Mail[send]",
        1,
    ))
    .unwrap();
    assert!(within(&mailer, &statement).unwrap_err().contains("`Mail`"));
    let trusting = parse_statement(&STATEMENT.replacen("ledger.keys", "ledger.keys, ledger.more", 1)).unwrap();
    assert!(within(&trusting, &statement).unwrap_err().contains("ledger.more"));
    let none = parse_statement(&STATEMENT.replacen(
        "authority Http[get], ledger.store.Store[find, put]",
        "authority none",
        1,
    ))
    .unwrap();
    assert!(none.authority.is_empty());
}
