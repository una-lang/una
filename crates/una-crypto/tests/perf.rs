//! Performance and scaling sanity (ADR 0133 rule 10): linear time in the message size, and a
//! floor on throughput so a quadratic or accidental slow path is caught. Optimized builds only;
//! the bounds are loose (an order of magnitude) because the machine is shared. The ordinary test
//! run is a debug build, where each test returns at once; they measure in the release gate
//! `timing-channels` of `scripts/release-gates.sh` and in the step of the same name of
//! `scripts/local-ci.sh`, with `cargo test --release --locked -p una-crypto --test perf --
//! --test-threads=1 --nocapture` (#2119, `docs/release-gates.md`).

// Test code: unwrap, indexing and panics are how a test fails (clippy.toml allows them in `#[test]`
// functions only, not in their helpers).
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]

mod common;

use std::time::Instant;
use una_crypto::aes_gcm::AesGcm;
use una_crypto::chacha20poly1305::seal_in_place;
use una_crypto::sha2::{Sha256, Sha512};
use una_crypto::x25519::x25519;

/// One pass of the primitive `name` over `buf`, the caller's buffer: nothing is allocated or
/// copied here, so the clock sees the primitive alone (#1481; a `to_vec` of the message inside
/// the timed pass counted a 32 MiB allocation, its page faults and the copy). An AEAD seals the
/// buffer in place, pass after pass: its time does not depend on the content.
fn run(name: &str, buf: &mut [u8]) {
    match name {
        "sha256" => {
            std::hint::black_box(Sha256::digest(buf));
        }
        "sha512" => {
            std::hint::black_box(Sha512::digest(buf));
        }
        "aes128gcm" | "aes256gcm" => {
            let tag = if name == "aes128gcm" {
                AesGcm::new_128(&[1u8; 16]).seal_in_place(&[2u8; 12], b"", buf)
            } else {
                AesGcm::new_256(&[1u8; 32]).seal_in_place(&[2u8; 12], b"", buf)
            };
            assert!(std::hint::black_box(tag).is_ok());
        }
        _ => {
            let tag = seal_in_place(&[1u8; 32], &[2u8; 12], b"", buf);
            assert!(std::hint::black_box(tag).is_ok());
        }
    }
}

fn seconds(mut f: impl FnMut()) -> f64 {
    f(); // warm up
    let best = (0..3)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed().as_secs_f64()
        })
        .fold(f64::MAX, f64::min);
    best.max(1e-9)
}

#[test]
fn time_is_linear_in_the_message_size() {
    if cfg!(debug_assertions) {
        return;
    }
    // Allocated and written once, before any clock starts: a nonzero fill makes the pages real,
    // and the warm-up pass of `seconds` touches them again.
    let mut small = vec![0x5au8; 4 << 20];
    let mut large = vec![0x5au8; 32 << 20];
    for name in [
        "sha256",
        "sha512",
        "chacha20poly1305",
        "aes128gcm",
        "aes256gcm",
    ] {
        let t_small = seconds(|| run(name, &mut small));
        let t_large = seconds(|| run(name, &mut large));
        let ratio = t_large / t_small;
        // 8x the data: linear is 8; allow 4 to 16.
        assert!((4.0..16.0).contains(&ratio), "{name}: ratio {ratio:.2}");
        let mib_s = 32.0 / t_large;
        // The bitsliced AES-GCM is slower by design (ADR 0133 rule 14).
        let floor = if name.starts_with("aes") { 5.0 } else { 50.0 };
        assert!(mib_s > floor, "{name}: {mib_s:.0} MiB/s");
    }
}

#[test]
fn one_scalar_multiplication_is_fast() {
    if cfg!(debug_assertions) {
        return;
    }
    let mut p = [0u8; 32];
    p[0] = 9;
    let k = [0x55u8; 32];
    let t = seconds(|| {
        let mut x = p;
        for _ in 0..200 {
            x = x25519(&k, &x);
        }
        std::hint::black_box(x);
    }) / 200.0;
    assert!(t < 500e-6, "{:.1} us per X25519", t * 1e6);
}

#[test]
fn ed25519_verification_is_fast_and_linear_in_the_message() {
    use una_crypto::ed25519::verify;
    if cfg!(debug_assertions) {
        return;
    }
    use ring::signature::{Ed25519KeyPair, KeyPair};
    let kp = Ed25519KeyPair::from_seed_unchecked(&[4u8; 32]).unwrap();
    let pk: [u8; 32] = kp.public_key().as_ref().try_into().unwrap();
    let small = vec![1u8; 1 << 20];
    let large = vec![1u8; 8 << 20];
    let sig_small: [u8; 64] = kp.sign(&small).as_ref().try_into().unwrap();
    let sig_large: [u8; 64] = kp.sign(&large).as_ref().try_into().unwrap();
    // A fixed-size message: the group work is constant, so one verification is fast.
    let tiny = b"x";
    let sig_tiny: [u8; 64] = kp.sign(tiny).as_ref().try_into().unwrap();
    let t = seconds(|| {
        for _ in 0..20 {
            assert!(verify(&pk, tiny, &sig_tiny).is_ok());
        }
    }) / 20.0;
    assert!(t < 5e-3, "{:.2} ms per Ed25519 verification", t * 1e3);
    // Large messages: the cost is the hash, linear in the size (8x data: 4 to 16x time once the
    // fixed group work is a small part of it).
    let t_small = seconds(|| assert!(verify(&pk, &small, &sig_small).is_ok()));
    let t_large = seconds(|| assert!(verify(&pk, &large, &sig_large).is_ok()));
    let ratio = t_large / t_small;
    assert!((3.0..16.0).contains(&ratio), "ratio {ratio:.2}");
}

#[test]
fn ecdsa_verification_is_fast_and_linear_in_the_message() {
    use ring::rand::SystemRandom;
    use ring::signature::{
        ECDSA_P256_SHA256_FIXED_SIGNING, ECDSA_P384_SHA384_FIXED_SIGNING, EcdsaKeyPair, KeyPair,
    };
    use una_crypto::ecdsa::{Curve, HashAlg, verify_fixed};
    if cfg!(debug_assertions) {
        return;
    }
    let rng = SystemRandom::new();
    for (curve, hash, alg, budget) in [
        (
            Curve::P256,
            HashAlg::Sha256,
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            3e-3,
        ),
        (
            Curve::P384,
            HashAlg::Sha384,
            &ECDSA_P384_SHA384_FIXED_SIGNING,
            8e-3,
        ),
    ] {
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(alg, &rng).unwrap();
        let kp = EcdsaKeyPair::from_pkcs8(alg, pkcs8.as_ref(), &rng).unwrap();
        let pk = kp.public_key().as_ref().to_vec();
        // A fixed-size message: the group work is constant (O(1)) and fast.
        let tiny = b"x";
        let sig_tiny = kp.sign(&rng, tiny).unwrap().as_ref().to_vec();
        let t = seconds(|| {
            for _ in 0..10 {
                assert!(verify_fixed(curve, hash, &pk, tiny, &sig_tiny).is_ok());
            }
        }) / 10.0;
        eprintln!("{curve:?}: {:.0} us per verification", t * 1e6);
        assert!(t < budget, "{curve:?}: {:.2} ms per verification", t * 1e3);
        // Large messages: the cost is the hash, linear in the size.
        let small = vec![1u8; 1 << 20];
        let large = vec![1u8; 8 << 20];
        let sig_small = kp.sign(&rng, &small).unwrap().as_ref().to_vec();
        let sig_large = kp.sign(&rng, &large).unwrap().as_ref().to_vec();
        let t_small =
            seconds(|| assert!(verify_fixed(curve, hash, &pk, &small, &sig_small).is_ok()));
        let t_large =
            seconds(|| assert!(verify_fixed(curve, hash, &pk, &large, &sig_large).is_ok()));
        let ratio = t_large / t_small;
        assert!((3.0..16.0).contains(&ratio), "{curve:?}: ratio {ratio:.2}");
    }
}

/// `(RSAPublicKey DER, message, PKCS#1 v1.5 SHA-256 signature)` of a known-answer key.
fn rsa_known_answer(doc: &serde_json::Value, name: &str) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let key = doc["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["name"] == name)
        .unwrap();
    let hex_text = key["n"].as_str().unwrap();
    let n = common::hex(&format!(
        "{}{hex_text}",
        if hex_text.len() % 2 == 1 { "0" } else { "" }
    ));
    let integer = |v: &[u8]| {
        let v = if v[0] & 0x80 != 0 {
            [&[0u8][..], v].concat()
        } else {
            v.to_vec()
        };
        let mut out = vec![0x02, 0x82, (v.len() >> 8) as u8, v.len() as u8];
        out.extend_from_slice(&v);
        out
    };
    let body = [integer(&n), vec![0x02, 0x03, 1, 0, 1]].concat();
    let mut der = vec![0x30, 0x82, (body.len() >> 8) as u8, body.len() as u8];
    der.extend_from_slice(&body);
    let msg = key["message_utf8"].as_str().unwrap().as_bytes().to_vec();
    let kat = &key["kats"][0];
    assert_eq!(kat["scheme"], "pkcs1");
    assert_eq!(kat["hash"], "sha256");
    (der, msg, common::hex(kat["sig"].as_str().unwrap()))
}

#[test]
fn rsa_verification_is_quadratic_in_the_modulus_and_linear_in_the_message() {
    use una_crypto::rsa::{HashAlg, PublicKey};
    if cfg!(debug_assertions) {
        return;
    }
    // Known answers made with plain big-integer arithmetic (tests/vectors/rsa_test_keys.json).
    let doc: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(common::vector_path("rsa_test_keys.json")).unwrap(),
    )
    .unwrap();
    let mut times = Vec::new();
    for name in ["k2048", "k4096", "k8192"] {
        let (der, msg, sig) = rsa_known_answer(&doc, name);
        // Key parsing and setup are part of the cost; e = 65537 is 16 squarings and one product.
        let t = seconds(|| {
            for _ in 0..5 {
                let key = PublicKey::from_rsa_public_key_der(&der).unwrap();
                assert!(key.verify_pkcs1(HashAlg::Sha256, &msg, &sig).is_ok());
            }
        }) / 5.0;
        eprintln!("{name}: {:.0} us per verification", t * 1e6);
        times.push(t);
    }
    // Doubling the modulus quadruples a Montgomery product (O(k^2)); fixed costs and memory
    // effects widen the band: 2.5 to 7 (a cubic algorithm would give 8).
    for (pair, label) in times.windows(2).zip(["2048 to 4096", "4096 to 8192"]) {
        let ratio = pair[1] / pair[0];
        assert!((2.5..7.0).contains(&ratio), "{label}: ratio {ratio:.2}");
    }
    assert!(times[0] < 3e-3, "{:.2} ms for 2048 bits", times[0] * 1e3);
    assert!(times[2] < 40e-3, "{:.2} ms for 8192 bits", times[2] * 1e3);
    // Large messages: the cost is the hash, linear in the size. The signature does not match the
    // message, so the result is a refusal after the hash and the exponentiation both ran.
    let (der, _, sig) = rsa_known_answer(&doc, "k2048");
    let key = PublicKey::from_rsa_public_key_der(&der).unwrap();
    let small = vec![1u8; 1 << 20];
    let large = vec![1u8; 8 << 20];
    let t_small = seconds(|| assert!(key.verify_pkcs1(HashAlg::Sha256, &small, &sig).is_err()));
    let t_large = seconds(|| assert!(key.verify_pkcs1(HashAlg::Sha256, &large, &sig).is_err()));
    let ratio = t_large / t_small;
    assert!((3.0..16.0).contains(&ratio), "message ratio {ratio:.2}");
}
