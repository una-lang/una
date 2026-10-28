//! The `registry_map_walk` fuzz target: the walk that proves a name's head from the map's
//! tiles (ADR 0122 rule 36.1, `spec/30 §14`), with every tile it asks for served from the
//! fuzzed input, at every tile level. Walking never panics, and a walk that succeeds gives a
//! proof that verifies against the root it walked from.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::tlog::map::{TILE_LEVELS, prove_from_tiles, verify};

fuzz_target!(|bytes: &[u8]| {
    let Some((root, rest)) = bytes.split_first_chunk::<32>() else {
        return;
    };
    // Tile level t is served the t-th part of the rest, cut at each 0xff byte.
    let parts: Vec<Vec<u8>> = rest.split(|b| *b == 0xff).map(<[u8]>::to_vec).collect();
    let mut fetch = |path: &str| -> Result<Option<Vec<u8>>, String> {
        let level: usize = path
            .strip_prefix("map/")
            .and_then(|rest| rest.split('/').next())
            .and_then(|t| t.parse().ok())
            .unwrap_or(TILE_LEVELS);
        Ok(parts.get(level).cloned())
    };
    for name in ["acme/ledger", "owner:acme", "json"] {
        if let Ok((head, proof)) = prove_from_tiles(root, name, &mut fetch) {
            assert!(verify(root, name, head.as_ref(), &proof));
        }
    }
});
