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

#[test]
fn every_vendored_file_matches_its_pinned_digest() {
    let pins = std::fs::read_to_string(vector_path("DIGESTS.sha256")).unwrap();
    let mut checked = 0;
    for line in pins.lines().filter(|l| !l.trim().is_empty()) {
        let (digest, name) = line.split_once("  ").unwrap();
        // The derived files (RFC 6979, NIST CAVP, the test keys) sit beside the Wycheproof
        // directory; everything else is in it.
        let path = if TOP_LEVEL.contains(&name) {
            vector_path(name)
        } else {
            vector_path(&format!("wycheproof/{name}"))
        };
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(to_hex(&Sha256::digest(&bytes)), digest, "{name}");
        assert_eq!(hex(digest).len(), 32);
        checked += 1;
    }
    assert_eq!(checked, 39, "every vendored vector file has a pin");
    let on_disk = std::fs::read_dir(vector_path("wycheproof"))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with("_test.json")
        })
        .count();
    assert_eq!(
        on_disk + TOP_LEVEL.len(),
        checked,
        "a vector file without a pin"
    );
}
