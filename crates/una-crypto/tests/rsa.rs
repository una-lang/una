//! RSA PKCS#1 v1.5 and PSS verification: Wycheproof (PKCS#1 v1.5 for 2048 to 8192 bits and SHA-256,
//! SHA-384 and SHA-512; PSS with MGF1), the NIST CAVP SigVer vectors, known answers made with
//! plain big-integer arithmetic, differential tests against `ring` (signatures made by `ring`,
//! then mutated signatures, messages, keys), and the key, signature and DER policy cases.

// Test code: unwrap, indexing and panics are how a test fails (clippy.toml allows them in `#[test]`
// functions only, not in their helpers).
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]

mod common;

use common::{Rng, hex, text, to_hex, vector_path, wycheproof};
use ring::rand::SystemRandom;
use ring::signature as rs;
use serde_json::Value;
use una_crypto::rsa::{HashAlg, PublicKey, verify_pkcs1, verify_pss};

fn hash_of(name: &str) -> Option<HashAlg> {
    match name {
        "SHA-256" | "sha256" | "SHA256" => Some(HashAlg::Sha256),
        "SHA-384" | "sha384" | "SHA384" => Some(HashAlg::Sha384),
        "SHA-512" | "sha512" | "SHA512" => Some(HashAlg::Sha512),
        _ => None,
    }
}

const HASHES: [HashAlg; 3] = [HashAlg::Sha256, HashAlg::Sha384, HashAlg::Sha512];

fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    match content.len() {
        0..=127 => out.push(content.len() as u8),
        128..=255 => out.extend_from_slice(&[0x81, content.len() as u8]),
        n => out.extend_from_slice(&[0x82, (n >> 8) as u8, n as u8]),
    }
    out.extend_from_slice(content);
    out
}

/// A DER INTEGER for the unsigned big-endian magnitude `v` (no leading zeros).
fn der_uint(v: &[u8]) -> Vec<u8> {
    if v[0] & 0x80 != 0 {
        tlv(0x02, &[&[0u8][..], v].concat())
    } else {
        tlv(0x02, v)
    }
}

/// Hex of a number written without leading zeros, possibly with an odd number of digits.
fn hex_number(s: &str) -> Vec<u8> {
    hex(&format!("{}{s}", if s.len() % 2 == 1 { "0" } else { "" }))
}

fn strip_zeros(v: &[u8]) -> &[u8] {
    let skip = v.iter().take_while(|&&b| b == 0).count();
    &v[skip..]
}

/// `RSAPublicKey` DER.
fn rsa_public_key(n: &[u8], e: &[u8]) -> Vec<u8> {
    tlv(
        0x30,
        &[der_uint(strip_zeros(n)), der_uint(strip_zeros(e))].concat(),
    )
}

const RSA_ENCRYPTION: [u8; 15] = [
    0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00,
];

fn spki(rsa_key: &[u8]) -> Vec<u8> {
    tlv(
        0x30,
        &[
            RSA_ENCRYPTION.to_vec(),
            tlv(0x03, &[&[0u8][..], rsa_key].concat()),
        ]
        .concat(),
    )
}

/// The policy a verifier must apply, written independently of the implementation.
fn key_in_policy(n: &[u8], e: u64) -> bool {
    let n = strip_zeros(n);
    let bits = n.len() * 8 - n.first().map_or(0, |b| b.leading_zeros() as usize);
    (2048..=8192).contains(&bits)
        && n.last().is_some_and(|b| b & 1 == 1)
        && e & 1 == 1
        && (65537..1 << 33).contains(&e)
}

fn be_u64(e: &[u8]) -> u64 {
    strip_zeros(e).iter().fold(0, |a, &b| a << 8 | u64::from(b))
}

fn verify(pss: bool, hash: HashAlg, key: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    if pss {
        verify_pss(hash, key, msg, sig).is_ok()
    } else {
        verify_pkcs1(hash, key, msg, sig).is_ok()
    }
}

// ----------------------------------------------------------------------------------------------
// Wycheproof

#[test]
fn wycheproof_pkcs1_v15() {
    let mut files = 0;
    let (mut total, mut accepted, mut refused_by_key_policy, mut missing_null) = (0, 0, 0, 0);
    for bits in [2048, 3072, 4096, 8192] {
        for sha in ["sha256", "sha384", "sha512"] {
            let file = format!("rsa_signature_{bits}_{sha}_test.json");
            if !std::path::Path::new(&vector_path(&format!("wycheproof/{file}"))).exists() {
                continue;
            }
            files += 1;
            let root = wycheproof(&file);
            let mut ran = 0;
            for group in root["testGroups"].as_array().unwrap() {
                let hash = hash_of(text(group, "sha")).unwrap();
                let n = hex(text(&group["publicKey"], "modulus"));
                let e = hex(text(&group["publicKey"], "publicExponent"));
                let der = rsa_public_key(&n, &e);
                // The SPKI of the file parses to the same key.
                let spki_der = hex(text(group, "publicKeyDer"));
                let in_policy = key_in_policy(&n, be_u64(&e));
                assert_eq!(PublicKey::from_spki_der(&spki_der).is_ok(), in_policy);
                assert_eq!(PublicKey::from_rsa_public_key_der(&der).is_ok(), in_policy);
                for t in group["tests"].as_array().unwrap() {
                    let id = t["tcId"].as_u64().unwrap();
                    let msg = hex(text(t, "msg"));
                    let sig = hex(text(t, "sig"));
                    let ok = verify_pkcs1(hash, &der, &msg, &sig).is_ok();
                    let ok_spki = PublicKey::from_spki_der(&spki_der)
                        .is_ok_and(|k| k.verify_pkcs1(hash, &msg, &sig).is_ok());
                    assert_eq!(ok, ok_spki, "{file} tc {id}: key forms differ");
                    ran += 1;
                    match text(t, "result") {
                        "valid" if in_policy => {
                            assert!(ok, "{file} tc {id} should verify");
                            accepted += 1;
                        }
                        // A valid signature under a key the policy refuses (e = 3).
                        "valid" => {
                            assert!(!ok, "{file} tc {id}: key outside the policy");
                            refused_by_key_policy += 1;
                        }
                        "invalid" => {
                            assert!(!ok, "{file} tc {id} must be rejected: {}", t["comment"]);
                        }
                        // The only `acceptable` vector of these files omits the NULL of the
                        // DigestInfo; one exact encoding is accepted, so it is refused.
                        "acceptable" => {
                            assert!(
                                t["flags"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .any(|f| f == "MissingNull")
                            );
                            assert!(!ok, "{file} tc {id}: missing NULL must be rejected");
                            missing_null += 1;
                        }
                        other => panic!("{file} tc {id}: unexpected result {other}"),
                    }
                }
            }
            assert_eq!(
                ran,
                root["numberOfTests"].as_u64().unwrap() as usize,
                "{file}"
            );
            total += ran;
        }
    }
    assert_eq!(files, 12);
    assert_eq!(missing_null, 12);
    assert!(refused_by_key_policy > 0, "the e = 3 groups ran");
    assert_eq!(total, 3_106, "vector count");
    assert!(accepted > 80, "valid vectors ran: {accepted}");
}

#[test]
fn wycheproof_pss() {
    let files = [
        "rsa_pss_2048_sha256_mgf1_0_test.json",
        "rsa_pss_2048_sha256_mgf1_32_test.json",
        "rsa_pss_2048_sha384_mgf1_48_test.json",
        "rsa_pss_3072_sha256_mgf1_32_test.json",
        "rsa_pss_4096_sha256_mgf1_32_test.json",
        "rsa_pss_4096_sha384_mgf1_48_test.json",
        "rsa_pss_4096_sha512_mgf1_32_test.json",
        "rsa_pss_4096_sha512_mgf1_64_test.json",
        "rsa_pss_misc_test.json",
    ];
    let (mut total, mut accepted, mut refused_by_policy, mut skipped_hash) = (0, 0, 0, 0);
    let mut rejected_invalid = 0;
    for file in files {
        let root = wycheproof(file);
        let mut ran = 0;
        for group in root["testGroups"].as_array().unwrap() {
            let n = hex(text(&group["publicKey"], "modulus"));
            let e = hex(text(&group["publicKey"], "publicExponent"));
            let der = rsa_public_key(&n, &e);
            let hash = hash_of(text(group, "sha"));
            let mgf_hash = hash_of(text(group, "mgfSha"));
            let s_len = group["sLen"].as_u64().unwrap() as usize;
            for t in group["tests"].as_array().unwrap() {
                let id = t["tcId"].as_u64().unwrap();
                let msg = hex(text(t, "msg"));
                let sig = hex(text(t, "sig"));
                ran += 1;
                let result = text(t, "result");
                let Some(hash) = hash else {
                    // SHA-1 or SHA-224: hashes this crate does not offer, nothing to call.
                    skipped_hash += 1;
                    continue;
                };
                let ok = verify_pss(hash, &der, &msg, &sig).is_ok();
                // The policy: MGF1 over the same hash, salt as long as the digest.
                let supported = mgf_hash == Some(hash) && s_len == hash.output_len();
                match result {
                    "valid" if supported => {
                        assert!(ok, "{file} tc {id} should verify: {}", t["comment"]);
                        accepted += 1;
                    }
                    "valid" => {
                        assert!(!ok, "{file} tc {id}: salt/MGF outside the policy");
                        refused_by_policy += 1;
                    }
                    // With the parameters of the file the signature is invalid. A group whose
                    // salt length is not the digest length (a vector made with the salt length
                    // changed to the digest length) can hold a signature that is valid with the
                    // salt this crate requires; only groups inside the policy are asserted.
                    "invalid" if supported => {
                        assert!(!ok, "{file} tc {id} must be rejected: {}", t["comment"]);
                        rejected_invalid += 1;
                    }
                    "invalid" => {}
                    other => panic!("{file} tc {id}: unexpected result {other}"),
                }
            }
        }
        assert_eq!(
            ran,
            root["numberOfTests"].as_u64().unwrap() as usize,
            "{file}"
        );
        total += ran;
    }
    assert_eq!(total, 1_215, "vector count");
    assert!(accepted > 500, "valid vectors ran: {accepted}");
    assert!(
        refused_by_policy > 100,
        "other salt lengths ran: {refused_by_policy}"
    );
    assert!(
        rejected_invalid > 200,
        "invalid vectors ran: {rejected_invalid}"
    );
    assert_eq!(
        skipped_hash, 60,
        "the SHA-1 and SHA-224 vectors of the misc file"
    );
}

// ----------------------------------------------------------------------------------------------
// NIST CAVP

fn nist() -> Value {
    serde_json::from_str(&std::fs::read_to_string(vector_path("nist_cavp_rsa.json")).unwrap())
        .unwrap()
}

#[test]
fn nist_cavp_sigver() {
    let doc = nist();
    let (mut accepted_1, mut accepted_p) = (0, 0);
    for (scheme, pss) in [("pkcs1", false), ("pss", true)] {
        let mut ran = 0;
        for row in doc[scheme].as_array().unwrap() {
            let hash = hash_of(text(row, "sha")).unwrap();
            let n = hex_number(text(row, "n"));
            let e = hex_number(text(row, "e"));
            let der = rsa_public_key(&n, &e);
            let msg = hex(text(row, "msg"));
            let sig = hex(text(row, "sig"));
            let ok = verify(pss, hash, &der, &msg, &sig);
            let salt_ok = !pss || row["salt_len"].as_u64().unwrap() as usize == hash.output_len();
            let expected =
                row["pass"].as_bool().unwrap() && key_in_policy(&n, be_u64(&e)) && salt_ok;
            assert_eq!(ok, expected, "{scheme} mod {} {}", row["mod"], row["sha"]);
            if ok {
                if pss {
                    accepted_p += 1
                } else {
                    accepted_1 += 1
                }
            }
            ran += 1;
        }
        assert_eq!(ran, 162, "{scheme} rows");
    }
    // 2048- and 3072-bit keys, three hashes, three valid rows each; PSS: only the 2048-bit rows
    // have a salt as long as the digest.
    assert_eq!(accepted_1, 18);
    assert_eq!(accepted_p, 9);
}

// ----------------------------------------------------------------------------------------------
// Known answers (plain big-integer arithmetic, hashlib), including moduli of 8k + 1 bits, where
// the PSS encoded message is one byte shorter than the signature, and unusual exponents.

fn keys_file() -> Value {
    serde_json::from_str(&std::fs::read_to_string(vector_path("rsa_test_keys.json")).unwrap())
        .unwrap()
}

#[test]
fn known_answers() {
    let doc = keys_file();
    let (mut accepted, mut refused_salt, mut refused_key) = (0, 0, 0);
    let mut seen_exponents = Vec::new();
    for key in doc["keys"].as_array().unwrap() {
        let name = text(key, "name");
        let n = hex_number(text(key, "n"));
        let e = key["e"].as_u64().unwrap();
        let der = rsa_public_key(&n, &e.to_be_bytes());
        let msg = text(key, "message_utf8").as_bytes();
        let in_policy = key_in_policy(&n, e);
        assert_eq!(
            PublicKey::from_rsa_public_key_der(&der).is_ok(),
            in_policy,
            "{name}"
        );
        assert_eq!(
            PublicKey::from_spki_der(&spki(&der)).is_ok(),
            in_policy,
            "{name} spki"
        );
        if in_policy {
            let k = PublicKey::from_rsa_public_key_der(&der).unwrap();
            assert_eq!(k.modulus_bits(), key["bits"].as_u64().unwrap() as usize);
            assert_eq!(k.modulus_len(), n.len());
            seen_exponents.push(e);
        }
        for kat in key["kats"].as_array().unwrap() {
            let hash = hash_of(text(kat, "hash")).unwrap();
            let pss = text(kat, "scheme") == "pss";
            let sig = hex(text(kat, "sig"));
            let ok = verify(pss, hash, &der, msg, &sig);
            let salt_ok = !pss || kat["salt_len"].as_u64().unwrap() as usize == hash.output_len();
            assert_eq!(
                ok,
                in_policy && salt_ok,
                "{name} {} {:?}",
                text(kat, "scheme"),
                hash
            );
            if ok {
                accepted += 1;
            } else if !in_policy {
                refused_key += 1;
            } else {
                refused_salt += 1;
            }
            if in_policy && salt_ok {
                // Wrong message, wrong hash, wrong scheme.
                assert!(!verify(pss, hash, &der, b"another message", &sig), "{name}");
                for other in HASHES.into_iter().filter(|h| *h != hash) {
                    assert!(!verify(pss, other, &der, msg, &sig), "{name} wrong hash");
                }
                assert!(!verify(!pss, hash, &der, msg, &sig), "{name} wrong scheme");
            }
        }
    }
    seen_exponents.sort_unstable();
    seen_exponents.dedup();
    // 65537, 65539, 2^32 + 1 and 2^33 - 1 verify; 3, 65535 and 2^33 + 1 are refused.
    assert_eq!(seen_exponents, [65537, 65539, 4_294_967_297, 8_589_934_591]);
    assert_eq!(accepted, 13 * 6);
    assert!(refused_salt > 50 && refused_key > 0);
}

// ----------------------------------------------------------------------------------------------
// Differential tests against ring

struct Pair {
    pss: bool,
    hash: HashAlg,
    padding: &'static dyn rs::RsaEncoding,
    verify_alg: &'static rs::RsaParameters,
}

fn pairs() -> Vec<Pair> {
    vec![
        Pair {
            pss: false,
            hash: HashAlg::Sha256,
            padding: &rs::RSA_PKCS1_SHA256,
            verify_alg: &rs::RSA_PKCS1_2048_8192_SHA256,
        },
        Pair {
            pss: false,
            hash: HashAlg::Sha384,
            padding: &rs::RSA_PKCS1_SHA384,
            verify_alg: &rs::RSA_PKCS1_2048_8192_SHA384,
        },
        Pair {
            pss: false,
            hash: HashAlg::Sha512,
            padding: &rs::RSA_PKCS1_SHA512,
            verify_alg: &rs::RSA_PKCS1_2048_8192_SHA512,
        },
        Pair {
            pss: true,
            hash: HashAlg::Sha256,
            padding: &rs::RSA_PSS_SHA256,
            verify_alg: &rs::RSA_PSS_2048_8192_SHA256,
        },
        Pair {
            pss: true,
            hash: HashAlg::Sha384,
            padding: &rs::RSA_PSS_SHA384,
            verify_alg: &rs::RSA_PSS_2048_8192_SHA384,
        },
        Pair {
            pss: true,
            hash: HashAlg::Sha512,
            padding: &rs::RSA_PSS_SHA512,
            verify_alg: &rs::RSA_PSS_2048_8192_SHA512,
        },
    ]
}

fn ring_ok(p: &Pair, key: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    rs::UnparsedPublicKey::new(p.verify_alg, key)
        .verify(msg, sig)
        .is_ok()
}

/// Signatures that exercise the edges of the range check, for a modulus `n` of `len` bytes.
fn edge_signatures(n: &[u8], len: usize) -> Vec<Vec<u8>> {
    let mut out = vec![vec![0u8; len], vec![0xffu8; len]];
    let mut one = vec![0u8; len];
    one[len - 1] = 1;
    out.push(one);
    out.push(n.to_vec()); // s = n
    let mut below = n.to_vec();
    below[len - 1] ^= 1; // n - 1 (n is odd)
    out.push(below);
    let mut above = n.to_vec();
    above[len - 1] = above[len - 1].wrapping_add(2); // n + 2 or wrapped
    out.push(above);
    out
}

/// Mutations of a valid `(message, signature)` pair for a modulus `n`: bit flips, truncation,
/// extension, complement, empty, message changes and the range edges. None of them is valid.
fn mutations(r: &mut Rng, n: &[u8], msg: &[u8], sig: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut cases: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for _ in 0..4 {
        let mut s = sig.to_vec();
        let i = r.below(s.len());
        s[i] ^= 1 << r.below(8);
        cases.push((msg.to_vec(), s));
    }
    let mut s = sig.to_vec();
    s[0] ^= 0x80;
    cases.push((msg.to_vec(), s));
    let mut s = sig.to_vec();
    *s.last_mut().unwrap() ^= 1;
    cases.push((msg.to_vec(), s));
    cases.push((msg.to_vec(), sig[1..].to_vec()));
    cases.push((msg.to_vec(), [sig, &[0u8][..]].concat()));
    cases.push((msg.to_vec(), [&[0u8][..], sig].concat()));
    cases.push((msg.to_vec(), sig.iter().map(|b| !b).collect()));
    cases.push((msg.to_vec(), Vec::new()));
    let mut m = msg.to_vec();
    m.push(0);
    cases.push((m, sig.to_vec()));
    if !msg.is_empty() {
        let mut m = msg.to_vec();
        m[0] ^= 1;
        cases.push((m, sig.to_vec()));
    }
    for s in edge_signatures(n, n.len()) {
        cases.push((msg.to_vec(), s));
    }
    cases
}

/// ring's verdict and ours agree on `(message, signature)`.
fn same_verdict(p: &Pair, key: &[u8], m: &[u8], s: &[u8], what: &str) -> bool {
    let (a, b) = (verify(p.pss, p.hash, key, m, s), ring_ok(p, key, m, s));
    assert_eq!(a, b, "{what}: verdicts differ ({:?} pss {})", p.hash, p.pss);
    a
}

#[test]
fn differential_against_ring_on_signatures_made_by_ring() {
    let rng = SystemRandom::new();
    let doc = keys_file();
    let mut r = Rng(0x5eed_1234_abcd_ef01);
    let (mut signed, mut mutated) = (0, 0);
    let mut sizes = Vec::new();
    for key in doc["keys"].as_array().unwrap() {
        let name = text(key, "name");
        if key["e"].as_u64().unwrap() != 65537 {
            continue;
        }
        // ring signs with moduli of a multiple of 512 bits up to 4096 only; the other sizes are covered by
        // the known answers and the next test.
        let Ok(kp) = rs::RsaKeyPair::from_der(&hex(text(key, "rsa_private_key_der"))) else {
            continue;
        };
        let pub_der = kp.public().as_ref().to_vec();
        let n = hex_number(text(key, "n"));
        let modulus_len = kp.public().modulus_len();
        assert_eq!(n.len(), modulus_len);
        sizes.push(key["bits"].as_u64().unwrap());
        // Few rounds for the big keys.
        let rounds = if modulus_len > 600 { 2 } else { 6 };
        for p in pairs() {
            for round in 0..rounds {
                let msg = r.vec(300);
                let mut sig = vec![0u8; modulus_len];
                kp.sign(p.padding, &rng, &msg, &mut sig).unwrap();
                assert!(ring_ok(&p, &pub_der, &msg, &sig));
                assert!(
                    verify(p.pss, p.hash, &pub_der, &msg, &sig),
                    "{name} round {round}"
                );
                signed += 1;
                for (m, s) in mutations(&mut r, &n, &msg, &sig) {
                    assert!(!same_verdict(&p, &pub_der, &m, &s, name));
                    mutated += 1;
                }
                // A signature of the other scheme or another hash verifies under neither.
                for other in pairs() {
                    if other.pss == p.pss && other.hash == p.hash {
                        continue;
                    }
                    assert!(!same_verdict(&other, &pub_der, &msg, &sig, name));
                    mutated += 1;
                }
            }
        }
        // Key mutations: the verdict (always a refusal here) agrees with ring's.
        let p = &pairs()[0];
        let msg = b"key mutation";
        let mut sig = vec![0u8; modulus_len];
        kp.sign(p.padding, &rng, msg, &mut sig).unwrap();
        let mut variants: Vec<Vec<u8>> = Vec::new();
        for _ in 0..6 {
            let mut n2 = n.clone();
            let i = r.below(n2.len());
            n2[i] ^= 1 << r.below(8);
            variants.push(rsa_public_key(&n2, &[1, 0, 1]));
        }
        let mut n_plus = n.clone();
        *n_plus.last_mut().unwrap() ^= 2; // another odd modulus of the same size
        variants.push(rsa_public_key(&n_plus, &[1, 0, 1]));
        for e in [
            &[1u8, 0, 3][..],
            &[3][..],
            &[1, 0, 0][..],
            &[1, 0, 0, 0, 1, 1][..],
        ] {
            variants.push(rsa_public_key(&n, e));
        }
        for v in variants {
            assert!(!same_verdict(p, &v, msg, &sig, name));
            mutated += 1;
        }
    }
    assert!(signed >= 100 && mutated > 2_000, "{signed} {mutated}");
    for bits in [2048, 3072, 4096] {
        assert!(
            sizes.contains(&bits),
            "no ring-signed key of {bits} bits: {sizes:?}"
        );
    }
}

#[test]
fn differential_against_ring_on_known_answers_of_every_size() {
    // Keys ring cannot sign with (moduli of 2049, 2050, 2056, 2057 and 3073 bits, and the
    // unusual exponents): ring verifies the known answers, and the verdicts agree on their
    // mutations. Salt lengths other than the digest length and out-of-policy exponents are the
    // documented differences and are asserted here too.
    let doc = keys_file();
    let mut r = Rng(0xdead_beef_0bad_f00d);
    let (mut agreed, mut different_policy) = (0, 0);
    for key in doc["keys"].as_array().unwrap() {
        let name = text(key, "name");
        let n = hex_number(text(key, "n"));
        let e = key["e"].as_u64().unwrap();
        let der = rsa_public_key(&n, &e.to_be_bytes());
        let msg = text(key, "message_utf8").as_bytes();
        let in_policy = key_in_policy(&n, e);
        for kat in key["kats"].as_array().unwrap() {
            let hash = hash_of(text(kat, "hash")).unwrap();
            let pss = text(kat, "scheme") == "pss";
            let p = pairs()
                .into_iter()
                .find(|p| p.pss == pss && p.hash == hash)
                .unwrap();
            let sig = hex(text(kat, "sig"));
            let ours = verify(pss, hash, &der, msg, &sig);
            let ring = ring_ok(&p, &der, msg, &sig);
            let salt_ok = !pss || kat["salt_len"].as_u64().unwrap() as usize == hash.output_len();
            // ring takes e >= 3 up to 33 bits and, for PSS, only a salt as long as the digest.
            assert_eq!(
                ring,
                e < 1 << 33 && salt_ok,
                "{name} ring on a valid signature {} {} {}",
                text(kat, "scheme"),
                text(kat, "hash"),
                kat["salt_len"]
            );
            assert_eq!(ours, in_policy && salt_ok, "{name}");
            if !(in_policy && salt_ok) {
                different_policy += 1;
                continue;
            }
            for (m, s) in mutations(&mut r, &n, msg, &sig) {
                assert!(!same_verdict(&p, &der, &m, &s, name));
                agreed += 1;
            }
        }
    }
    assert!(
        agreed > 1_000 && different_policy > 100,
        "{agreed} {different_policy}"
    );
}

#[test]
fn pss_salt_length_equals_the_digest_length_as_in_ring() {
    // Signatures with a salt length other than the digest length are valid PSS, and both ring
    // and this crate refuse them (RFC 8446 section 4.2.3 asks for a salt as long as the digest).
    let doc = keys_file();
    let key = doc["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| text(k, "name") == "k2048")
        .unwrap();
    let n = hex_number(text(key, "n"));
    let der = rsa_public_key(&n, &[1, 0, 1]);
    let msg = text(key, "message_utf8").as_bytes();
    let (mut other_salt, mut same_salt) = (0, 0);
    for kat in key["kats"].as_array().unwrap() {
        if text(kat, "scheme") != "pss" {
            continue;
        }
        let hash = hash_of(text(kat, "hash")).unwrap();
        let sig = hex(text(kat, "sig"));
        let p = pairs()
            .into_iter()
            .find(|p| p.pss && p.hash == hash)
            .unwrap();
        let ring = ring_ok(&p, &der, msg, &sig);
        let ours = verify(true, hash, &der, msg, &sig);
        assert_eq!(ring, ours, "salt {}", kat["salt_len"]);
        if kat["salt_len"].as_u64().unwrap() as usize == hash.output_len() {
            assert!(ours);
            same_salt += 1;
        } else {
            assert!(!ours);
            other_salt += 1;
        }
    }
    assert_eq!((same_salt, other_salt), (3, 12));
}

// ----------------------------------------------------------------------------------------------
// Policy: keys, DER, signature range

fn modulus_of_bits(bits: usize) -> Vec<u8> {
    // An odd modulus-shaped number of exactly `bits` bits (not a product of primes: the policy
    // checks do not need one).
    let len = bits.div_ceil(8);
    let mut n = vec![0xa5u8; len];
    let top = bits - 8 * (len - 1);
    n[0] = ((1u16 << top) - 1) as u8;
    n[0] |= 1 << (top - 1);
    *n.last_mut().unwrap() |= 1;
    n
}

#[test]
fn modulus_and_exponent_policy() {
    let e = [1u8, 0, 1];
    for bits in [2047, 2048, 2049, 4096, 8191, 8192, 8193, 1024, 1] {
        let n = modulus_of_bits(bits);
        let expected = (2048..=8192).contains(&bits);
        assert_eq!(
            PublicKey::from_components(&n, &e).is_ok(),
            expected,
            "{bits}"
        );
        assert_eq!(
            PublicKey::from_rsa_public_key_der(&rsa_public_key(&n, &e)).is_ok(),
            expected,
            "{bits} der"
        );
    }
    let n = modulus_of_bits(2048);
    // Even modulus.
    let mut even = n.clone();
    *even.last_mut().unwrap() &= !1;
    assert!(PublicKey::from_components(&even, &e).is_err());
    // Exponents.
    let ok = |e: u64| {
        let b = e.to_be_bytes();
        PublicKey::from_components(&n, strip_zeros(&b)).is_ok()
    };
    for (e, expected) in [
        (1, false),
        (3, false),
        (17, false),
        (65535, false),
        (65536, false),
        (65537, true),
        (65538, false),
        (65539, true),
        ((1 << 32) + 1, true),
        ((1 << 33) - 3, true),
        ((1 << 33) - 1, true),
        (1 << 33, false),
        ((1 << 33) + 1, false),
        (u64::MAX, false),
    ] {
        assert_eq!(ok(e), expected, "e = {e}");
    }
    // Empty, zero and zero-padded components.
    assert!(PublicKey::from_components(&n, &[]).is_err());
    assert!(PublicKey::from_components(&n, &[0, 1, 0, 1]).is_err());
    assert!(PublicKey::from_components(&[], &e).is_err());
    assert!(PublicKey::from_components(&[&[0u8][..], &n].concat(), &e).is_err());
    assert!(PublicKey::from_components(&n, &[1, 0, 0, 0, 0, 1]).is_err());
}

#[test]
fn der_forms_are_strict() {
    let n = modulus_of_bits(2048);
    let e = [1u8, 0, 1];
    let good = rsa_public_key(&n, &e);
    assert!(PublicKey::from_rsa_public_key_der(&good).is_ok());
    assert!(PublicKey::from_spki_der(&spki(&good)).is_ok());
    let seq = |parts: &[Vec<u8>]| tlv(0x30, &parts.concat());
    let mut bad: Vec<(&str, Vec<u8>)> = Vec::new();
    bad.push(("empty", Vec::new()));
    bad.push(("trailing byte", [&good[..], &[0u8][..]].concat()));
    bad.push(("truncated", good[..good.len() - 1].to_vec()));
    bad.push(("wrong outer tag", {
        let mut v = good.clone();
        v[0] = 0x31;
        v
    }));
    bad.push((
        "extra element",
        seq(&[der_uint(&n), der_uint(&e), der_uint(&[1])]),
    ));
    bad.push(("one element", seq(&[der_uint(&n)])));
    bad.push((
        "negative modulus",
        seq(&[tlv(0x02, &n), der_uint(&e)]).to_vec(),
    ));
    bad.push((
        "negative exponent",
        seq(&[der_uint(&n), tlv(0x02, &[0x81, 0, 1])]),
    ));
    bad.push((
        "padded modulus",
        seq(&[tlv(0x02, &[&[0u8, 0][..], &n].concat()), der_uint(&e)]),
    ));
    bad.push((
        "padded exponent",
        seq(&[der_uint(&n), tlv(0x02, &[0, 1, 0, 1])]),
    ));
    bad.push(("zero exponent", seq(&[der_uint(&n), tlv(0x02, &[0])])));
    bad.push((
        "exponent as OCTET STRING",
        seq(&[der_uint(&n), tlv(0x04, &e)]),
    ));
    bad.push(("modulus as BIT STRING", seq(&[tlv(0x03, &n), der_uint(&e)])));
    // Non-minimal lengths: the outer length in the long form where the short form fits cannot
    // arise for a key this long, so use a three-byte length and a padded two-byte length.
    let mut long_len = good.clone();
    long_len.splice(1..4, [0x83, good[2], good[3]]);
    bad.push(("three-byte length", long_len));
    let mut indefinite = vec![0x30, 0x80];
    indefinite.extend_from_slice(&good[4..]);
    indefinite.extend_from_slice(&[0, 0]);
    bad.push(("indefinite length", indefinite));
    for (name, der) in &bad {
        assert!(PublicKey::from_rsa_public_key_der(der).is_err(), "{name}");
    }
    // SubjectPublicKeyInfo variants.
    let alg_rsa = RSA_ENCRYPTION.to_vec();
    let bits = |k: &[u8]| tlv(0x03, &[&[0u8][..], k].concat());
    let with = |alg: Vec<u8>, bit_string: Vec<u8>| tlv(0x30, &[alg, bit_string].concat());
    let mut no_null = alg_rsa.clone();
    no_null.truncate(13);
    no_null[1] = 0x0b;
    let mut pss_oid = alg_rsa.clone();
    pss_oid[11] = 0x0a; // 1.2.840.113549.1.1.10
    let mut ec_oid = alg_rsa.clone();
    ec_oid[8] = 0x2b;
    let mut spki_bad: Vec<(&str, Vec<u8>)> = vec![
        ("no NULL", with(no_null, bits(&good))),
        ("PSS key OID", with(pss_oid, bits(&good))),
        ("other OID", with(ec_oid, bits(&good))),
        (
            "unused bits",
            with(
                alg_rsa.clone(),
                tlv(0x03, &[&[1u8][..], &good[..]].concat()),
            ),
        ),
        (
            "no unused-bits byte",
            with(alg_rsa.clone(), tlv(0x03, &good)),
        ),
        (
            "trailing in bit string",
            with(alg_rsa.clone(), bits(&[&good[..], &[0u8][..]].concat())),
        ),
        (
            "octet string",
            with(
                alg_rsa.clone(),
                tlv(0x04, &[&[0u8][..], &good[..]].concat()),
            ),
        ),
        ("trailing after", [&spki(&good)[..], &[0u8][..]].concat()),
        ("bare RSAPublicKey", good.clone()),
    ];
    spki_bad.push((
        "key in policy but SPKI of the wrong kind",
        spki(&spki(&good)),
    ));
    for (name, der) in &spki_bad {
        assert!(PublicKey::from_spki_der(der).is_err(), "{name}");
    }
    assert!(PublicKey::from_rsa_public_key_der(&spki(&good)).is_err());
}

#[test]
fn signature_range_and_length() {
    let doc = keys_file();
    let key = &doc["keys"].as_array().unwrap()[0];
    assert_eq!(text(key, "name"), "k2048");
    let n = hex_number(text(key, "n"));
    let der = rsa_public_key(&n, &[1, 0, 1]);
    let msg = text(key, "message_utf8").as_bytes();
    let sig = hex(text(&key["kats"][0], "sig"));
    assert!(verify_pkcs1(HashAlg::Sha256, &der, msg, &sig).is_ok());
    // Wrong lengths: one short, one long, empty, double.
    for bad in [
        sig[1..].to_vec(),
        [&sig[..], &[0u8][..]].concat(),
        Vec::new(),
        [&sig[..], &sig[..]].concat(),
        [&[0u8][..], &sig[..]].concat(),
    ] {
        assert!(verify_pkcs1(HashAlg::Sha256, &der, msg, &bad).is_err());
    }
    // s = 0, 1, n - 1, n, n + 1, 2^bits - 1 are never accepted; s + n (same value mod n) neither.
    for s in edge_signatures(&n, n.len()) {
        assert!(verify_pkcs1(HashAlg::Sha256, &der, msg, &s).is_err());
        assert!(verify_pss(HashAlg::Sha256, &der, msg, &s).is_err());
    }
    // The congruent signature s + n has the same residue but is not below n; it overflows the
    // byte length for most s, so check the modular twin by subtraction when it fits.
    let mut twin = sig.clone();
    let mut carry = 0u16;
    for i in (0..n.len()).rev() {
        let v = u16::from(twin[i]) + u16::from(n[i]) + carry;
        twin[i] = v as u8;
        carry = v >> 8;
    }
    if carry == 0 {
        assert!(
            verify_pkcs1(HashAlg::Sha256, &der, msg, &twin).is_err(),
            "s + n"
        );
    }
}

#[test]
fn garbage_never_panics() {
    let mut r = Rng(0x0bad_cafe_1234_5678);
    let doc = keys_file();
    let key = &doc["keys"].as_array().unwrap()[0];
    let n = hex_number(text(key, "n"));
    let der = rsa_public_key(&n, &[1, 0, 1]);
    for _ in 0..300 {
        let junk_key = r.vec(400);
        let msg = r.vec(100);
        let sig = r.bytes(256);
        for h in HASHES {
            assert!(verify_pkcs1(h, &junk_key, &msg, &sig).is_err());
            assert!(verify_pss(h, &junk_key, &msg, &sig).is_err());
            assert!(verify_pkcs1(h, &der, &msg, &sig).is_err());
            assert!(verify_pss(h, &der, &msg, &sig).is_err());
        }
    }
    // Keys that parse but come from structured junk.
    for _ in 0..50 {
        let mut n = r.bytes(256);
        n[0] |= 0x80;
        n[255] |= 1;
        let k = rsa_public_key(&n, &[1, 0, 1]);
        for h in HASHES {
            assert!(verify_pkcs1(h, &k, b"m", &r.bytes(256)).is_err());
            assert!(verify_pss(h, &k, b"m", &r.bytes(256)).is_err());
        }
    }
    let _ = to_hex(&[]);
}
