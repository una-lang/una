//! SHA-2 known answers (FIPS 180-4 examples) and differential tests.

// Test code: unwrap, indexing and panics are how a test fails (clippy.toml allows them in `#[test]`
// functions only, not in their helpers).
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]

mod common;

use common::{Rng, to_hex};
use una_crypto::Hash;
use una_crypto::sha2::{Sha256, Sha384, Sha512};

fn d256(m: &[u8]) -> String {
    to_hex(&Sha256::digest(m))
}
fn d384(m: &[u8]) -> String {
    to_hex(&Sha384::digest(m))
}
fn d512(m: &[u8]) -> String {
    to_hex(&Sha512::digest(m))
}

#[test]
fn fips_180_4_examples() {
    assert_eq!(
        d256(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        d256(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        d256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    assert_eq!(
        d384(b"abc"),
        "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
    );
    assert_eq!(
        d384(b""),
        "38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da274edebfe76f65fbd51ad2f14898b95b"
    );
    assert_eq!(
        d512(b"abc"),
        "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
    );
    assert_eq!(
        d512(b""),
        "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
    );
    let two_block = b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu";
    assert_eq!(
        d512(two_block),
        "8e959b75dae313da8cf4f72814fc143f8f7779c6eb9f7fa17299aeadb6889018501d289e4900f7e4331b99dec4b5433ac7d329eeb6dd26545e96e55b874be909"
    );
    assert_eq!(
        d384(two_block),
        "09330c33f71147e83d192fc782cd1b4753111b173b3b05d22fa08086e3b0f712fcc7c71a557e2db966c3e9fa91746039"
    );
}

#[test]
fn one_million_a() {
    let m = vec![b'a'; 1_000_000];
    assert_eq!(
        d256(&m),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
    assert_eq!(
        d512(&m),
        "e718483d0ce769644e2e42c7bc15b4638e1f98b13b2044285632a803afa973ebde0ff244877ea60a4cb0432ce577c31beb009c5c2c49aa2e4eadb217ad8cc09b"
    );
    assert_eq!(
        d384(&m),
        "9d0e1809716474cb086e834e310a4a1ced149e9c00f248527972cec5704c2a5b07b8b3dc38ecc4ebae97ddd87f3d8985"
    );
}

#[test]
fn every_length_around_the_block_boundaries_matches_the_reference() {
    use sha2::Digest;
    let mut rng = Rng(0x5eed);
    for len in 0..=300 {
        let m = rng.bytes(len);
        assert_eq!(
            Sha256::digest(&m)[..],
            sha2::Sha256::digest(&m)[..],
            "256/{len}"
        );
        assert_eq!(
            Sha384::digest(&m)[..],
            sha2::Sha384::digest(&m)[..],
            "384/{len}"
        );
        assert_eq!(
            Sha512::digest(&m)[..],
            sha2::Sha512::digest(&m)[..],
            "512/{len}"
        );
        let r = ring::digest::digest(&ring::digest::SHA256, &m);
        assert_eq!(Sha256::digest(&m)[..], *r.as_ref());
        let r = ring::digest::digest(&ring::digest::SHA384, &m);
        assert_eq!(Sha384::digest(&m)[..], *r.as_ref());
        let r = ring::digest::digest(&ring::digest::SHA512, &m);
        assert_eq!(Sha512::digest(&m)[..], *r.as_ref());
    }
}

#[test]
fn incremental_updates_equal_one_shot_for_any_split() {
    let mut rng = Rng(77);
    for _ in 0..300 {
        let len = rng.below(700);
        let m = rng.bytes(len);
        let mut h = Sha256::new();
        let mut h5 = Sha512::new();
        let mut at = 0;
        while at < len {
            let step = 1 + rng.below(150);
            let end = (at + step).min(len);
            h.update(&m[at..end]);
            h5.update(&m[at..end]);
            at = end;
        }
        let mut o = [0u8; 32];
        h.finalize_into(&mut o);
        assert_eq!(o, Sha256::digest(&m));
        let mut o5 = [0u8; 64];
        h5.finalize_into(&mut o5);
        assert_eq!(o5, Sha512::digest(&m));
    }
}
