//! Ed25519 verification: RFC 8032 vectors (unit tests of the module), Wycheproof, differential
//! tests against `ring`, and the strict policy cases.

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
use una_crypto::ed25519::verify;

/// The group order, little-endian.
const L: [u8; 32] = [
    0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
];

fn add_le(a: &[u8; 32], b: &[u8; 32]) -> ([u8; 32], bool) {
    let mut out = [0u8; 32];
    let mut carry = 0u16;
    for i in 0..32 {
        let s = u16::from(a[i]) + u16::from(b[i]) + carry;
        out[i] = s as u8;
        carry = s >> 8;
    }
    (out, carry != 0)
}

#[test]
fn wycheproof_ed25519() {
    let root = wycheproof("ed25519_test.json");
    let (mut valid, mut invalid, mut acceptable) = (0, 0, 0);
    for (group, t) in tests_of(&root) {
        let id = t["tcId"].as_u64().unwrap();
        let pk = hex(group["publicKey"]["pk"].as_str().unwrap());
        let msg = hex(text(t, "msg"));
        let sig = hex(text(t, "sig"));
        let result = text(t, "result");
        let outcome = match (<[u8; 32]>::try_from(pk), <[u8; 64]>::try_from(sig)) {
            (Ok(pk), Ok(sig)) => verify(&pk, &msg, &sig).is_ok(),
            // A signature or key of the wrong length cannot be expressed; it is rejected by type.
            _ => false,
        };
        match result {
            "valid" => {
                assert!(outcome, "tc {id} should verify");
                valid += 1;
            }
            "invalid" => {
                assert!(!outcome, "tc {id} must be rejected");
                invalid += 1;
            }
            "acceptable" => {
                // Small-order and mixed-order public keys: the strict policy of the module
                // rejects them, on purpose.
                assert!(!outcome, "tc {id}: the strict policy rejects it");
                let flags = t["flags"].to_string();
                assert!(
                    flags.contains("SmallOrderPublicKey") || flags.contains("MixedOrderPublicKey"),
                    "tc {id}: unexpected acceptable flags {flags}"
                );
                acceptable += 1;
            }
            other => panic!("unexpected result {other}"),
        }
    }
    assert_eq!(
        valid + invalid + acceptable,
        root["numberOfTests"].as_u64().unwrap()
    );
    assert!(
        valid > 100 && invalid > 40 && acceptable == 29,
        "{valid} {invalid} {acceptable}"
    );
}

fn keypair(seed: &[u8]) -> ring::signature::Ed25519KeyPair {
    ring::signature::Ed25519KeyPair::from_seed_unchecked(seed).unwrap()
}

fn ring_verify(pk: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    use ring::signature::{ED25519, UnparsedPublicKey};
    UnparsedPublicKey::new(&ED25519, pk)
        .verify(msg, sig)
        .is_ok()
}

#[test]
fn differential_against_ring() {
    use ring::signature::KeyPair;
    let mut rng = Rng(0xed25519);
    for round in 0..60 {
        let seed = rng.bytes(32);
        let kp = keypair(&seed);
        let pk: [u8; 32] = kp.public_key().as_ref().try_into().unwrap();
        let len = if round < 20 {
            round * 3
        } else {
            rng.below(3000)
        };
        let msg = rng.bytes(len);
        let sig: [u8; 64] = kp.sign(&msg).as_ref().try_into().unwrap();
        assert!(verify(&pk, &msg, &sig).is_ok(), "round {round}");

        // Mutations: wherever ours accepts, ring accepts too (ours is the stricter), and the
        // honest signature is accepted by both.
        for _ in 0..8 {
            let mut bad_sig = sig;
            bad_sig[rng.below(64)] ^= 1 << rng.below(8);
            let mut bad_msg = msg.clone();
            if !bad_msg.is_empty() {
                let i = rng.below(bad_msg.len());
                bad_msg[i] ^= 1 << rng.below(8);
            }
            let mut bad_pk = pk;
            bad_pk[rng.below(32)] ^= 1 << rng.below(8);
            for (p, m, s) in [
                (&pk, &msg, &bad_sig),
                (&pk, &bad_msg, &sig),
                (&bad_pk, &msg, &sig),
            ] {
                let ours = verify(p, m, s).is_ok();
                let theirs = ring_verify(p, m, s);
                assert!(
                    !ours || theirs,
                    "round {round}: ours accepts what ring rejects"
                );
                if ours {
                    // Only the unmodified triple can verify.
                    assert!(p == &pk && m == &msg && s == &sig, "round {round}");
                }
            }
        }
    }
}

#[test]
fn a_signature_with_s_plus_l_is_rejected() {
    use ring::signature::KeyPair;
    let kp = keypair(&[7u8; 32]);
    let pk: [u8; 32] = kp.public_key().as_ref().try_into().unwrap();
    let msg = b"malleability";
    let sig: [u8; 64] = kp.sign(msg).as_ref().try_into().unwrap();
    assert!(verify(&pk, msg, &sig).is_ok());
    let s: [u8; 32] = sig[32..].try_into().unwrap();
    let (s_plus_l, carry) = add_le(&s, &L);
    // S < L < 2^253, so S + L fits in 256 bits and is a valid-looking but non-canonical scalar.
    assert!(!carry);
    let mut forged = sig;
    forged[32..].copy_from_slice(&s_plus_l);
    assert!(verify(&pk, msg, &forged).is_err());
    // S = L itself and S with the top bits set.
    let mut forged = sig;
    forged[32..].copy_from_slice(&L);
    assert!(verify(&pk, msg, &forged).is_err());
    let mut forged = sig;
    forged[63] |= 0xe0;
    assert!(verify(&pk, msg, &forged).is_err());
}

/// The eight points of the torsion subgroup, as canonical encodings (y coordinates, sign bit
/// included where the point has a nonzero x).
fn torsion_encodings() -> Vec<[u8; 32]> {
    let mut v = Vec::new();
    let identity = {
        let mut b = [0u8; 32];
        b[0] = 1;
        b
    };
    v.push(identity);
    let minus_one = {
        let mut b = [0xffu8; 32];
        b[0] = 0xec;
        b[31] = 0x7f;
        b
    };
    v.push(minus_one);
    // (0, -1) order 2; order 4: y = 0 with either sign; order 8: the two known y values.
    let zero_y = [0u8; 32];
    v.push(zero_y);
    let mut zero_y_neg = zero_y;
    zero_y_neg[31] = 0x80;
    v.push(zero_y_neg);
    for y in [
        "26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05",
        "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a",
    ] {
        let b: [u8; 32] = hex(y).try_into().unwrap();
        v.push(b);
        let mut neg = b;
        neg[31] ^= 0x80;
        v.push(neg);
    }
    v
}

#[test]
fn small_order_keys_and_r_never_verify() {
    use ring::signature::KeyPair;
    let kp = keypair(&[9u8; 32]);
    let real_pk: [u8; 32] = kp.public_key().as_ref().try_into().unwrap();
    let msg = b"small order";
    let good: [u8; 64] = kp.sign(msg).as_ref().try_into().unwrap();
    for enc in torsion_encodings() {
        // As the public key, with the honest signature and with R = the same point, S = 0.
        assert!(verify(&enc, msg, &good).is_err());
        let mut sig = [0u8; 64];
        sig[..32].copy_from_slice(&enc);
        assert!(verify(&enc, msg, &sig).is_err());
        // As R under the real key.
        let mut sig = good;
        sig[..32].copy_from_slice(&enc);
        assert!(verify(&real_pk, msg, &sig).is_err());
    }
}

#[test]
fn non_canonical_encodings_never_verify() {
    use ring::signature::KeyPair;
    let kp = keypair(&[11u8; 32]);
    let pk: [u8; 32] = kp.public_key().as_ref().try_into().unwrap();
    let msg = b"canonical";
    let sig: [u8; 64] = kp.sign(msg).as_ref().try_into().unwrap();
    // y + p instead of y: only encodings with y < 2^255 - 19 + 19 can be added to, so test the
    // boundary values y = p, p + 1, 2^255 - 1 as public keys and as R.
    let mut non_canonical = Vec::new();
    for low in [0xedu8, 0xee, 0xff] {
        let mut b = [0xffu8; 32];
        b[0] = low;
        b[31] = 0x7f;
        non_canonical.push(b);
        let mut with_sign = b;
        with_sign[31] = 0xff;
        non_canonical.push(with_sign);
    }
    for enc in non_canonical {
        assert!(verify(&enc, msg, &sig).is_err());
        let mut bad = sig;
        bad[..32].copy_from_slice(&enc);
        assert!(verify(&pk, msg, &bad).is_err());
    }
    // x = 0 with the sign bit set (y = 1, sign 1) is refused as a key.
    let mut negative_zero = [0u8; 32];
    negative_zero[0] = 1;
    negative_zero[31] = 0x80;
    assert!(verify(&negative_zero, msg, &sig).is_err());
}

#[test]
fn a_key_that_is_not_a_point_is_refused() {
    // y = 2 has no square root for x (checked by the Wycheproof set too); scan some y until a
    // non-point is found and make sure every one fails, never panics.
    let msg = b"x";
    let sig = [0u8; 64];
    for y in 0u8..=255 {
        let mut pk = [0u8; 32];
        pk[0] = y;
        assert!(verify(&pk, msg, &sig).is_err());
    }
}
