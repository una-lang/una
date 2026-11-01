//! AES-128/256-GCM: the specification vectors are unit tests of the module; here Wycheproof and
//! differential tests against `ring`.

// Test code: unwrap, indexing and panics are how a test fails (clippy.toml allows them in `#[test]`
// functions only, not in their helpers).
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]

mod common;

use common::{Rng, hex, tests_of, text, wycheproof};
use una_crypto::aes_gcm::AesGcm;

fn cipher(key: &[u8]) -> AesGcm {
    match key.len() {
        16 => AesGcm::new_128(key.try_into().unwrap()),
        32 => AesGcm::new_256(key.try_into().unwrap()),
        n => panic!("key length {n}"),
    }
}

#[test]
fn wycheproof_aes_gcm() {
    let root = wycheproof("aes_gcm_test.json");
    let (mut valid, mut invalid, mut skipped, mut prefix) = (0, 0, 0, 0);
    for (group, t) in tests_of(&root) {
        let id = t["tcId"].as_u64().unwrap();
        let (iv_bits, key_bits, tag_bits) = (
            group["ivSize"].as_u64().unwrap(),
            group["keySize"].as_u64().unwrap(),
            group["tagSize"].as_u64().unwrap(),
        );
        let result = text(t, "result");
        if iv_bits != 96 || tag_bits != 128 || (key_bits != 128 && key_bits != 256) {
            // The typed interface takes the 96-bit nonce and the 128-bit tag of TLS (a valid
            // vector with another nonce size or a truncated tag is outside its contract). A
            // truncated tag must still be a prefix of the tag we compute.
            if iv_bits == 96 && tag_bits < 128 && result == "valid" {
                let g = cipher(&hex(text(t, "key")));
                let nonce: [u8; 12] = hex(text(t, "iv")).try_into().unwrap();
                let mut buf = hex(text(t, "msg"));
                let full = g
                    .seal_in_place(&nonce, &hex(text(t, "aad")), &mut buf)
                    .unwrap();
                assert_eq!(buf, hex(text(t, "ct")), "tc {id}");
                assert_eq!(
                    full[..hex(text(t, "tag")).len()],
                    hex(text(t, "tag"))[..],
                    "tc {id}"
                );
                prefix += 1;
            }
            skipped += 1;
            continue;
        }
        let g = cipher(&hex(text(t, "key")));
        let nonce: [u8; 12] = hex(text(t, "iv")).try_into().unwrap();
        let aad = hex(text(t, "aad"));
        let msg = hex(text(t, "msg"));
        let ct = hex(text(t, "ct"));
        let tag: [u8; 16] = hex(text(t, "tag")).try_into().unwrap();
        match result {
            "valid" => {
                let mut buf = msg.clone();
                let got = g.seal_in_place(&nonce, &aad, &mut buf).unwrap();
                assert_eq!(buf, ct, "tc {id}");
                assert_eq!(got, tag, "tc {id}");
                let mut buf = ct.clone();
                g.open_in_place(&nonce, &aad, &mut buf, &tag).unwrap();
                assert_eq!(buf, msg, "tc {id}");
                valid += 1;
            }
            "invalid" => {
                let mut buf = ct.clone();
                assert!(
                    g.open_in_place(&nonce, &aad, &mut buf, &tag).is_err(),
                    "tc {id}"
                );
                assert_eq!(buf, ct, "buffer must stay untouched on failure, tc {id}");
                invalid += 1;
            }
            other => panic!("unexpected result {other}"),
        }
    }
    assert_eq!(
        valid + invalid + skipped,
        root["numberOfTests"].as_u64().unwrap()
    );
    assert!(
        valid > 70 && invalid > 50,
        "{valid} valid, {invalid} invalid"
    );
    // Skipped: other nonce sizes, 192-bit keys and truncated tags (checked as prefixes).
    assert!(
        skipped > 0 && prefix <= skipped,
        "{skipped} skipped, {prefix} prefixes"
    );
}

#[test]
fn differential_against_ring_and_tamper_rejection() {
    use ring::aead::{AES_128_GCM, AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
    let mut rng = Rng(0xae5);
    for key_len in [16usize, 32] {
        for len in (0..=130).chain([191, 192, 193, 255, 256, 257, 1000, 4096, 65_537]) {
            let key = rng.bytes(key_len);
            let nonce: [u8; 12] = rng.bytes(12).try_into().unwrap();
            let aad = rng.vec(70);
            let msg = rng.bytes(len);

            let alg = if key_len == 16 {
                &AES_128_GCM
            } else {
                &AES_256_GCM
            };
            let theirs = LessSafeKey::new(UnboundKey::new(alg, &key).unwrap());
            let mut expected = msg.clone();
            let rtag = theirs
                .seal_in_place_separate_tag(
                    Nonce::assume_unique_for_key(nonce),
                    Aad::from(&aad),
                    &mut expected,
                )
                .unwrap();

            let g = cipher(&key);
            let mut buf = msg.clone();
            let tag = g.seal_in_place(&nonce, &aad, &mut buf).unwrap();
            assert_eq!(buf, expected, "key {key_len} len {len}");
            assert_eq!(&tag[..], rtag.as_ref(), "key {key_len} len {len}");

            let mut back = buf.clone();
            g.open_in_place(&nonce, &aad, &mut back, &tag).unwrap();
            assert_eq!(back, msg);

            // Any single flipped bit in the ciphertext, the tag or the associated data fails.
            if !buf.is_empty() {
                let mut bad = buf.clone();
                let i = rng.below(bad.len());
                bad[i] ^= 1 << rng.below(8);
                let before = bad.clone();
                assert!(g.open_in_place(&nonce, &aad, &mut bad, &tag).is_err());
                assert_eq!(bad, before);
            }
            let mut bad_tag = tag;
            bad_tag[rng.below(16)] ^= 1 << rng.below(8);
            let mut copy = buf.clone();
            assert!(g.open_in_place(&nonce, &aad, &mut copy, &bad_tag).is_err());
            let mut bad_aad = aad.clone();
            bad_aad.push(0);
            let mut copy = buf.clone();
            assert!(g.open_in_place(&nonce, &bad_aad, &mut copy, &tag).is_err());
            let mut bad_nonce = nonce;
            bad_nonce[rng.below(12)] ^= 1;
            let mut copy = buf.clone();
            assert!(g.open_in_place(&bad_nonce, &aad, &mut copy, &tag).is_err());
        }
    }
}
