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

#[test]
fn every_vendored_file_matches_its_pinned_digest() {
    let pins = std::fs::read_to_string(vector_path("DIGESTS.sha256")).unwrap();
    let mut checked = 0;
    for line in pins.lines().filter(|l| !l.trim().is_empty()) {
        let (digest, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(vector_path(&format!("wycheproof/{name}"))).unwrap();
        assert_eq!(to_hex(&Sha256::digest(&bytes)), digest, "{name}");
        assert_eq!(hex(digest).len(), 32);
        checked += 1;
    }
    assert_eq!(checked, 10, "every vendored vector file has a pin");
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
    assert_eq!(on_disk, checked, "a vector file without a pin");
}
