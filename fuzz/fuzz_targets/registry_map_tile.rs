//! The `registry_map_tile` fuzz target: the reader of a tile of the name map (ADR 0122 rule
//! 36.1, `spec/30 §14`), on any bytes, at the tile level and key prefix the first byte gives,
//! and the walk that proves a name's head from tiles. Reading never panics; a tile that reads
//! encodes back to itself, and a walk over any tiles either fails or gives a proof that
//! verifies against the root it walked from.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::tlog::map::{
    TILE_LEVELS, decode_tile, encode_tile, prove_from_tiles, tile_root, verify,
};

fuzz_target!(|bytes: &[u8]| {
    let Some((first, rest)) = bytes.split_first() else {
        return;
    };
    let t = usize::from(*first) % TILE_LEVELS;
    let prefix = vec![*first; t];
    if let Ok(tile) = decode_tile(rest, t, &prefix) {
        assert_eq!(encode_tile(&tile, t), rest);
        // The input as the only tile of a map at its own root: a walk either fails or proves.
        if t == 0 {
            let root = tile_root(&tile.nodes, 0);
            let bytes = rest.to_vec();
            let mut fetch = |path: &str| {
                Ok(path.starts_with("map/0/").then(|| bytes.clone()))
            };
            if let Ok((head, proof)) = prove_from_tiles(&root, "acme/ledger", &mut fetch) {
                assert!(verify(&root, "acme/ledger", head.as_ref(), &proof));
            }
        }
    }
});
