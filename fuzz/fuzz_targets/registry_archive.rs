//! The `registry_archive` fuzz target: the reader of a package archive (ADR 0122 rule 29,
//! `spec/30 §9` rule 3), the bytes a registry serves at `blobs/sha256/<hex>`, on any bytes.
//! Reading never panics; an archive that reads lists files a package may hold, and its digest
//! is the one its files give.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::content::digest_of;
use una_project::tree::VerifiedTree;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(tree) = VerifiedTree::from_archive(bytes) {
        let mut files = tree.files().to_vec();
        assert_eq!(digest_of(&mut files).as_deref(), Ok(tree.digest()));
    }
});
