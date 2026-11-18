//! HMAC and HKDF: RFC vectors, Wycheproof, differential tests against `ring`.

// Test code: unwrap, indexing and panics are how a test fails (clippy.toml allows them in `#[test]`
// functions only, not in their helpers).
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]

mod common;

use common::{Rng, hex, tests_of, text, to_hex, wycheproof};
use una_crypto::hkdf;
use una_crypto::hmac::{mac_into, verify, verify_truncated};
use una_crypto::sha2::{Hash, Sha256, Sha384, Sha512};

fn mac<H: Hash>(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; H::OUTPUT_LEN];
    mac_into::<H>(key, msg, &mut out).unwrap();
    out
}

#[test]
fn rfc4231_cases() {
    // Test cases 1, 2, 3, 6 (key longer than the block) and 7.
    let k1 = vec![0x0b; 20];
    assert_eq!(
        to_hex(&mac::<Sha256>(&k1, b"Hi There")),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    );
    assert_eq!(
        to_hex(&mac::<Sha512>(&k1, b"Hi There")),
        "87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cdedaa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854"
    );
    assert_eq!(
        to_hex(&mac::<Sha384>(&k1, b"Hi There")),
        "afd03944d84895626b0825f4ab46907f15f9dadbe4101ec682aa034c7cebc59cfaea9ea9076ede7f4af152e8b2fa9cb6"
    );
    assert_eq!(
        to_hex(&mac::<Sha256>(b"Jefe", b"what do ya want for nothing?")),
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
    let k6 = vec![0xaa; 131];
    assert_eq!(
        to_hex(&mac::<Sha256>(
            &k6,
            b"Test Using Larger Than Block-Size Key - Hash Key First"
        )),
        "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
    );
}

#[test]
fn rfc5869_case_1_and_3() {
    let ikm = vec![0x0b; 22];
    let salt = hex("000102030405060708090a0b0c");
    let info = hex("f0f1f2f3f4f5f6f7f8f9");
    let mut prk = [0u8; 32];
    hkdf::extract::<Sha256>(&salt, &ikm, &mut prk).unwrap();
    assert_eq!(
        to_hex(&prk),
        "077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5"
    );
    let mut okm = [0u8; 42];
    hkdf::expand::<Sha256>(&prk, &info, &mut okm).unwrap();
    assert_eq!(
        to_hex(&okm),
        "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
    );
    // Case 3: empty salt and info.
    let mut prk3 = [0u8; 32];
    hkdf::extract::<Sha256>(&[], &ikm, &mut prk3).unwrap();
    let mut okm3 = [0u8; 42];
    hkdf::expand::<Sha256>(&prk3, &[], &mut okm3).unwrap();
    assert_eq!(
        to_hex(&okm3),
        "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d9d201395faa4b61a96c8"
    );
}

#[test]
fn hkdf_length_limit() {
    let prk = [1u8; 32];
    let mut ok = vec![0u8; 255 * 32];
    assert!(hkdf::expand::<Sha256>(&prk, b"i", &mut ok).is_ok());
    let mut too_long = vec![0u8; 255 * 32 + 1];
    assert!(hkdf::expand::<Sha256>(&prk, b"i", &mut too_long).is_err());
    assert!(hkdf::expand::<Sha256>(&prk[..31], b"i", &mut ok[..8]).is_err());
}

fn wycheproof_hmac<H: Hash>(file: &str) {
    let root = wycheproof(file);
    let mut n = 0;
    for (group, t) in tests_of(&root) {
        let key = hex(text(t, "key"));
        let msg = hex(text(t, "msg"));
        let tag = hex(text(t, "tag"));
        let tag_bits = group["tagSize"].as_u64().unwrap() as usize;
        let full = mac::<H>(&key, &msg);
        match text(t, "result") {
            "valid" => {
                assert_eq!(tag.len() * 8, tag_bits);
                assert_eq!(full[..tag.len()], tag[..], "{file} tc {}", t["tcId"]);
                let ok = if tag.len() == H::OUTPUT_LEN {
                    verify::<H>(&key, &msg, &tag)
                } else {
                    // A half-length tag: the plain check refuses it, the opt-in accepts it.
                    assert!(!verify::<H>(&key, &msg, &tag));
                    verify_truncated::<H>(&key, &msg, &tag)
                };
                assert!(ok, "{file} tc {}", t["tcId"]);
            }
            "invalid" => {
                assert!(!verify::<H>(&key, &msg, &tag), "{file} tc {}", t["tcId"]);
                assert!(
                    !verify_truncated::<H>(&key, &msg, &tag),
                    "{file} tc {}",
                    t["tcId"]
                );
            }
            other => panic!("unexpected result {other}"),
        }
        n += 1;
    }
    assert_eq!(n as u64, root["numberOfTests"].as_u64().unwrap());
}

#[test]
fn wycheproof_hmac_sha2() {
    wycheproof_hmac::<Sha256>("hmac_sha256_test.json");
    wycheproof_hmac::<Sha384>("hmac_sha384_test.json");
    wycheproof_hmac::<Sha512>("hmac_sha512_test.json");
}

fn wycheproof_hkdf<H: Hash>(file: &str) {
    let root = wycheproof(file);
    let mut n = 0;
    for (_, t) in tests_of(&root) {
        let ikm = hex(text(t, "ikm"));
        let salt = hex(text(t, "salt"));
        let info = hex(text(t, "info"));
        let size = t["size"].as_u64().unwrap() as usize;
        let mut prk = vec![0u8; H::OUTPUT_LEN];
        hkdf::extract::<H>(&salt, &ikm, &mut prk).unwrap();
        let mut okm = vec![0u8; size];
        let res = hkdf::expand::<H>(&prk, &info, &mut okm);
        match text(t, "result") {
            "valid" => {
                res.unwrap();
                assert_eq!(to_hex(&okm), text(t, "okm"), "{file} tc {}", t["tcId"]);
            }
            "invalid" => assert!(res.is_err(), "{file} tc {}", t["tcId"]),
            other => panic!("unexpected result {other}"),
        }
        n += 1;
    }
    assert_eq!(n as u64, root["numberOfTests"].as_u64().unwrap());
}

#[test]
fn wycheproof_hkdf_sha2() {
    wycheproof_hkdf::<Sha256>("hkdf_sha256_test.json");
    wycheproof_hkdf::<Sha384>("hkdf_sha384_test.json");
    wycheproof_hkdf::<Sha512>("hkdf_sha512_test.json");
}

#[test]
fn differential_against_ring() {
    use ring::{hkdf as rh, hmac as rm};
    let mut rng = Rng(0xfeed);
    for _ in 0..400 {
        let key = rng.vec(300);
        let msg = rng.vec(600);
        for (alg, ours) in [
            (rm::HMAC_SHA256, mac::<Sha256>(&key, &msg)),
            (rm::HMAC_SHA384, mac::<Sha384>(&key, &msg)),
            (rm::HMAC_SHA512, mac::<Sha512>(&key, &msg)),
        ] {
            let k = rm::Key::new(alg, &key);
            assert_eq!(rm::sign(&k, &msg).as_ref(), &ours[..]);
        }
    }
    struct Len(usize);
    impl rh::KeyType for Len {
        fn len(&self) -> usize {
            self.0
        }
    }
    for _ in 0..200 {
        let ikm = rng.vec(80);
        let salt_b = rng.vec(80);
        let info = rng.vec(80);
        let len = rng.below(255 * 32);
        let salt = rh::Salt::new(rh::HKDF_SHA256, &salt_b);
        let prk_r = salt.extract(&ikm);
        let mut theirs = vec![0u8; len];
        prk_r
            .expand(&[&info], Len(len))
            .unwrap()
            .fill(&mut theirs)
            .unwrap();
        let mut prk = [0u8; 32];
        hkdf::extract::<Sha256>(&salt_b, &ikm, &mut prk).unwrap();
        let mut ours = vec![0u8; len];
        hkdf::expand::<Sha256>(&prk, &info, &mut ours).unwrap();
        assert_eq!(ours, theirs);
    }
}

#[test]
fn tag_and_slice_lengths_are_checked() {
    let key = [7u8; 20];
    let full = mac::<Sha256>(&key, b"m");
    assert!(verify::<Sha256>(&key, b"m", &full));
    for n in [0, 1, 4, 15, 31] {
        assert!(!verify::<Sha256>(&key, b"m", &full[..n]), "plain {n}");
    }
    assert!(verify_truncated::<Sha256>(&key, b"m", &full[..16]));
    for n in [0, 4, 15] {
        assert!(
            !verify_truncated::<Sha256>(&key, b"m", &full[..n]),
            "truncated {n}"
        );
    }
    let mut longer = full.clone();
    longer.push(0);
    assert!(!verify::<Sha256>(&key, b"m", &longer));
    assert!(!verify_truncated::<Sha256>(&key, b"m", &longer));
    // Output slices of the wrong length are refused and left untouched.
    let mut wrong = [0xaau8; 31];
    assert!(mac_into::<Sha256>(&key, b"m", &mut wrong).is_err());
    assert_eq!(wrong, [0xaau8; 31]);
    let mut out = [0u8; 33];
    assert!(hkdf::extract::<Sha256>(b"s", b"k", &mut out).is_err());
    assert!(hkdf::extract::<Sha256>(b"s", b"k", &mut out[..31]).is_err());
    let mut h = Sha256::new();
    h.update(b"x");
    assert!(h.finalize_into(&mut out).is_err());
    // expand rejects a pseudorandom key of the wrong length, shorter or longer.
    let mut okm = [0u8; 8];
    assert!(hkdf::expand::<Sha256>(&[1u8; 31], b"", &mut okm).is_err());
    assert!(hkdf::expand::<Sha256>(&[1u8; 33], b"", &mut okm).is_err());
    assert!(hkdf::expand::<Sha256>(&[1u8; 32], b"", &mut okm).is_ok());
}
