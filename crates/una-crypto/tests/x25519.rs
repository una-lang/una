//! X25519: RFC 7748 vectors, Wycheproof, differential tests against `curve25519-dalek`.

// Test code: unwrap, indexing and panics are how a test fails (clippy.toml allows them in `#[test]`
// functions only, not in their helpers).
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]

mod common;

use common::{Rng, array32, hex, tests_of, text, to_hex, wycheproof};
use una_crypto::x25519::{LowOrderPoint, public_key, shared_secret, x25519};

#[test]
fn rfc7748_section_5_2() {
    let k = array32(&hex(
        "a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4",
    ));
    let u = array32(&hex(
        "e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c",
    ));
    assert_eq!(
        to_hex(&x25519(&k, &u)),
        "c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552"
    );
    let k = array32(&hex(
        "4b66e9d4d1b4673c5ad22691957d6af5c11b6421e0ea01d42ca4169e7918ba0d",
    ));
    let u = array32(&hex(
        "e5210f12786811d3f4b7959d0538ae2c31dbe7106fc03c3efc4cd549c715a493",
    ));
    assert_eq!(
        to_hex(&x25519(&k, &u)),
        "95cbde9476e8907d7aade45cb4b873f88b595a68799fa152e6f8f7647aac7957"
    );
}

#[test]
fn rfc7748_iterated() {
    let mut k = [0u8; 32];
    k[0] = 9;
    let mut u = k;
    let mut after_1 = String::new();
    for i in 1..=1000 {
        let out = x25519(&k, &u);
        u = k;
        k = out;
        if i == 1 {
            after_1 = to_hex(&k);
        }
    }
    assert_eq!(
        after_1,
        "422c8e7a6227d7bca1350b3e2bb7279f7897b87bb6854b783c60e80311ae3079"
    );
    assert_eq!(
        to_hex(&k),
        "684cf59ba83309552800ef566f2f4d3c1c3887c49360e3875f2eb94d99532c51"
    );
}

#[test]
fn rfc7748_section_6_1_diffie_hellman() {
    let a = array32(&hex(
        "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a",
    ));
    let b = array32(&hex(
        "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb",
    ));
    let a_pub = public_key(&a);
    let b_pub = public_key(&b);
    assert_eq!(
        to_hex(&a_pub),
        "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"
    );
    assert_eq!(
        to_hex(&b_pub),
        "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f"
    );
    let want = "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742";
    assert_eq!(to_hex(&shared_secret(&a, &b_pub).unwrap()), want);
    assert_eq!(to_hex(&shared_secret(&b, &a_pub).unwrap()), want);
}

#[test]
fn low_order_inputs_are_rejected() {
    let zero = [0u8; 32];
    let mut one = [0u8; 32];
    one[0] = 1;
    let sk = [0x42u8; 32];
    assert_eq!(shared_secret(&sk, &zero), Err(LowOrderPoint));
    assert_eq!(shared_secret(&sk, &one), Err(LowOrderPoint));
    // 2^255 - 19 + 0 (p itself, non canonical zero) and its twin with the ignored top bit.
    let mut p = [0xffu8; 32];
    p[0] = 0xed;
    p[31] = 0x7f;
    assert_eq!(shared_secret(&sk, &p), Err(LowOrderPoint));
    p[31] = 0xff;
    assert_eq!(shared_secret(&sk, &p), Err(LowOrderPoint));
}

#[test]
fn wycheproof_x25519() {
    let root = wycheproof("x25519_test.json");
    let mut n = 0;
    for (_, t) in tests_of(&root) {
        let private = array32(&hex(text(t, "private")));
        let public = array32(&hex(text(t, "public")));
        let shared = array32(&hex(text(t, "shared")));
        let got = x25519(&private, &public);
        assert_eq!(got, shared, "tc {}", t["tcId"]);
        let checked = shared_secret(&private, &public);
        if shared == [0u8; 32] {
            assert_eq!(checked, Err(LowOrderPoint), "tc {}", t["tcId"]);
        } else {
            assert_eq!(checked, Ok(shared), "tc {}", t["tcId"]);
        }
        n += 1;
    }
    assert_eq!(n as u64, root["numberOfTests"].as_u64().unwrap());
}

#[test]
fn differential_against_curve25519_dalek() {
    use curve25519_dalek::montgomery::MontgomeryPoint;
    let mut rng = Rng(0x25519);
    for i in 0..300 {
        let k = array32(&rng.bytes(32));
        let mut u = array32(&rng.bytes(32));
        if i % 7 == 0 {
            u[31] |= 0x80; // the top bit is ignored
        }
        let theirs = MontgomeryPoint(u).mul_clamped(k).0;
        assert_eq!(x25519(&k, &u), theirs, "case {i}");
    }
    // Edge scalars: all zeros, all ones, and the clamping boundaries.
    for k in [[0u8; 32], [0xffu8; 32], [0x80u8; 32], [0x7fu8; 32]] {
        let mut u = [0u8; 32];
        u[0] = 9;
        assert_eq!(x25519(&k, &u), MontgomeryPoint(u).mul_clamped(k).0);
    }
}
