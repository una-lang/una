//! The vendored Wycheproof files are the ones whose digests are pinned (ADR 0133 rule 11).

// Test code: unwrap, indexing and panics are how a test fails (clippy.toml allows them in `#[test]`
// functions only, not in their helpers).
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]

mod common;

use common::{hex, to_hex, vector_path};
use una_crypto::sha2::Sha256;

/// The pinned files that are not Wycheproof files.
const TOP_LEVEL: [&str; 3] = [
    "rfc6979_ecdsa.json",
    "nist_cavp_rsa.json",
    "rsa_test_keys.json",
];

/// The result of checking a set of pin lines against a directory listing of vector files.
#[derive(Debug, PartialEq, Eq)]
enum Problem {
    /// A pinned file is missing or its digest differs.
    Pinned(String),
    /// A vector file on disk has no pin.
    Unpinned(String),
    /// Pin lines and files do not match in number.
    Count { pinned: usize, on_disk: usize },
}

/// Checks `pins` (the lines of `DIGESTS.sha256`) against the files `read` can give and the file
/// names `on_disk` lists (Wycheproof files plus [`TOP_LEVEL`]). The real test and the negative
/// tests below run this one function.
fn check_pins(
    pins: &str,
    read: &dyn Fn(&str) -> Option<Vec<u8>>,
    on_disk: &[String],
) -> Vec<Problem> {
    let mut problems = Vec::new();
    let mut pinned = Vec::new();
    for line in pins.lines().filter(|l| !l.trim().is_empty()) {
        let Some((digest, name)) = line.split_once("  ") else {
            problems.push(Problem::Pinned(line.to_owned()));
            continue;
        };
        pinned.push(name.to_owned());
        match read(name) {
            Some(bytes) if to_hex(&Sha256::digest(&bytes)) == digest && hex(digest).len() == 32 => {
            }
            _ => problems.push(Problem::Pinned(name.to_owned())),
        }
    }
    for name in on_disk {
        if !pinned.contains(name) {
            problems.push(Problem::Unpinned(name.clone()));
        }
    }
    if pinned.len() != on_disk.len() {
        problems.push(Problem::Count {
            pinned: pinned.len(),
            on_disk: on_disk.len(),
        });
    }
    problems
}

fn real_path(name: &str) -> std::path::PathBuf {
    // The derived files (RFC 6979, NIST CAVP, the test keys) sit beside the Wycheproof directory;
    // everything else is in it.
    if TOP_LEVEL.contains(&name) {
        vector_path(name)
    } else {
        vector_path(&format!("wycheproof/{name}"))
    }
}

fn real_listing() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(vector_path("wycheproof"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with("_test.json"))
        .collect();
    names.extend(TOP_LEVEL.iter().map(|n| (*n).to_owned()));
    names
}

#[test]
fn every_vendored_file_matches_its_pinned_digest() {
    let pins = std::fs::read_to_string(vector_path("DIGESTS.sha256")).unwrap();
    let read = |name: &str| std::fs::read(real_path(name)).ok();
    let listing = real_listing();
    assert_eq!(check_pins(&pins, &read, &listing), Vec::<Problem>::new());
    assert_eq!(listing.len(), 39, "every vendored vector file has a pin");
}

#[test]
fn a_deleted_unpinned_or_altered_vector_file_is_a_failure() {
    let pins = std::fs::read_to_string(vector_path("DIGESTS.sha256")).unwrap();
    let read = |name: &str| std::fs::read(real_path(name)).ok();
    let listing = real_listing();
    let victim = listing
        .iter()
        .find(|n| n.starts_with("aes_gcm"))
        .unwrap()
        .clone();

    // A pinned file that is deleted (the reader finds nothing).
    let deleted = |name: &str| if name == victim { None } else { read(name) };
    assert_eq!(
        check_pins(&pins, &deleted, &listing),
        [Problem::Pinned(victim.clone())]
    );

    // A pinned file whose bytes changed by one bit.
    let altered = |name: &str| {
        let mut bytes = read(name)?;
        if name == victim {
            *bytes.first_mut()? ^= 1;
        }
        Some(bytes)
    };
    assert_eq!(
        check_pins(&pins, &altered, &listing),
        [Problem::Pinned(victim.clone())]
    );

    // A pin line removed while the file stays on disk (unpinned).
    let without: String = pins
        .lines()
        .filter(|l| !l.ends_with(&format!("  {victim}")))
        .map(|l| format!("{l}\n"))
        .collect();
    assert_eq!(
        check_pins(&without, &read, &listing),
        [
            Problem::Unpinned(victim.clone()),
            Problem::Count {
                pinned: listing.len() - 1,
                on_disk: listing.len()
            }
        ]
    );

    // A new vector file on disk that has no pin.
    let mut extra = listing.clone();
    extra.push("new_test.json".to_owned());
    assert_eq!(
        check_pins(&pins, &read, &extra),
        [
            Problem::Unpinned("new_test.json".to_owned()),
            Problem::Count {
                pinned: listing.len(),
                on_disk: listing.len() + 1
            }
        ]
    );

    // A pin line that is not "digest  name".
    assert_eq!(
        check_pins("garbage\n", &read, &[]),
        [Problem::Pinned("garbage".to_owned())]
    );
}
