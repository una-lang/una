//! ECDSA P-256 and P-384 verification: RFC 6979 Appendix A.2.5 and A.2.6, Wycheproof (DER and
//! IEEE P1363 forms, including SHA-256 on P-384), differential tests against `ring` with mutated
//! signatures, keys equal to plus and minus the generator, and the policy cases.

// Test code: unwrap, indexing and panics are how a test fails (clippy.toml allows them in `#[test]`
// functions only, not in their helpers).
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]

mod common;

use common::{Rng, hex, tests_of, text, to_hex, vector_path, wycheproof};
use ring::rand::SystemRandom;
use ring::signature as rs;
use una_crypto::ecdsa::{Curve, HashAlg, verify_asn1, verify_fixed};

fn hash_of(name: &str) -> HashAlg {
    match name {
        "SHA-256" => HashAlg::Sha256,
        "SHA-384" => HashAlg::Sha384,
        other => panic!("unexpected hash {other}"),
    }
}

fn curve_of(name: &str) -> Curve {
    match name {
        "secp256r1" => Curve::P256,
        "secp384r1" => Curve::P384,
        other => panic!("unexpected curve {other}"),
    }
}

/// Runs one Wycheproof file and returns the number of vectors checked.
fn wycheproof_file(file: &str, fixed: bool) -> usize {
    let root = wycheproof(file);
    let mut ran = 0;
    let mut valid = 0;
    for (group, t) in tests_of(&root) {
        let id = t["tcId"].as_u64().unwrap();
        let curve = curve_of(group["publicKey"]["curve"].as_str().unwrap());
        let hash = hash_of(group["sha"].as_str().unwrap());
        let key = hex(group["publicKey"]["uncompressed"].as_str().unwrap());
        let msg = hex(text(t, "msg"));
        let sig = hex(text(t, "sig"));
        let ok = if fixed {
            verify_fixed(curve, hash, &key, &msg, &sig).is_ok()
        } else {
            verify_asn1(curve, hash, &key, &msg, &sig).is_ok()
        };
        match text(t, "result") {
            "valid" => {
                assert!(ok, "{file} tc {id} should verify");
                valid += 1;
            }
            "invalid" => assert!(!ok, "{file} tc {id} must be rejected: {}", t["comment"]),
            other => panic!("{file} tc {id}: unexpected result {other}"),
        }
        ran += 1;
    }
    assert_eq!(
        ran as u64,
        root["numberOfTests"].as_u64().unwrap(),
        "{file}"
    );
    assert!(valid > 100, "{file}: {valid} valid vectors");
    ran
}

#[test]
fn wycheproof_ecdsa() {
    let mut total = 0;
    total += wycheproof_file("ecdsa_secp256r1_sha256_test.json", false);
    total += wycheproof_file("ecdsa_secp256r1_sha256_p1363_test.json", true);
    total += wycheproof_file("ecdsa_secp384r1_sha384_test.json", false);
    total += wycheproof_file("ecdsa_secp384r1_sha384_p1363_test.json", true);
    total += wycheproof_file("ecdsa_secp384r1_sha256_test.json", false);
    assert_eq!(total, 484 + 262 + 504 + 280 + 472);
}

fn der_of(fixed: &[u8]) -> Vec<u8> {
    fn int(v: &[u8]) -> Vec<u8> {
        let mut v = v;
        while v.len() > 1 && v[0] == 0 {
            v = &v[1..];
        }
        let mut out = vec![0x02];
        if v[0] & 0x80 != 0 {
            out.push(v.len() as u8 + 1);
            out.push(0);
        } else {
            out.push(v.len() as u8);
        }
        out.extend_from_slice(v);
        out
    }
    let (r, s) = fixed.split_at(fixed.len() / 2);
    let body = [int(r), int(s)].concat();
    let mut out = vec![0x30, body.len() as u8];
    out.extend(body);
    out
}

#[test]
fn rfc6979_appendix_a2() {
    let root: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(vector_path("rfc6979_ecdsa.json")).unwrap())
            .unwrap();
    let vectors = root["vectors"].as_array().unwrap();
    assert_eq!(vectors.len(), 8);
    for v in vectors {
        let curve = match v["curve"].as_str().unwrap() {
            "P-256" => Curve::P256,
            _ => Curve::P384,
        };
        let hash = if v["sha"].as_u64().unwrap() == 256 {
            HashAlg::Sha256
        } else {
            HashAlg::Sha384
        };
        let key = hex(&format!("04{}{}", text(v, "ux"), text(v, "uy")));
        let msg = text(v, "msg").as_bytes();
        let fixed = hex(&format!("{}{}", text(v, "r"), text(v, "s")));
        assert!(verify_fixed(curve, hash, &key, msg, &fixed).is_ok(), "{v}");
        assert!(
            verify_asn1(curve, hash, &key, msg, &der_of(&fixed)).is_ok(),
            "{v}"
        );
        // The other message, or a flipped bit of the key or signature, does not verify.
        let other: &[u8] = if msg == b"sample" { b"test" } else { b"sample" };
        assert!(verify_fixed(curve, hash, &key, other, &fixed).is_err());
        let mut bad = fixed.clone();
        bad[5] ^= 0x10;
        assert!(verify_fixed(curve, hash, &key, msg, &bad).is_err());
    }
}

struct Pair {
    curve: Curve,
    hash: HashAlg,
    ring_sign_asn1: &'static rs::EcdsaSigningAlgorithm,
    ring_sign_fixed: &'static rs::EcdsaSigningAlgorithm,
    ring_verify_asn1: &'static rs::EcdsaVerificationAlgorithm,
    ring_verify_fixed: &'static rs::EcdsaVerificationAlgorithm,
}

fn pairs() -> [Pair; 2] {
    [
        Pair {
            curve: Curve::P256,
            hash: HashAlg::Sha256,
            ring_sign_asn1: &rs::ECDSA_P256_SHA256_ASN1_SIGNING,
            ring_sign_fixed: &rs::ECDSA_P256_SHA256_FIXED_SIGNING,
            ring_verify_asn1: &rs::ECDSA_P256_SHA256_ASN1,
            ring_verify_fixed: &rs::ECDSA_P256_SHA256_FIXED,
        },
        Pair {
            curve: Curve::P384,
            hash: HashAlg::Sha384,
            ring_sign_asn1: &rs::ECDSA_P384_SHA384_ASN1_SIGNING,
            ring_sign_fixed: &rs::ECDSA_P384_SHA384_FIXED_SIGNING,
            ring_verify_asn1: &rs::ECDSA_P384_SHA384_ASN1,
            ring_verify_fixed: &rs::ECDSA_P384_SHA384_FIXED,
        },
    ]
}

fn flip(rng: &mut Rng, b: &[u8]) -> Vec<u8> {
    let mut v = b.to_vec();
    let i = rng.below(v.len());
    v[i] ^= 1 << rng.below(8);
    v
}

/// Ours and ring's verdicts on the same input agree, for both signature forms.
fn agree(p: &Pair, key: &[u8], msg: &[u8], asn1: &[u8], fixed: &[u8], what: &str) -> (bool, bool) {
    let ours_a = verify_asn1(p.curve, p.hash, key, msg, asn1).is_ok();
    let ring_a = rs::UnparsedPublicKey::new(p.ring_verify_asn1, key)
        .verify(msg, asn1)
        .is_ok();
    assert_eq!(ours_a, ring_a, "{what}: DER verdicts differ");
    let ours_f = verify_fixed(p.curve, p.hash, key, msg, fixed).is_ok();
    let ring_f = rs::UnparsedPublicKey::new(p.ring_verify_fixed, key)
        .verify(msg, fixed)
        .is_ok();
    assert_eq!(ours_f, ring_f, "{what}: fixed verdicts differ");
    (ours_a, ours_f)
}

#[test]
fn differential_against_ring_with_mutations() {
    use rs::KeyPair;
    let rng = SystemRandom::new();
    let mut prng = Rng(0xec_d5a);
    for p in pairs() {
        for round in 0..12 {
            let pkcs8 = rs::EcdsaKeyPair::generate_pkcs8(p.ring_sign_asn1, &rng).unwrap();
            let kp_a =
                rs::EcdsaKeyPair::from_pkcs8(p.ring_sign_asn1, pkcs8.as_ref(), &rng).unwrap();
            let kp_f =
                rs::EcdsaKeyPair::from_pkcs8(p.ring_sign_fixed, pkcs8.as_ref(), &rng).unwrap();
            let key = kp_a.public_key().as_ref().to_vec();
            let len = if round < 10 {
                round * 17
            } else {
                prng.below(2000)
            };
            let msg = prng.bytes(len);
            let asn1 = kp_a.sign(&rng, &msg).unwrap().as_ref().to_vec();
            let fixed = kp_f.sign(&rng, &msg).unwrap().as_ref().to_vec();
            assert_eq!(agree(&p, &key, &msg, &asn1, &fixed, "honest"), (true, true));
            // Mutations of each part: the verdicts of both libraries agree, and ours accepts
            // only an unmodified triple.
            for _ in 0..12 {
                let bad_key = flip(&mut prng, &key);
                let bad_msg = if msg.is_empty() {
                    prng.bytes(1)
                } else {
                    flip(&mut prng, &msg)
                };
                let bad_asn1 = flip(&mut prng, &asn1);
                let bad_fixed = flip(&mut prng, &fixed);
                assert_eq!(
                    agree(&p, &bad_key, &msg, &asn1, &fixed, "key"),
                    (false, false)
                );
                assert_eq!(
                    agree(&p, &key, &bad_msg, &asn1, &fixed, "message"),
                    (false, false)
                );
                assert!(!agree(&p, &key, &msg, &bad_asn1, &fixed, "der").0, "der");
                assert!(
                    !agree(&p, &key, &msg, &asn1, &bad_fixed, "fixed").1,
                    "fixed"
                );
                // Other forms of damage: truncation and an extra byte.
                let mut short = asn1.clone();
                short.pop();
                let mut long = asn1.clone();
                long.push(0);
                let _ = agree(&p, &key, &msg, &short, &fixed[1..], "short");
                let _ = agree(
                    &p,
                    &key,
                    &msg,
                    &long,
                    &[fixed.as_slice(), &[0]].concat(),
                    "long",
                );
            }
        }
    }
}

#[test]
fn high_s_is_accepted_and_so_is_the_negated_pair() {
    use rs::KeyPair;
    let rng = SystemRandom::new();
    // n for both curves, to compute n - s.
    let orders = [
        "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551",
        "ffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52973",
    ];
    for (p, order) in pairs().into_iter().zip(orders) {
        let pkcs8 = rs::EcdsaKeyPair::generate_pkcs8(p.ring_sign_fixed, &rng).unwrap();
        let kp = rs::EcdsaKeyPair::from_pkcs8(p.ring_sign_fixed, pkcs8.as_ref(), &rng).unwrap();
        let key = kp.public_key().as_ref().to_vec();
        let msg = b"malleable by (r, n - s)";
        let fixed = kp.sign(&rng, msg).unwrap().as_ref().to_vec();
        let len = fixed.len() / 2;
        let n = hex(order);
        // n - s, big-endian.
        let mut neg = vec![0u8; len];
        let mut borrow = 0i32;
        for i in (0..len).rev() {
            let d = i32::from(n[i]) - i32::from(fixed[len + i]) - borrow;
            neg[i] = d.rem_euclid(256) as u8;
            borrow = i32::from(d < 0);
        }
        let mut other = fixed[..len].to_vec();
        other.extend_from_slice(&neg);
        assert!(verify_fixed(p.curve, p.hash, &key, msg, &fixed).is_ok());
        assert!(
            verify_fixed(p.curve, p.hash, &key, msg, &other).is_ok(),
            "the high-s form of a valid signature verifies too"
        );
        assert!(verify_asn1(p.curve, p.hash, &key, msg, &der_of(&other)).is_ok());
        assert!(
            rs::UnparsedPublicKey::new(p.ring_verify_fixed, &key)
                .verify(msg, &other)
                .is_ok()
        );
        // s = 0, r = 0, s = n, r = n, and all-ones are refused.
        let zero = vec![0u8; len];
        let all_ones = vec![0xffu8; len];
        for (r, s) in [
            (&zero, &fixed[len..].to_vec()),
            (&fixed[..len].to_vec(), &zero),
            (&n, &fixed[len..].to_vec()),
            (&fixed[..len].to_vec(), &n),
            (&all_ones, &all_ones),
        ] {
            let sig = [r.as_slice(), s.as_slice()].concat();
            assert!(verify_fixed(p.curve, p.hash, &key, msg, &sig).is_err());
            assert!(verify_asn1(p.curve, p.hash, &key, msg, &der_of(&sig)).is_err());
        }
    }
}

/// A PKCS#8 `PrivateKeyInfo` for an EC key with a chosen scalar and public point.
fn pkcs8(curve: Curve, d: &[u8], public: &[u8]) -> Vec<u8> {
    fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        if body.len() < 128 {
            out.push(body.len() as u8);
        } else {
            out.extend([0x81, body.len() as u8]);
        }
        out.extend_from_slice(body);
        out
    }
    let curve_oid: &[u8] = match curve {
        Curve::P256 => &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07],
        Curve::P384 => &[0x2b, 0x81, 0x04, 0x00, 0x22],
    };
    let alg = tlv(
        0x30,
        &[
            tlv(0x06, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]),
            tlv(0x06, curve_oid),
        ]
        .concat(),
    );
    let bits = tlv(0x03, &[&[0u8][..], public].concat());
    let ec_private = tlv(
        0x30,
        &[tlv(0x02, &[1]), tlv(0x04, d), tlv(0xa1, &bits)].concat(),
    );
    tlv(
        0x30,
        &[tlv(0x02, &[0]), alg, tlv(0x04, &ec_private)].concat(),
    )
}

#[test]
fn keys_equal_to_plus_and_minus_the_generator() {
    use rs::KeyPair;
    let rng = SystemRandom::new();
    // (curve, G, -G, 1, n - 1)
    let cases = [
        (
            Curve::P256,
            "046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5",
            "046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296b01cbd1c01e58065711814b583f061e9d431cca994cea1313449bf97c840ae0a",
            "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632550",
        ),
        (
            Curve::P384,
            "04aa87ca22be8b05378eb1c71ef320ad746e1d3b628ba79b9859f741e082542a385502f25dbf55296c3a545e3872760ab73617de4a96262c6f5d9e98bf9292dc29f8f41dbd289a147ce9da3113b5f0b8c00a60b1ce1d7e819d7a431d7c90ea0e5f",
            "04aa87ca22be8b05378eb1c71ef320ad746e1d3b628ba79b9859f741e082542a385502f25dbf55296c3a545e3872760ab7c9e821b569d9d390a26167406d6d23d6070be242d765eb831625ceec4a0f473ef59f4e30e2817e6285bce2846f15f1a0",
            "ffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52972",
        ),
    ];
    for (p, (curve, g, neg_g, n_minus_1)) in pairs().into_iter().zip(cases) {
        assert_eq!(p.curve, curve);
        let one = {
            let mut d = vec![0u8; curve.scalar_len()];
            *d.last_mut().unwrap() = 1;
            d
        };
        for (d, public) in [(one, hex(g)), (hex(n_minus_1), hex(neg_g))] {
            let der = pkcs8(curve, &d, &public);
            let kp_a = rs::EcdsaKeyPair::from_pkcs8(p.ring_sign_asn1, &der, &rng).unwrap();
            let kp_f = rs::EcdsaKeyPair::from_pkcs8(p.ring_sign_fixed, &der, &rng).unwrap();
            assert_eq!(kp_a.public_key().as_ref(), public.as_slice());
            for n in 0..40 {
                let msg = format!("generator key {n}").into_bytes();
                let asn1 = kp_a.sign(&rng, &msg).unwrap().as_ref().to_vec();
                let fixed = kp_f.sign(&rng, &msg).unwrap().as_ref().to_vec();
                assert!(
                    verify_asn1(curve, p.hash, &public, &msg, &asn1).is_ok(),
                    "{curve:?} {}",
                    to_hex(&d)
                );
                assert!(verify_fixed(curve, p.hash, &public, &msg, &fixed).is_ok());
                let mut bad = fixed.clone();
                bad[3] ^= 1;
                assert!(verify_fixed(curve, p.hash, &public, &msg, &bad).is_err());
            }
        }
    }
}

#[test]
fn key_policy() {
    use rs::KeyPair;
    let rng = SystemRandom::new();
    for p in pairs() {
        let pkcs8 = rs::EcdsaKeyPair::generate_pkcs8(p.ring_sign_fixed, &rng).unwrap();
        let kp = rs::EcdsaKeyPair::from_pkcs8(p.ring_sign_fixed, pkcs8.as_ref(), &rng).unwrap();
        let key = kp.public_key().as_ref().to_vec();
        let msg = b"keys";
        let sig = kp.sign(&rng, msg).unwrap().as_ref().to_vec();
        assert!(verify_fixed(p.curve, p.hash, &key, msg, &sig).is_ok());
        let l = p.curve.scalar_len();
        // Wrong prefix (compressed, hybrid, raw), wrong length, empty, off the curve, a
        // coordinate equal to the field prime or above it, the all-zero point.
        let mut compressed = vec![if key[2 * l] & 1 == 1 { 3 } else { 2 }];
        compressed.extend_from_slice(&key[1..=l]);
        let mut hybrid = key.clone();
        hybrid[0] = 6;
        let mut off_curve = key.clone();
        off_curve[2 * l] ^= 1;
        let mut zero_point = vec![0u8; key.len()];
        zero_point[0] = 4;
        let mut ones = vec![0xffu8; key.len()];
        ones[0] = 4;
        for bad in [
            compressed,
            hybrid,
            key[1..].to_vec(),
            key[..key.len() - 1].to_vec(),
            [key.as_slice(), &[0]].concat(),
            Vec::new(),
            off_curve,
            zero_point,
            ones,
        ] {
            assert!(verify_fixed(p.curve, p.hash, &bad, msg, &sig).is_err());
            assert!(verify_asn1(p.curve, p.hash, &bad, msg, &der_of(&sig)).is_err());
        }
        // A key of the other curve is not on this one.
        let other_curve = match p.curve {
            Curve::P256 => Curve::P384,
            Curve::P384 => Curve::P256,
        };
        assert!(verify_fixed(other_curve, p.hash, &key, msg, &sig).is_err());
    }
}

#[test]
fn der_signature_policy() {
    use rs::KeyPair;
    let rng = SystemRandom::new();
    for p in pairs() {
        let pkcs8 = rs::EcdsaKeyPair::generate_pkcs8(p.ring_sign_asn1, &rng).unwrap();
        let kp = rs::EcdsaKeyPair::from_pkcs8(p.ring_sign_asn1, pkcs8.as_ref(), &rng).unwrap();
        let key = kp.public_key().as_ref().to_vec();
        let msg = b"der";
        let der = kp.sign(&rng, msg).unwrap().as_ref().to_vec();
        assert!(verify_asn1(p.curve, p.hash, &key, msg, &der).is_ok());
        // Not the fixed form, not DER with extra bytes, not BER long-form lengths, not an
        // indefinite length, not a padded integer.
        assert!(verify_asn1(p.curve, p.hash, &key, msg, &[]).is_err());
        assert!(verify_fixed(p.curve, p.hash, &key, msg, &der).is_err());
        let mut trailing = der.clone();
        trailing.push(0);
        assert!(verify_asn1(p.curve, p.hash, &key, msg, &trailing).is_err());
        let mut long_form = vec![0x30, 0x81, der[1]];
        long_form.extend_from_slice(&der[2..]);
        assert!(verify_asn1(p.curve, p.hash, &key, msg, &long_form).is_err());
        let mut indefinite = vec![0x30, 0x80];
        indefinite.extend_from_slice(&der[2..]);
        indefinite.extend_from_slice(&[0, 0]);
        assert!(verify_asn1(p.curve, p.hash, &key, msg, &indefinite).is_err());
        // Pad r with a zero byte (a non-minimal encoding of the same value).
        let r_len = usize::from(der[3]);
        let mut padded = vec![0x30, der[1] + 1, 0x02, der[3] + 1, 0x00];
        padded.extend_from_slice(&der[4..]);
        let _ = r_len;
        assert!(verify_asn1(p.curve, p.hash, &key, msg, &padded).is_err());
    }
}

#[test]
fn a_digest_longer_or_shorter_than_the_order_is_handled() {
    // P-256 with SHA-384 truncates the digest; P-384 with SHA-256 uses it as is. The vectors of
    // RFC 6979 cover both pairings (`rfc6979_appendix_a2`); here, a signature under one pairing
    // never verifies under the other hash.
    let root: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(vector_path("rfc6979_ecdsa.json")).unwrap())
            .unwrap();
    for v in root["vectors"].as_array().unwrap() {
        let curve = if v["curve"] == "P-256" {
            Curve::P256
        } else {
            Curve::P384
        };
        let (hash, wrong) = if v["sha"].as_u64().unwrap() == 256 {
            (HashAlg::Sha256, HashAlg::Sha384)
        } else {
            (HashAlg::Sha384, HashAlg::Sha256)
        };
        let key = hex(&format!("04{}{}", text(v, "ux"), text(v, "uy")));
        let sig = hex(&format!("{}{}", text(v, "r"), text(v, "s")));
        let msg = text(v, "msg").as_bytes();
        assert!(verify_fixed(curve, hash, &key, msg, &sig).is_ok());
        assert!(verify_fixed(curve, wrong, &key, msg, &sig).is_err());
    }
}
