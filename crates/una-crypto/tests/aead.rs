//! ChaCha20-Poly1305: RFC 8439 vectors, Wycheproof, differential tests against `ring`.

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
use una_crypto::chacha20poly1305::{open_in_place, seal_in_place};

#[test]
fn wycheproof_chacha20_poly1305() {
    let root = wycheproof("chacha20_poly1305_test.json");
    let (mut valid, mut invalid, mut skipped) = (0, 0, 0);
    for (group, t) in tests_of(&root) {
        let id = t["tcId"].as_u64().unwrap();
        if group["ivSize"].as_u64().unwrap() != 96 {
            // The API takes a 12-byte nonce by type; other sizes cannot be expressed.
            assert_eq!(text(t, "result"), "invalid", "tc {id}");
            skipped += 1;
            continue;
        }
        let key = common::array32(&hex(text(t, "key")));
        let nonce: [u8; 12] = hex(text(t, "iv")).try_into().unwrap();
        let aad = hex(text(t, "aad"));
        let msg = hex(text(t, "msg"));
        let ct = hex(text(t, "ct"));
        let tag = hex(text(t, "tag"));
        let tag16: [u8; 16] = tag.clone().try_into().unwrap();
        match text(t, "result") {
            "valid" => {
                let mut buf = msg.clone();
                let got = seal_in_place(&key, &nonce, &aad, &mut buf).unwrap();
                assert_eq!(buf, ct, "tc {id}");
                assert_eq!(got.to_vec(), tag, "tc {id}");
                let mut buf = ct.clone();
                open_in_place(&key, &nonce, &aad, &mut buf, &tag16).unwrap();
                assert_eq!(buf, msg, "tc {id}");
                valid += 1;
            }
            "invalid" => {
                let mut buf = ct.clone();
                assert!(
                    open_in_place(&key, &nonce, &aad, &mut buf, &tag16).is_err(),
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
        valid > 50 && invalid > 50,
        "{valid} valid, {invalid} invalid"
    );
}

#[test]
fn differential_against_ring_and_tamper_rejection() {
    use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey};
    let mut rng = Rng(0xaead);
    for len in (0..=200).chain([255, 256, 257, 1000, 4096, 65_537]) {
        let key: [u8; 32] = common::array32(&rng.bytes(32));
        let nonce: [u8; 12] = rng.bytes(12).try_into().unwrap();
        let aad = rng.vec(70);
        let msg = rng.bytes(len);

        let theirs = LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, &key).unwrap());
        let mut expected = msg.clone();
        let rtag = theirs
            .seal_in_place_separate_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(&aad),
                &mut expected,
            )
            .unwrap();

        let mut buf = msg.clone();
        let tag = seal_in_place(&key, &nonce, &aad, &mut buf).unwrap();
        assert_eq!(buf, expected, "len {len}");
        assert_eq!(&tag[..], rtag.as_ref(), "len {len}");

        let mut back = buf.clone();
        open_in_place(&key, &nonce, &aad, &mut back, &tag).unwrap();
        assert_eq!(back, msg);

        // Any single flipped bit in the ciphertext, the tag or the associated data fails.
        if !buf.is_empty() {
            let mut bad = buf.clone();
            let i = rng.below(bad.len());
            bad[i] ^= 1 << rng.below(8);
            let before = bad.clone();
            assert!(open_in_place(&key, &nonce, &aad, &mut bad, &tag).is_err());
            assert_eq!(bad, before);
        }
        let mut bad_tag = tag;
        bad_tag[rng.below(16)] ^= 1 << rng.below(8);
        let mut copy = buf.clone();
        assert!(open_in_place(&key, &nonce, &aad, &mut copy, &bad_tag).is_err());
        let mut bad_aad = aad.clone();
        bad_aad.push(0);
        let mut copy = buf.clone();
        assert!(open_in_place(&key, &nonce, &bad_aad, &mut copy, &tag).is_err());
    }
}
