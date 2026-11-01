//! The parts of the own TLS provider (ADR 0133 rule 17) checked one by one against the `ring`
//! provider of `rustls`: signature verification algorithms, hashes, HMAC, HKDF, the record
//! protection of each AEAD in both directions, the key exchange, the random source and the shape
//! of the provider. The handshakes are in `provider_tls.rs`.

// Test code: unwrap, indexing and panics are how a test fails.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use ring::rand::SystemRandom;
use ring::signature::{self, EcdsaKeyPair, Ed25519KeyPair, KeyPair, RsaKeyPair};
use rustls::crypto::cipher::{
    InboundOpaqueMessage, Iv, OutboundChunks, OutboundOpaqueMessage, OutboundPlainMessage,
};
use rustls::crypto::hash::HashAlgorithm;
use rustls::crypto::tls13::OkmBlock;
use rustls::crypto::{CryptoProvider, SupportedKxGroup};
use rustls::internal::{derive_traffic_iv, derive_traffic_key};
use rustls::pki_types::SignatureVerificationAlgorithm;
use rustls::{
    CipherSuite, ContentType, NamedGroup, ProtocolVersion, SupportedCipherSuite, Tls13CipherSuite,
};
use una_http::provider;

fn own() -> CryptoProvider {
    provider::crypto_provider()
}

fn reference() -> CryptoProvider {
    rustls::crypto::ring::default_provider()
}

/// A deterministic byte stream for test inputs (splitmix64).
struct Prng(u64);

impl Prng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

// Signature verification algorithms ------------------------------------------------------------

/// A signed triple: a public key (as webpki hands it over), a message and a signature, with the
/// algorithm identifiers it is valid for.
struct Triple {
    name: String,
    key: Vec<u8>,
    message: Vec<u8>,
    signature: Vec<u8>,
    public_key_alg: Vec<u8>,
    signature_alg: Vec<u8>,
}

fn ids(alg: &dyn SignatureVerificationAlgorithm) -> (Vec<u8>, Vec<u8>) {
    (
        alg.public_key_alg_id().as_ref().to_vec(),
        alg.signature_alg_id().as_ref().to_vec(),
    )
}

/// The algorithm of a provider with those identifiers.
fn find(
    provider: &CryptoProvider,
    public_key_alg: &[u8],
    signature_alg: &[u8],
) -> Option<&'static dyn SignatureVerificationAlgorithm> {
    provider
        .signature_verification_algorithms
        .all
        .iter()
        .copied()
        .find(|alg| {
            let (k, s) = ids(*alg);
            k == public_key_alg && s == signature_alg
        })
}

fn rsa_keys() -> Vec<(String, Vec<u8>)> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../una-crypto/tests/vectors/rsa_test_keys.json");
    let root: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    root["keys"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|k| {
            // The sizes ring signs with and webpki accepts, at the usual exponent.
            matches!(k["bits"].as_u64().unwrap(), 2048 | 3072 | 4096)
                && k["e"].as_u64().unwrap() == 65537
        })
        .map(|k| {
            let hex = k["rsa_private_key_der"].as_str().unwrap();
            let der = (0..hex.len() / 2)
                .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
                .collect();
            (k["name"].as_str().unwrap().to_owned(), der)
        })
        .collect()
}

/// Valid triples for every algorithm of the list, signed by `ring`.
fn triples() -> Vec<Triple> {
    let rng = SystemRandom::new();
    let mut out = Vec::new();
    let mut prng = Prng(7);
    let reference = reference();
    let mut add = |name: String,
                   key: Vec<u8>,
                   message: Vec<u8>,
                   signature: Vec<u8>,
                   alg: &dyn SignatureVerificationAlgorithm| {
        let (public_key_alg, signature_alg) = ids(alg);
        out.push(Triple {
            name,
            key,
            message,
            signature,
            public_key_alg,
            signature_alg,
        });
    };
    let by = |public_key_alg: &rustls::pki_types::AlgorithmIdentifier,
              signature_alg: &rustls::pki_types::AlgorithmIdentifier| {
        find(&reference, public_key_alg.as_ref(), signature_alg.as_ref()).unwrap()
    };
    use rustls::pki_types::alg_id;
    for (curve, signing, public_alg) in [
        (
            "p256",
            &signature::ECDSA_P256_SHA256_ASN1_SIGNING,
            alg_id::ECDSA_P256,
        ),
        (
            "p384",
            &signature::ECDSA_P384_SHA384_ASN1_SIGNING,
            alg_id::ECDSA_P384,
        ),
    ] {
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(signing, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(signing, pkcs8.as_ref(), &rng).unwrap();
        let message = prng.bytes(100);
        let signature = pair.sign(&rng, &message).unwrap();
        // ring's signing algorithm fixes the hash: SHA-256 with P-256, SHA-384 with P-384.
        let sig_alg = if curve == "p256" {
            alg_id::ECDSA_SHA256
        } else {
            alg_id::ECDSA_SHA384
        };
        add(
            format!("ecdsa-{curve}"),
            pair.public_key().as_ref().to_vec(),
            message,
            signature.as_ref().to_vec(),
            by(&public_alg, &sig_alg),
        );
    }
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let message = prng.bytes(77);
    let signature = pair.sign(&message);
    add(
        "ed25519".to_owned(),
        pair.public_key().as_ref().to_vec(),
        message,
        signature.as_ref().to_vec(),
        by(&alg_id::ED25519, &alg_id::ED25519),
    );
    for (name, der) in rsa_keys() {
        let pair = RsaKeyPair::from_der(&der).unwrap();
        let key = pair.public().as_ref().to_vec();
        for (label, padding, sig_alg) in [
            (
                "pkcs1-sha256",
                &signature::RSA_PKCS1_SHA256 as &'static dyn signature::RsaEncoding,
                alg_id::RSA_PKCS1_SHA256,
            ),
            (
                "pkcs1-sha384",
                &signature::RSA_PKCS1_SHA384,
                alg_id::RSA_PKCS1_SHA384,
            ),
            (
                "pkcs1-sha512",
                &signature::RSA_PKCS1_SHA512,
                alg_id::RSA_PKCS1_SHA512,
            ),
            (
                "pss-sha256",
                &signature::RSA_PSS_SHA256,
                alg_id::RSA_PSS_SHA256,
            ),
            (
                "pss-sha384",
                &signature::RSA_PSS_SHA384,
                alg_id::RSA_PSS_SHA384,
            ),
            (
                "pss-sha512",
                &signature::RSA_PSS_SHA512,
                alg_id::RSA_PSS_SHA512,
            ),
        ] {
            let message = prng.bytes(60);
            let mut signature = vec![0u8; pair.public().modulus_len()];
            pair.sign(padding, &rng, &message, &mut signature).unwrap();
            add(
                format!("rsa-{name}-{label}"),
                key.clone(),
                message,
                signature,
                by(&alg_id::RSA_ENCRYPTION, &sig_alg),
            );
        }
    }
    out
}

#[test]
fn the_list_has_the_same_algorithms_as_the_reference() {
    let (own, reference) = (own(), reference());
    let sorted = |p: &CryptoProvider| {
        let mut v: Vec<_> = p
            .signature_verification_algorithms
            .all
            .iter()
            .map(|a| ids(*a))
            .collect();
        v.sort();
        v
    };
    assert_eq!(sorted(&own), sorted(&reference));
    // The scheme mapping offers the same schemes, each to the same identifiers.
    let schemes = |p: &CryptoProvider| {
        let mut v: Vec<_> = p
            .signature_verification_algorithms
            .mapping
            .iter()
            .map(|(scheme, algs)| {
                (
                    u16::from(*scheme),
                    algs.iter().map(|a| ids(*a)).collect::<Vec<_>>(),
                )
            })
            .collect();
        v.sort();
        v
    };
    assert_eq!(schemes(&own), schemes(&reference));
}

#[test]
fn every_algorithm_accepts_what_the_reference_signs() {
    let (own, reference) = (own(), reference());
    let triples = triples();
    assert!(triples.len() >= 2 + 1 + 6, "{}", triples.len());
    for t in &triples {
        let ours = find(&own, &t.public_key_alg, &t.signature_alg).unwrap();
        let theirs = find(&reference, &t.public_key_alg, &t.signature_alg).unwrap();
        assert!(
            theirs
                .verify_signature(&t.key, &t.message, &t.signature)
                .is_ok(),
            "{}",
            t.name
        );
        assert!(
            ours.verify_signature(&t.key, &t.message, &t.signature)
                .is_ok(),
            "{}",
            t.name
        );
    }
}

/// Our verdict never accepts what the reference rejects, and for these mutations they agree.
fn same_verdict(
    own: &CryptoProvider,
    reference: &CryptoProvider,
    t: &Triple,
    key: &[u8],
    message: &[u8],
    signature: &[u8],
    what: &str,
) {
    let ours = find(own, &t.public_key_alg, &t.signature_alg).unwrap();
    let theirs = find(reference, &t.public_key_alg, &t.signature_alg).unwrap();
    let a = ours.verify_signature(key, message, signature).is_ok();
    let b = theirs.verify_signature(key, message, signature).is_ok();
    assert_eq!(a, b, "{} {what}: ours {a}, reference {b}", t.name);
}

#[test]
fn mutated_keys_messages_and_signatures_get_the_verdict_of_the_reference() {
    let (own, reference) = (own(), reference());
    let mut prng = Prng(99);
    for t in &triples() {
        let mut count = 0;
        // Bit flips: every 7th bit of the signature, a few of the message, a few of the key.
        for bit in (0..t.signature.len() * 8).step_by(7) {
            let mut s = t.signature.clone();
            s[bit / 8] ^= 1 << (bit % 8);
            same_verdict(&own, &reference, t, &t.key, &t.message, &s, "signature bit");
            count += 1;
        }
        for bit in (0..t.message.len() * 8).step_by(13) {
            let mut m = t.message.clone();
            m[bit / 8] ^= 1 << (bit % 8);
            same_verdict(&own, &reference, t, &t.key, &m, &t.signature, "message bit");
        }
        let key_step = if t.key.len() > 100 { 37 } else { 5 };
        for bit in (0..t.key.len() * 8).step_by(key_step) {
            let mut k = t.key.clone();
            k[bit / 8] ^= 1 << (bit % 8);
            same_verdict(&own, &reference, t, &k, &t.message, &t.signature, "key bit");
        }
        // Truncation, extension, empty and random values of the right and wrong lengths.
        for len in [
            0,
            1,
            t.signature.len() - 1,
            t.signature.len() + 1,
            t.signature.len() * 2,
        ] {
            let s = if len <= t.signature.len() {
                t.signature[..len].to_vec()
            } else {
                let mut v = t.signature.clone();
                v.extend(prng.bytes(len - t.signature.len()));
                v
            };
            same_verdict(
                &own,
                &reference,
                t,
                &t.key,
                &t.message,
                &s,
                "signature length",
            );
        }
        for len in [0, 1, t.key.len() - 1, t.key.len() + 1] {
            let k = if len <= t.key.len() {
                t.key[..len].to_vec()
            } else {
                let mut v = t.key.clone();
                v.push(0);
                v
            };
            same_verdict(
                &own,
                &reference,
                t,
                &k,
                &t.message,
                &t.signature,
                "key length",
            );
        }
        let random = prng.bytes(t.signature.len());
        same_verdict(
            &own,
            &reference,
            t,
            &t.key,
            &t.message,
            &random,
            "random signature",
        );
        same_verdict(
            &own,
            &reference,
            t,
            &t.key,
            &[],
            &t.signature,
            "empty message",
        );
        assert!(count > 3, "{}", t.name);
    }
}

#[test]
fn a_signature_of_one_algorithm_never_verifies_under_another() {
    let (own, reference) = (own(), reference());
    let triples = triples();
    let mut crossed = 0;
    for t in &triples {
        for alg in own.signature_verification_algorithms.all {
            let (k, s) = ids(*alg);
            if k == t.public_key_alg && s == t.signature_alg {
                continue;
            }
            // The twin with the `NULL` parameters left out names the same scheme (11 bytes of
            // object identifier in common) and verifies the same signatures, by design.
            let twin = k == t.public_key_alg
                && s.len() >= 11
                && t.signature_alg.len() >= 11
                && s[..11] == t.signature_alg[..11];
            if twin {
                continue;
            }
            // The wrong algorithm: another curve, hash, scheme or key type.
            let theirs = find(&reference, &k, &s).unwrap();
            let a = alg
                .verify_signature(&t.key, &t.message, &t.signature)
                .is_ok();
            let b = theirs
                .verify_signature(&t.key, &t.message, &t.signature)
                .is_ok();
            assert!(
                !a,
                "{} accepted by the wrong algorithm (reference {b})",
                t.name
            );
            assert_eq!(a, b, "{}", t.name);
            crossed += 1;
        }
    }
    assert!(crossed > 100, "{crossed}");
}

#[test]
fn identifiers_are_those_of_the_defaults() {
    use rustls::pki_types::alg_id;
    let own = own();
    let check = |k: &rustls::pki_types::AlgorithmIdentifier,
                 s: &rustls::pki_types::AlgorithmIdentifier| {
        assert!(find(&own, k.as_ref(), s.as_ref()).is_some());
    };
    check(&alg_id::ECDSA_P256, &alg_id::ECDSA_SHA256);
    check(&alg_id::ECDSA_P384, &alg_id::ECDSA_SHA384);
    check(&alg_id::ED25519, &alg_id::ED25519);
    check(&alg_id::RSA_ENCRYPTION, &alg_id::RSA_PKCS1_SHA256);
    check(&alg_id::RSA_ENCRYPTION, &alg_id::RSA_PSS_SHA512);
    // No SHA-1, no P-521, no Ed448, no RSA-PSS key type.
    assert!(
        find(
            &own,
            alg_id::ECDSA_P521.as_ref(),
            alg_id::ECDSA_SHA512.as_ref()
        )
        .is_none()
    );
    assert!(find(&own, alg_id::ED448.as_ref(), alg_id::ED448.as_ref()).is_none());
    assert_eq!(own.signature_verification_algorithms.all.len(), 14);
}

// Hashes, HMAC, HKDF -------------------------------------------------------------------------------

fn tls13_suites(provider: &CryptoProvider) -> Vec<&'static Tls13CipherSuite> {
    provider
        .cipher_suites
        .iter()
        .filter_map(|s| match s {
            SupportedCipherSuite::Tls13(s) => Some(*s),
            SupportedCipherSuite::Tls12(_) => None,
        })
        .collect()
}

fn same_suite(provider: &CryptoProvider, id: CipherSuite) -> SupportedCipherSuite {
    *provider
        .cipher_suites
        .iter()
        .find(|s| s.suite() == id)
        .unwrap()
}

#[test]
fn hashes_match_the_reference_and_the_known_digests() {
    let (own, reference) = (own(), reference());
    let mut prng = Prng(5);
    let mut inputs: Vec<Vec<u8>> = vec![Vec::new(), b"abc".to_vec()];
    for len in [1, 55, 56, 63, 64, 65, 111, 112, 127, 128, 129, 1000, 4097] {
        inputs.push(prng.bytes(len));
    }
    for suite in tls13_suites(&own) {
        let theirs = match same_suite(&reference, suite.common.suite) {
            SupportedCipherSuite::Tls13(s) => s,
            SupportedCipherSuite::Tls12(_) => panic!(),
        };
        let (a, b) = (suite.common.hash_provider, theirs.common.hash_provider);
        assert_eq!(a.output_len(), b.output_len());
        assert_eq!(a.algorithm(), b.algorithm());
        for input in &inputs {
            assert_eq!(a.hash(input).as_ref(), b.hash(input).as_ref());
            // Split updates, and a fork finished before and after more data.
            let mut ctx = a.start();
            let mut rctx = b.start();
            let (head, tail) = input.split_at(input.len() / 3);
            ctx.update(head);
            rctx.update(head);
            assert_eq!(ctx.fork_finish().as_ref(), rctx.fork_finish().as_ref());
            let mut fork = ctx.fork();
            ctx.update(tail);
            rctx.update(tail);
            fork.update(tail);
            assert_eq!(fork.finish().as_ref(), rctx.fork_finish().as_ref());
            assert_eq!(ctx.finish().as_ref(), rctx.finish().as_ref());
        }
    }
    // FIPS 180-4 examples.
    let by_id = |id| match same_suite(&own, id) {
        SupportedCipherSuite::Tls13(s) => s.common.hash_provider,
        SupportedCipherSuite::Tls12(_) => panic!(),
    };
    let sha256 = by_id(CipherSuite::TLS13_AES_128_GCM_SHA256);
    let sha384 = by_id(CipherSuite::TLS13_AES_256_GCM_SHA384);
    assert_eq!(sha256.algorithm(), HashAlgorithm::SHA256);
    assert_eq!(sha384.algorithm(), HashAlgorithm::SHA384);
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    assert_eq!(
        hex(sha256.hash(b"abc").as_ref()),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        hex(sha384.hash(b"abc").as_ref()),
        "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
    );
}

#[test]
fn hkdf_and_hmac_match_the_reference() {
    let (own, reference) = (own(), reference());
    let mut prng = Prng(11);
    for suite in tls13_suites(&own) {
        let theirs = match same_suite(&reference, suite.common.suite) {
            SupportedCipherSuite::Tls13(s) => s,
            SupportedCipherSuite::Tls12(_) => panic!(),
        };
        let len = suite.common.hash_provider.output_len();
        for case in 0..20 {
            let salt = (case % 3 != 0).then(|| prng.bytes(case * 7 % 90));
            let secret = prng.bytes(case * 13 % 140);
            let a = suite
                .hkdf_provider
                .extract_from_secret(salt.as_deref(), &secret);
            let b = theirs
                .hkdf_provider
                .extract_from_secret(salt.as_deref(), &secret);
            let info1 = prng.bytes(3);
            let info2 = prng.bytes(case + 1);
            let info = [info1.as_slice(), info2.as_slice()];
            assert_eq!(
                a.expand_block(&info).as_ref(),
                b.expand_block(&info).as_ref()
            );
            assert_eq!(a.hash_len(), len);
            for out_len in [1, 12, len, 2 * len + 1, 255 * len] {
                let (mut x, mut y) = (vec![0u8; out_len], vec![0u8; out_len]);
                assert_eq!(
                    a.expand_slice(&info, &mut x).is_ok(),
                    b.expand_slice(&info, &mut y).is_ok()
                );
                assert_eq!(x, y);
            }
            // One byte past the limit of 255 blocks is an error, not a panic.
            let mut long = vec![0u8; 255 * len + 1];
            assert!(a.expand_slice(&info, &mut long).is_err());
            assert!(b.expand_slice(&info, &mut long).is_err());
            let okm = OkmBlock::new(&prng.bytes(len));
            let msg = prng.bytes(case * 31);
            assert_eq!(
                suite.hkdf_provider.hmac_sign(&okm, &msg).as_ref(),
                theirs.hkdf_provider.hmac_sign(&okm, &msg).as_ref()
            );
            let (ea, eb) = (
                suite.hkdf_provider.expander_for_okm(&okm),
                theirs.hkdf_provider.expander_for_okm(&okm),
            );
            assert_eq!(
                ea.expand_block(&info).as_ref(),
                eb.expand_block(&info).as_ref()
            );
            let (za, zb) = (
                suite.hkdf_provider.extract_from_zero_ikm(salt.as_deref()),
                theirs.hkdf_provider.extract_from_zero_ikm(salt.as_deref()),
            );
            assert_eq!(
                za.expand_block(&info).as_ref(),
                zb.expand_block(&info).as_ref()
            );
        }
    }
    // RFC 5869 test case 1 (SHA-256).
    let suite = same_suite(&own, CipherSuite::TLS13_AES_128_GCM_SHA256);
    let SupportedCipherSuite::Tls13(suite) = suite else {
        panic!()
    };
    let ikm = [0x0b; 22];
    let salt: Vec<u8> = (0..=12).collect();
    let prk = suite.hkdf_provider.extract_from_secret(Some(&salt), &ikm);
    let info: Vec<u8> = (0xf0..=0xf9).collect();
    let mut okm = [0u8; 42];
    prk.expand_slice(&[&info], &mut okm).unwrap();
    let want =
        "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865";
    let got: String = okm.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(got, want);
}

// Record protection -------------------------------------------------------------------------------

fn encode(message: OutboundOpaqueMessage) -> (ContentType, ProtocolVersion, Vec<u8>) {
    let (typ, version) = (message.typ, message.version);
    let bytes = message.encode();
    (typ, version, bytes[5..].to_vec())
}

fn plain(typ: ContentType, payload: &[u8]) -> OutboundPlainMessage<'_> {
    OutboundPlainMessage {
        typ,
        version: ProtocolVersion::TLSv1_3,
        payload: OutboundChunks::Single(payload),
    }
}

#[test]
fn tls13_records_cross_between_the_providers_for_every_suite() {
    let (own, reference) = (own(), reference());
    let mut prng = Prng(21);
    for suite in tls13_suites(&own) {
        let theirs = match same_suite(&reference, suite.common.suite) {
            SupportedCipherSuite::Tls13(s) => s,
            SupportedCipherSuite::Tls12(_) => panic!(),
        };
        let name = format!("{:?}", suite.common.suite);
        let secret = OkmBlock::new(&prng.bytes(suite.common.hash_provider.output_len()));
        let expander = suite.hkdf_provider.expander_for_okm(&secret);
        let key_own = derive_traffic_key(expander.as_ref(), suite.aead_alg);
        let key_ref = derive_traffic_key(expander.as_ref(), theirs.aead_alg);
        assert_eq!(key_own.as_ref(), key_ref.as_ref());
        assert_eq!(suite.aead_alg.key_len(), theirs.aead_alg.key_len());
        let iv_bytes = derive_traffic_iv(expander.as_ref());
        let iv = || Iv::copy(iv_bytes.as_ref());
        for len in (0..=200).chain([1000, 4096, 16383, 16384]) {
            for seq in [0u64, 1, 255, 1 << 32, u64::MAX - 1] {
                let data = prng.bytes(len);
                // own -> reference, and reference -> own
                for own_encrypts in [true, false] {
                    let (enc_alg, dec_alg) = if own_encrypts {
                        (suite.aead_alg, theirs.aead_alg)
                    } else {
                        (theirs.aead_alg, suite.aead_alg)
                    };
                    let (ek, dk) = (
                        derive_traffic_key(expander.as_ref(), enc_alg),
                        derive_traffic_key(expander.as_ref(), dec_alg),
                    );
                    let mut enc = enc_alg.encrypter(ek, iv());
                    let mut dec = dec_alg.decrypter(dk, iv());
                    let record = enc
                        .encrypt(plain(ContentType::ApplicationData, &data), seq)
                        .unwrap();
                    assert_eq!(
                        enc.encrypted_payload_len(len),
                        record.payload.as_ref().len(),
                        "{name}"
                    );
                    let (typ, version, mut body) = encode(record);
                    let opened = dec
                        .decrypt(InboundOpaqueMessage::new(typ, version, &mut body), seq)
                        .unwrap_or_else(|e| {
                            panic!("{name} len {len} seq {seq} own_encrypts {own_encrypts}: {e:?}")
                        });
                    assert_eq!(opened.payload, &data[..], "{name} len {len}");
                    assert_eq!(opened.typ, ContentType::ApplicationData);

                    // The same record under another sequence number, or with a bit flipped in
                    // the ciphertext or the tag, or shortened, fails to open.
                    let record = enc
                        .encrypt(plain(ContentType::ApplicationData, &data), seq)
                        .unwrap();
                    let (typ, version, body) = encode(record);
                    let mut dk2 =
                        dec_alg.decrypter(derive_traffic_key(expander.as_ref(), dec_alg), iv());
                    let mut wrong_seq = body.clone();
                    assert!(
                        dk2.decrypt(
                            InboundOpaqueMessage::new(typ, version, &mut wrong_seq),
                            seq.wrapping_add(1)
                        )
                        .is_err(),
                        "{name} wrong sequence number"
                    );
                    for position in [0, body.len() / 2, body.len() - 1] {
                        let mut flipped = body.clone();
                        flipped[position] ^= 0x01;
                        assert!(
                            dk2.decrypt(InboundOpaqueMessage::new(typ, version, &mut flipped), seq)
                                .is_err(),
                            "{name} flip at {position} of {}",
                            body.len()
                        );
                    }
                    let mut shorter = body[..body.len() - 1].to_vec();
                    assert!(
                        dk2.decrypt(InboundOpaqueMessage::new(typ, version, &mut shorter), seq)
                            .is_err()
                    );
                    let mut tiny = vec![0u8; 15];
                    assert!(
                        dk2.decrypt(InboundOpaqueMessage::new(typ, version, &mut tiny), seq)
                            .is_err()
                    );
                }
            }
        }
    }
}

#[test]
fn tls12_records_cross_between_the_providers() {
    let (own, reference) = (own(), reference());
    let mut prng = Prng(23);
    let mut tested = 0;
    for suite in own.cipher_suites.iter().filter_map(|s| match s {
        SupportedCipherSuite::Tls12(s) => Some(*s),
        SupportedCipherSuite::Tls13(_) => None,
    }) {
        let theirs = match same_suite(&reference, suite.common.suite) {
            SupportedCipherSuite::Tls12(s) => s,
            SupportedCipherSuite::Tls13(_) => panic!(),
        };
        let shape = suite.aead_alg.key_block_shape();
        let other = theirs.aead_alg.key_block_shape();
        assert_eq!(
            (
                shape.enc_key_len,
                shape.fixed_iv_len,
                shape.explicit_nonce_len
            ),
            (
                other.enc_key_len,
                other.fixed_iv_len,
                other.explicit_nonce_len
            ),
            "{:?}",
            suite.common.suite
        );
        // The 128-bit key cannot be made outside `rustls` (its constructor is private); the
        // 256-bit suites and ChaCha20-Poly1305 run here, AES-128-GCM in the handshake tests.
        if shape.enc_key_len != 32 {
            continue;
        }
        let name = format!("{:?}", suite.common.suite);
        let key = || rustls::crypto::cipher::AeadKey::from([0x5au8; 32]);
        let fixed = prng.bytes(shape.fixed_iv_len);
        let explicit = prng.bytes(shape.explicit_nonce_len);
        for len in (0..=100).chain([1000, 16384]) {
            for seq in [0u64, 1, 7, 1 << 40] {
                let data = prng.bytes(len);
                for own_encrypts in [true, false] {
                    let (enc_alg, dec_alg) = if own_encrypts {
                        (suite.aead_alg, theirs.aead_alg)
                    } else {
                        (theirs.aead_alg, suite.aead_alg)
                    };
                    let mut enc = enc_alg.encrypter(key(), &fixed, &explicit);
                    let mut dec = dec_alg.decrypter(key(), &fixed);
                    let message = OutboundPlainMessage {
                        typ: ContentType::ApplicationData,
                        version: ProtocolVersion::TLSv1_2,
                        payload: OutboundChunks::Single(&data),
                    };
                    let record = enc.encrypt(message, seq).unwrap();
                    assert_eq!(
                        enc.encrypted_payload_len(len),
                        record.payload.as_ref().len(),
                        "{name}"
                    );
                    let (typ, version, body) = encode(record);
                    let mut copy = body.clone();
                    let opened = dec
                        .decrypt(InboundOpaqueMessage::new(typ, version, &mut copy), seq)
                        .unwrap_or_else(|e| {
                            panic!("{name} len {len} seq {seq} own_encrypts {own_encrypts}: {e:?}")
                        });
                    assert_eq!(opened.payload, &data[..], "{name} len {len}");
                    for position in [0, body.len() / 2, body.len() - 1] {
                        let mut flipped = body.clone();
                        flipped[position] ^= 0x80;
                        assert!(
                            dec_alg
                                .decrypter(key(), &fixed)
                                .decrypt(InboundOpaqueMessage::new(typ, version, &mut flipped), seq)
                                .is_err(),
                            "{name} flip at {position} of {}",
                            body.len()
                        );
                    }
                    let mut wrong = body.clone();
                    assert!(
                        dec_alg
                            .decrypter(key(), &fixed)
                            .decrypt(InboundOpaqueMessage::new(typ, version, &mut wrong), seq + 1)
                            .is_err(),
                        "{name} wrong sequence number"
                    );
                }
                tested += 1;
            }
        }
    }
    assert!(tested > 100, "{tested}");
}

// Key exchange, random, shape ----------------------------------------------------------------------

#[test]
fn x25519_agrees_with_the_reference_both_ways() {
    let own: &dyn SupportedKxGroup = provider::ALL_KX_GROUPS[0];
    let reference = rustls::crypto::ring::kx_group::X25519;
    assert_eq!(own.name(), NamedGroup::X25519);
    for _ in 0..20 {
        let (a, b) = (own.start().unwrap(), reference.start().unwrap());
        let (pa, pb) = (a.pub_key().to_vec(), b.pub_key().to_vec());
        assert_eq!(pa.len(), 32);
        assert_eq!(a.group(), NamedGroup::X25519);
        let sa = a.complete(&pb).unwrap();
        let sb = b.complete(&pa).unwrap();
        assert_eq!(sa.secret_bytes(), sb.secret_bytes());
    }
    // Two exchanges never share a key.
    let (a, b) = (own.start().unwrap(), own.start().unwrap());
    assert_ne!(a.pub_key(), b.pub_key());
    // Wrong lengths, the all-zero (low-order) value and other small-order points are refused.
    for bad in [vec![], vec![1u8; 31], vec![1u8; 33], vec![0u8; 32]] {
        assert!(own.start().unwrap().complete(&bad).is_err(), "{bad:?}");
    }
    let mut one = [0u8; 32];
    one[0] = 1;
    assert!(
        own.start().unwrap().complete(&one).is_err(),
        "u = 1 has order 4 and 1"
    );
    // The order-8 point of RFC 7748 §6.1 / the p-1 value.
    let mut minus_one = [0xffu8; 32];
    minus_one[0] = 0xec;
    minus_one[31] = 0x7f;
    assert!(own.start().unwrap().complete(&minus_one).is_err());
}

#[test]
fn the_provider_has_the_shape_the_record_says() {
    let own = own();
    let ids: Vec<_> = own
        .cipher_suites
        .iter()
        .map(SupportedCipherSuite::suite)
        .collect();
    assert_eq!(
        ids,
        [
            CipherSuite::TLS13_CHACHA20_POLY1305_SHA256,
            CipherSuite::TLS13_AES_256_GCM_SHA384,
            CipherSuite::TLS13_AES_128_GCM_SHA256,
            CipherSuite::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
            CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
            CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
            CipherSuite::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
            CipherSuite::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
            CipherSuite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
        ]
    );
    let groups: Vec<_> = own.kx_groups.iter().map(|g| g.name()).collect();
    assert_eq!(groups, [NamedGroup::X25519]);
    assert!(own.cipher_suites.iter().all(|s| !s.fips()));
    // Every suite the reference offers is offered, except none: the sets are equal.
    let reference = reference();
    let mut theirs: Vec<_> = reference
        .cipher_suites
        .iter()
        .map(SupportedCipherSuite::suite)
        .collect();
    let mut ours = ids.clone();
    theirs.sort_by_key(|s| u16::from(*s));
    ours.sort_by_key(|s| u16::from(*s));
    assert_eq!(ours, theirs);
}

#[test]
fn the_random_source_fills_and_differs_and_the_key_provider_refuses() {
    let own = own();
    let mut a = [0u8; 64];
    let mut b = [0u8; 64];
    own.secure_random.fill(&mut a).unwrap();
    own.secure_random.fill(&mut b).unwrap();
    assert_ne!(a, b);
    assert_ne!(a, [0u8; 64]);
    own.secure_random.fill(&mut []).unwrap();
    let mut big = vec![0u8; 1 << 20];
    own.secure_random.fill(&mut big).unwrap();
    // Not all zero in any 4 KiB block, a cheap guard against a partial fill.
    assert!(big.chunks(4096).all(|c| c.iter().any(|&x| x != 0)));

    // A client provider holds no key: every format is refused.
    use rustls::pki_types::{
        PrivateKeyDer, PrivatePkcs1KeyDer, PrivatePkcs8KeyDer, PrivateSec1KeyDer,
    };
    for key in [
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(vec![0u8; 8])),
        PrivateKeyDer::Pkcs1(PrivatePkcs1KeyDer::from(vec![0u8; 8])),
        PrivateKeyDer::Sec1(PrivateSec1KeyDer::from(vec![0u8; 8])),
    ] {
        assert!(own.key_provider.load_private_key(key).is_err());
    }
    let rng = SystemRandom::new();
    let pkcs8 =
        EcdsaKeyPair::generate_pkcs8(&signature::ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
    let real = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pkcs8.as_ref().to_vec()));
    assert!(
        own.key_provider.load_private_key(real).is_err(),
        "even a valid key"
    );
}
