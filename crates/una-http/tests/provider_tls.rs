//! The own TLS provider (ADR 0133 rule 17) in whole handshakes against the `ring` provider of
//! `rustls`, in memory (no sockets), suite by suite and in both directions: own client against
//! a `ring` server, `ring` client against a server whose record protection, hashes, key
//! exchange and random source are the own provider's (its certificate key is loaded by `ring`,
//! because a client provider signs nothing), and own against own. Then tampered records, bad
//! certificate signatures on every chain algorithm, wrong algorithms, and mutated first flights
//! with the verdict compared between the providers.

// Test code: unwrap, indexing and panics are how a test fails.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::io::{Read, Write};
use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair, SignatureAlgorithm};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::{
    ClientConfig, ClientConnection, Connection, Error, RootCertStore, ServerConfig,
    ServerConnection, SupportedCipherSuite, SupportedProtocolVersion,
};
use una_http::provider;

fn own() -> CryptoProvider {
    provider::crypto_provider()
}

fn reference() -> CryptoProvider {
    rustls::crypto::ring::default_provider()
}

// Certificates ---------------------------------------------------------------------------------

/// A key type of a certificate or of the authority that signs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    P256,
    P384,
    Ed25519,
    Rsa256,
    Rsa384,
    Rsa512,
}

impl Kind {
    fn algorithm(self) -> &'static SignatureAlgorithm {
        match self {
            Self::P256 => &rcgen::PKCS_ECDSA_P256_SHA256,
            Self::P384 => &rcgen::PKCS_ECDSA_P384_SHA384,
            Self::Ed25519 => &rcgen::PKCS_ED25519,
            Self::Rsa256 => &rcgen::PKCS_RSA_SHA256,
            Self::Rsa384 => &rcgen::PKCS_RSA_SHA384,
            Self::Rsa512 => &rcgen::PKCS_RSA_SHA512,
        }
    }

    fn is_rsa(self) -> bool {
        matches!(self, Self::Rsa256 | Self::Rsa384 | Self::Rsa512)
    }
}

/// An RSA test key (PKCS#1 `RSAPrivateKey` of the vendored test-only keys of `una-crypto`),
/// wrapped as PKCS#8.
fn rsa_pkcs8(name: &str) -> Vec<u8> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../una-crypto/tests/vectors/rsa_test_keys.json");
    let root: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let key = root["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["name"].as_str() == Some(name))
        .unwrap();
    let hex = key["rsa_private_key_der"].as_str().unwrap();
    let pkcs1: Vec<u8> = (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    // PrivateKeyInfo { version 0, AlgorithmIdentifier { rsaEncryption, NULL }, OCTET STRING }.
    let mut body = vec![0x02, 0x01, 0x00];
    body.extend([
        0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00,
    ]);
    body.push(0x04);
    der_len(&mut body, pkcs1.len());
    body.extend(&pkcs1);
    let mut out = vec![0x30];
    der_len(&mut out, body.len());
    out.extend(body);
    out
}

fn der_len(out: &mut Vec<u8>, len: usize) {
    if len < 128 {
        out.push(len as u8);
    } else if len < 256 {
        out.extend([0x81, len as u8]);
    } else {
        out.extend([0x82, (len >> 8) as u8, len as u8]);
    }
}

fn key_pair(kind: Kind, rsa_name: &str) -> KeyPair {
    if kind.is_rsa() {
        let der = PrivatePkcs8KeyDer::from(rsa_pkcs8(rsa_name));
        KeyPair::from_pkcs8_der_and_sign_algo(&der, kind.algorithm()).unwrap()
    } else {
        KeyPair::generate_for(kind.algorithm()).unwrap()
    }
}

/// A leaf (for `localhost`) signed by a fresh authority.
struct Pki {
    root: CertificateDer<'static>,
    leaf: CertificateDer<'static>,
    leaf_key: Vec<u8>,
}

fn issue(ca: Kind, leaf: Kind) -> Pki {
    let ca_key = key_pair(ca, "k3072");
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let root = ca_params.self_signed(&ca_key).unwrap().der().clone();
    let issuer = Issuer::new(ca_params, ca_key);
    let key = key_pair(leaf, "k2048");
    let params = CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
    let cert = params.signed_by(&key, &issuer).unwrap();
    Pki {
        root,
        leaf: cert.der().clone(),
        leaf_key: key.serialize_der(),
    }
}

fn roots(pki: &Pki) -> Arc<RootCertStore> {
    let mut store = RootCertStore::empty();
    store.add(pki.root.clone()).unwrap();
    Arc::new(store)
}

// In-memory connections ------------------------------------------------------------------------

/// A server certificate resolver that holds one key loaded by `ring` (the own provider is a
/// client provider and loads no key).
#[derive(Debug)]
struct Fixed(Arc<CertifiedKey>);

impl ResolvesServerCert for Fixed {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(Arc::clone(&self.0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Own,
    Ring,
}

fn provider_for(side: Side, suite: SupportedCipherSuite) -> Arc<CryptoProvider> {
    let base = match side {
        Side::Own => own(),
        Side::Ring => reference(),
    };
    Arc::new(CryptoProvider {
        cipher_suites: vec![suite],
        ..base
    })
}

fn version_of(suite: SupportedCipherSuite) -> &'static SupportedProtocolVersion {
    match suite {
        SupportedCipherSuite::Tls13(_) => &rustls::version::TLS13,
        SupportedCipherSuite::Tls12(_) => &rustls::version::TLS12,
    }
}

fn client(side: Side, suite: SupportedCipherSuite, pki: &Pki) -> Connection {
    let config = ClientConfig::builder_with_provider(provider_for(side, suite))
        .with_protocol_versions(&[version_of(suite)])
        .unwrap()
        .with_root_certificates(roots(pki).as_ref().clone())
        .with_no_client_auth();
    Connection::Client(
        ClientConnection::new(Arc::new(config), ServerName::try_from("localhost").unwrap())
            .unwrap(),
    )
}

fn server(side: Side, suite: SupportedCipherSuite, pki: &Pki) -> Connection {
    let provider = provider_for(side, suite);
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pki.leaf_key.clone()));
    let builder = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[version_of(suite)])
        .unwrap()
        .with_no_client_auth();
    let config = match side {
        Side::Ring => builder
            .with_single_cert(vec![pki.leaf.clone()], key)
            .unwrap(),
        Side::Own => {
            let signer = reference().key_provider.load_private_key(key).unwrap();
            let certified = CertifiedKey::new(vec![pki.leaf.clone()], signer);
            builder.with_cert_resolver(Arc::new(Fixed(Arc::new(certified))))
        }
    };
    Connection::Server(ServerConnection::new(Arc::new(config)).unwrap())
}

/// Moves every byte `from` wants to write into `to`, after `tamper` changed it. Returns whether
/// anything moved.
fn flow(
    from: &mut Connection,
    to: &mut Connection,
    tamper: &mut dyn FnMut(&mut Vec<u8>),
) -> Result<bool, Error> {
    let mut bytes = Vec::new();
    while from.wants_write() {
        from.write_tls(&mut bytes).unwrap();
    }
    if bytes.is_empty() {
        return Ok(false);
    }
    tamper(&mut bytes);
    let mut rest = &bytes[..];
    while !rest.is_empty() {
        let n = to.read_tls(&mut rest).unwrap();
        if n == 0 {
            break;
        }
        to.process_new_packets()?;
    }
    Ok(true)
}

/// Runs the handshake to its end, or to the first error of either side.
fn handshake(
    client: &mut Connection,
    server: &mut Connection,
    tamper_server_flight: &mut dyn FnMut(&mut Vec<u8>),
) -> Result<(), Error> {
    for _ in 0..30 {
        let a = flow(client, server, &mut |_| {})?;
        let b = flow(server, client, tamper_server_flight);
        let b = match b {
            Ok(b) => b,
            Err(e) => {
                // Let the server see the alert, as a real connection would; the client's error
                // is the verdict.
                let _ = flow(client, server, &mut |_| {});
                return Err(e);
            }
        };
        if !client.is_handshaking() && !server.is_handshaking() {
            // Deliver the last flight (the server's tickets, the client's Finished).
            flow(client, server, &mut |_| {})?;
            flow(server, client, &mut |_| {})?;
            return Ok(());
        }
        if !a && !b {
            break;
        }
    }
    Err(Error::General("handshake did not finish".to_owned()))
}

fn no_tamper(_: &mut Vec<u8>) {}

/// Sends `data` from `from` to `to` and reads all of it back.
fn send_all(from: &mut Connection, to: &mut Connection, data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut received = Vec::new();
    let mut offset = 0;
    while received.len() < data.len() {
        if offset < data.len() {
            let end = (offset + 15_000).min(data.len());
            offset += from.writer().write(&data[offset..end]).unwrap();
        }
        flow(from, to, &mut no_tamper)?;
        let mut buf = vec![0u8; 20_000];
        loop {
            match to.reader().read(&mut buf) {
                Ok(0) => break,
                Ok(n) => received.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => panic!("read: {e}"),
            }
        }
    }
    Ok(received)
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

// Interop matrix -------------------------------------------------------------------------------

fn suites() -> Vec<SupportedCipherSuite> {
    provider::ALL_CIPHER_SUITES.to_vec()
}

fn reference_suite(suite: SupportedCipherSuite) -> SupportedCipherSuite {
    *reference()
        .cipher_suites
        .iter()
        .find(|s| s.suite() == suite.suite())
        .unwrap()
}

/// The certificate kinds a suite can run with.
fn leaf_kinds(suite: SupportedCipherSuite) -> Vec<Kind> {
    let name = format!("{:?}", suite.suite());
    match suite {
        SupportedCipherSuite::Tls13(_) => {
            vec![Kind::P256, Kind::P384, Kind::Ed25519, Kind::Rsa256]
        }
        SupportedCipherSuite::Tls12(_) if name.contains("ECDSA") => {
            vec![Kind::P256, Kind::P384, Kind::Ed25519]
        }
        SupportedCipherSuite::Tls12(_) => vec![Kind::Rsa256],
    }
}

#[test]
fn every_suite_works_in_every_direction_with_every_certificate_kind() {
    let setups = [
        ("own client, ring server", Side::Own, Side::Ring),
        ("ring client, own server", Side::Ring, Side::Own),
        ("own client, own server", Side::Own, Side::Own),
        ("ring client, ring server (control)", Side::Ring, Side::Ring),
    ];
    let mut handshakes = 0;
    for suite in suites() {
        let ring_suite = reference_suite(suite);
        for leaf in leaf_kinds(suite) {
            let pki = issue(Kind::P256, leaf);
            for (label, client_side, server_side) in setups {
                let pick = |side: Side| if side == Side::Own { suite } else { ring_suite };
                let mut c = client(client_side, pick(client_side), &pki);
                let mut s = server(server_side, pick(server_side), &pki);
                let what = format!("{:?} / {leaf:?} / {label}", suite.suite());
                handshake(&mut c, &mut s, &mut no_tamper)
                    .unwrap_or_else(|e| panic!("{what}: {e:?}"));
                for end in [&c, &s] {
                    assert_eq!(
                        end.negotiated_cipher_suite().unwrap().suite(),
                        suite.suite(),
                        "{what}"
                    );
                    assert_eq!(
                        end.protocol_version(),
                        Some(version_of(suite).version),
                        "{what}"
                    );
                }
                // Application data both ways: empty-ish, a record boundary, several records.
                for (len, seed) in [(1usize, 1u8), (16_384, 2), (100_000, 3)] {
                    let up = pattern(len, seed);
                    assert_eq!(
                        send_all(&mut c, &mut s, &up).unwrap(),
                        up,
                        "{what} up {len}"
                    );
                    let down = pattern(len + 7, seed + 10);
                    assert_eq!(
                        send_all(&mut s, &mut c, &down).unwrap(),
                        down,
                        "{what} down {len}"
                    );
                }
                handshakes += 1;
            }
        }
    }
    // 3 TLS 1.3 suites x 4 certificates + 3 ECDSA suites x 3 + 3 RSA suites x 1, times 4 setups.
    assert_eq!(handshakes, (3 * 4 + 3 * 3 + 3) * 4);
}

#[test]
fn the_certificate_authority_may_sign_with_every_chain_algorithm() {
    // The leaf's signature is checked by the own client under each algorithm: ECDSA P-256 and
    // P-384, Ed25519 and RSA PKCS#1 with SHA-256, SHA-384 and SHA-512.
    let suite = suites()[0];
    for ca in [
        Kind::P256,
        Kind::P384,
        Kind::Ed25519,
        Kind::Rsa256,
        Kind::Rsa384,
        Kind::Rsa512,
    ] {
        let pki = issue(ca, Kind::P256);
        for (client_side, server_side) in [(Side::Own, Side::Ring), (Side::Ring, Side::Ring)] {
            let mut c = client(
                client_side,
                if client_side == Side::Own {
                    suite
                } else {
                    reference_suite(suite)
                },
                &pki,
            );
            let mut s = server(server_side, reference_suite(suite), &pki);
            handshake(&mut c, &mut s, &mut no_tamper)
                .unwrap_or_else(|e| panic!("CA {ca:?} {client_side:?}: {e:?}"));
        }
    }
}

// Tampered records -----------------------------------------------------------------------------

#[test]
fn a_tampered_replayed_or_truncated_record_is_refused_by_every_receiver() {
    let setups = [
        (Side::Own, Side::Ring),
        (Side::Ring, Side::Own),
        (Side::Own, Side::Own),
    ];
    for suite in suites() {
        let ring_suite = reference_suite(suite);
        let leaf = leaf_kinds(suite)[0];
        let pki = issue(Kind::P256, leaf);
        for (client_side, server_side) in setups {
            let pick = |side: Side| if side == Side::Own { suite } else { ring_suite };
            let what = format!("{:?} {client_side:?}/{server_side:?}", suite.suite());
            // Each case needs a fresh connection: a failed decryption ends it.
            let fresh = || {
                let mut c = client(client_side, pick(client_side), &pki);
                let mut s = server(server_side, pick(server_side), &pki);
                handshake(&mut c, &mut s, &mut no_tamper).unwrap();
                (c, s)
            };
            // Capture a record the client sends and a record the server sends.
            for upstream in [true, false] {
                let capture = |c: &mut Connection, s: &mut Connection| -> Vec<u8> {
                    let (sender, _) = if upstream { (c, s) } else { (s, c) };
                    sender.writer().write_all(&pattern(300, 9)).unwrap();
                    let mut bytes = Vec::new();
                    while sender.wants_write() {
                        sender.write_tls(&mut bytes).unwrap();
                    }
                    bytes
                };
                let deliver =
                    |c: &mut Connection, s: &mut Connection, bytes: &[u8]| -> Result<(), Error> {
                        let receiver = if upstream { s } else { c };
                        let mut rest = bytes;
                        while !rest.is_empty() {
                            let n = receiver.read_tls(&mut rest).unwrap();
                            if n == 0 {
                                break;
                            }
                            receiver.process_new_packets()?;
                        }
                        Ok(())
                    };
                let (mut c, mut s) = fresh();
                let record = capture(&mut c, &mut s);
                assert!(record.len() > 5 + 300);
                // The untouched record is accepted.
                deliver(&mut c, &mut s, &record).unwrap_or_else(|e| panic!("{what}: {e:?}"));

                // A flipped bit anywhere in the body (explicit nonce, ciphertext, tag) is refused.
                let len = record.len();
                for position in [5, 6, 12, 13, 5 + 150, len - 17, len - 16, len - 8, len - 1] {
                    let (mut c, mut s) = fresh();
                    let mut bad = capture(&mut c, &mut s);
                    bad[position] ^= 0x10;
                    assert!(
                        deliver(&mut c, &mut s, &bad).is_err(),
                        "{what} upstream {upstream}: flip at {position} of {len} accepted"
                    );
                }
                // A replay: the same record twice; the second has a stale sequence number.
                let (mut c, mut s) = fresh();
                let first = capture(&mut c, &mut s);
                deliver(&mut c, &mut s, &first).unwrap();
                assert!(
                    deliver(&mut c, &mut s, &first).is_err(),
                    "{what} upstream {upstream}: replay accepted"
                );
                // A record whose length field was shortened by one byte: the tag moves, so it
                // fails (or waits for more data and never completes; it must not decrypt).
                let (mut c, mut s) = fresh();
                let mut short = capture(&mut c, &mut s);
                let body = short.len() - 5;
                short[3] = ((body - 1) >> 8) as u8;
                short[4] = (body - 1) as u8;
                short.truncate(short.len() - 1);
                assert!(
                    deliver(&mut c, &mut s, &short).is_err(),
                    "{what} upstream {upstream}: shortened record accepted"
                );
                // Two records swapped (reordered): both are refused or the first one is.
                let (mut c, mut s) = fresh();
                let one = capture(&mut c, &mut s);
                let two = capture(&mut c, &mut s);
                assert!(
                    deliver(&mut c, &mut s, &two).is_err() || {
                        // A second record alone has sequence number 1 on the sender but 0 at
                        // the receiver.
                        false
                    },
                    "{what} upstream {upstream}: out-of-order record accepted"
                );
                drop(one);
            }
        }
    }
}

// Certificates: bad signatures and wrong algorithms --------------------------------------------

fn verifier(provider: CryptoProvider, pki: &Pki) -> Arc<WebPkiServerVerifier> {
    WebPkiServerVerifier::builder_with_provider(roots(pki), Arc::new(provider))
        .build()
        .unwrap()
}

fn verdict(v: &WebPkiServerVerifier, leaf: &[u8]) -> Result<(), String> {
    use rustls::client::danger::ServerCertVerifier;
    let name = ServerName::try_from("localhost").unwrap();
    v.verify_server_cert(
        &CertificateDer::from(leaf.to_vec()),
        &[],
        &name,
        &[],
        UnixTime::now(),
    )
    .map(|_| ())
    .map_err(|e| format!("{e:?}"))
}

#[test]
fn a_certificate_with_a_bad_signature_fails_like_it_does_with_the_reference() {
    for ca in [
        Kind::P256,
        Kind::P384,
        Kind::Ed25519,
        Kind::Rsa256,
        Kind::Rsa384,
        Kind::Rsa512,
    ] {
        let pki = issue(ca, Kind::P256);
        let (ours, theirs) = (verifier(own(), &pki), verifier(reference(), &pki));
        verdict(&ours, pki.leaf.as_ref()).unwrap();
        verdict(&theirs, pki.leaf.as_ref()).unwrap();
        let der = pki.leaf.as_ref().to_vec();
        // The signature value is the end of the certificate: every one of its last bytes flips
        // to a failure, with the error of the reference.
        let sig_tail = if ca.is_rsa() { 256 } else { 40 };
        let mut failures = 0;
        for position in der.len() - sig_tail..der.len() {
            let mut bad = der.clone();
            bad[position] ^= 0x04;
            let (a, b) = (verdict(&ours, &bad), verdict(&theirs, &bad));
            assert_eq!(a, b, "CA {ca:?} flip at {position}");
            assert!(a.is_err(), "CA {ca:?} flip at {position} accepted");
            failures += 1;
        }
        assert_eq!(failures, sig_tail);
        // The error says the signature is bad, not that something else is wrong.
        let mut bad = der.clone();
        *bad.last_mut().unwrap() ^= 0x01;
        let err = verdict(&ours, &bad).unwrap_err();
        assert!(
            err.contains("BadSignature") || err.contains("BadEncoding"),
            "CA {ca:?}: {err}"
        );
        // Flips anywhere in the certificate: the same verdict as the reference, never a success
        // that the reference refuses.
        for position in (0..der.len() - sig_tail).step_by(3) {
            let mut bad = der.clone();
            bad[position] ^= 0x20;
            assert_eq!(
                verdict(&ours, &bad),
                verdict(&theirs, &bad),
                "CA {ca:?} flip at {position}"
            );
        }
    }
}

#[test]
fn a_provider_without_the_algorithm_of_the_certificate_refuses_it() {
    use rustls::crypto::WebPkiSupportedAlgorithms;
    // A provider that verifies RSA only, against a chain signed with ECDSA, and the reverse.
    let leak = |all: Vec<&'static dyn rustls::pki_types::SignatureVerificationAlgorithm>| {
        WebPkiSupportedAlgorithms {
            all: Box::leak(all.into_boxed_slice()),
            mapping: &[],
        }
    };
    let ecdsa_only = leak(vec![
        provider::ECDSA_P256_SHA256,
        provider::ECDSA_P384_SHA384,
    ]);
    let rsa_only = leak(vec![provider::RSA_PKCS1_SHA256, provider::RSA_PSS_SHA256]);
    for (ca, allowed, refused) in [
        (Kind::P256, ecdsa_only, rsa_only),
        (Kind::Rsa256, rsa_only, ecdsa_only),
    ] {
        let pki = issue(ca, Kind::P256);
        let with = |algs: WebPkiSupportedAlgorithms| {
            verifier(
                CryptoProvider {
                    signature_verification_algorithms: algs,
                    ..own()
                },
                &pki,
            )
        };
        // The chain verifies when the algorithm is there ...
        verdict(&with(allowed), pki.leaf.as_ref()).unwrap();
        // ... and is refused when it is not, with an error about the algorithm.
        let err = verdict(&with(refused), pki.leaf.as_ref()).unwrap_err();
        assert!(
            err.contains("UnsupportedSignatureAlgorithm"),
            "{ca:?}: {err}"
        );
    }
    // A handshake whose peer (a P-256 chain, an RSA certificate verifier) cannot be verified.
    let pki = issue(Kind::P256, Kind::P256);
    let suite = suites()[0];
    let restricted = Arc::new(CryptoProvider {
        cipher_suites: vec![suite],
        signature_verification_algorithms: rsa_only,
        ..own()
    });
    let config = ClientConfig::builder_with_provider(restricted)
        .with_protocol_versions(&[version_of(suite)])
        .unwrap()
        .with_root_certificates(roots(&pki).as_ref().clone())
        .with_no_client_auth();
    let mut c = Connection::Client(
        ClientConnection::new(Arc::new(config), ServerName::try_from("localhost").unwrap())
            .unwrap(),
    );
    let mut s = server(Side::Ring, reference_suite(suite), &pki);
    assert!(handshake(&mut c, &mut s, &mut no_tamper).is_err());
}

#[test]
fn a_handshake_signature_in_the_wrong_algorithm_or_tampered_is_refused() {
    use rustls::SignatureScheme;
    use rustls::client::danger::ServerCertVerifier;
    // The server's key signs in the scheme it has; the verifier is given a signature made under
    // one scheme and told another, or with a bit flipped, in TLS 1.2 and TLS 1.3 form.
    let pki = issue(Kind::P256, Kind::P256);
    let v = verifier(own(), &pki);
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pki.leaf_key.clone()));
    let signer = reference().key_provider.load_private_key(key).unwrap();
    let message = b"handshake transcript stand-in";
    let signing = signer
        .choose_scheme(&[SignatureScheme::ECDSA_NISTP256_SHA256])
        .unwrap();
    let signature = signing.sign(message).unwrap();
    let ok = signed(SignatureScheme::ECDSA_NISTP256_SHA256, signature.clone());
    v.verify_tls12_signature(message, &pki.leaf, &ok).unwrap();
    v.verify_tls13_signature(message, &pki.leaf, &ok).unwrap();
    // Flipped bits.
    for position in 0..signature.len() {
        let mut bad = signature.clone();
        bad[position] ^= 0x08;
        let bad = signed(SignatureScheme::ECDSA_NISTP256_SHA256, bad);
        assert!(
            v.verify_tls12_signature(message, &pki.leaf, &bad).is_err(),
            "{position}"
        );
        assert!(
            v.verify_tls13_signature(message, &pki.leaf, &bad).is_err(),
            "{position}"
        );
    }
    // Another message.
    assert!(
        v.verify_tls13_signature(b"another", &pki.leaf, &ok)
            .is_err()
    );
    // Another scheme than the key's: an Ed25519, RSA or P-384 scheme on a P-256 key.
    for scheme in [
        SignatureScheme::ED25519,
        SignatureScheme::RSA_PSS_SHA256,
        SignatureScheme::RSA_PKCS1_SHA256,
        SignatureScheme::ECDSA_NISTP384_SHA384,
        SignatureScheme::ECDSA_NISTP521_SHA512,
        SignatureScheme::RSA_PKCS1_SHA1,
    ] {
        let wrong = signed(scheme, signature.clone());
        assert!(
            v.verify_tls12_signature(message, &pki.leaf, &wrong)
                .is_err(),
            "{scheme:?}"
        );
        assert!(
            v.verify_tls13_signature(message, &pki.leaf, &wrong)
                .is_err(),
            "{scheme:?}"
        );
    }
}

/// A `DigitallySignedStruct` (the constructor is private; the wire form is public).
fn signed(scheme: rustls::SignatureScheme, signature: Vec<u8>) -> rustls::DigitallySignedStruct {
    use rustls::internal::msgs::codec::{Codec, Reader};
    let mut wire = u16::from(scheme).to_be_bytes().to_vec();
    wire.extend((signature.len() as u16).to_be_bytes());
    wire.extend(signature);
    rustls::DigitallySignedStruct::read(&mut Reader::init(&wire)).unwrap()
}

// Mutated first flights --------------------------------------------------------------------------

#[test]
fn a_mutated_server_flight_gets_the_verdict_of_the_reference_for_every_suite() {
    let mut checked = 0;
    let mut refused = 0;
    for suite in suites() {
        let ring_suite = reference_suite(suite);
        // A key whose signatures have a fixed length (Ed25519, RSA), so that the flight has the
        // same layout in every run and a position means the same field in both clients' runs.
        let leaf = if leaf_kinds(suite).contains(&Kind::Ed25519) {
            Kind::Ed25519
        } else {
            Kind::Rsa256
        };
        let pki = issue(Kind::P256, leaf);
        // Find the length of the flight once.
        let flight_len = {
            let mut c = client(Side::Ring, ring_suite, &pki);
            let mut s = server(Side::Ring, ring_suite, &pki);
            let mut len = 0;
            handshake(&mut c, &mut s, &mut |b: &mut Vec<u8>| len += b.len()).unwrap();
            len
        };
        let stride = if matches!(suite, SupportedCipherSuite::Tls12(_)) {
            1
        } else {
            4
        };
        for position in (0..flight_len).step_by(stride) {
            // Returns (the handshake finished, the flip was applied). The flight's length varies
            // by a few bytes between runs (DER signature lengths, tickets), so a position past
            // the end of a shorter flight applies no flip and is not compared.
            let run = |side: Side| {
                let pick = if side == Side::Own { suite } else { ring_suite };
                let mut c = client(side, pick, &pki);
                let mut s = server(Side::Ring, ring_suite, &pki);
                let mut seen = 0usize;
                let mut applied = false;
                let ok = handshake(&mut c, &mut s, &mut |b: &mut Vec<u8>| {
                    // Flip one bit at `position` of the concatenated server bytes.
                    if position >= seen && position < seen + b.len() {
                        b[position - seen] ^= 0x01;
                        applied = true;
                    }
                    seen += b.len();
                })
                .is_ok();
                (ok, applied)
            };
            let ((a, applied_a), (b, applied_b)) = (run(Side::Own), run(Side::Ring));
            if !(applied_a && applied_b) {
                continue;
            }
            assert_eq!(
                a,
                b,
                "{:?} flip at {position}: own {a}, ring {b}",
                suite.suite()
            );
            if !a {
                refused += 1;
            }
            checked += 1;
        }
    }
    assert!(checked > 1500, "{checked}");
    assert!(
        refused * 10 > checked * 8,
        "most flips are refused: {refused} of {checked}"
    );
}

// Network smoke (run by hand) --------------------------------------------------------------------

/// Handshakes with real servers on the public roots compiled into `webpki-roots`, with the own
/// provider. Needs the network, so it is ignored; run it with
/// `cargo test -p una-http --test provider_tls -- --ignored --nocapture`.
#[test]
#[ignore = "needs the network"]
fn the_own_provider_reaches_public_servers() {
    use std::net::TcpStream;
    let roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = Arc::new(
        ClientConfig::builder_with_provider(Arc::new(own()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    let mut ok = 0;
    for host in [
        "example.com",
        "www.cloudflare.com",
        "github.com",
        "www.wikipedia.org",
        "www.google.com",
        "letsencrypt.org",
    ] {
        let name = ServerName::try_from(host.to_owned()).unwrap();
        let mut conn = ClientConnection::new(Arc::clone(&config), name).unwrap();
        let mut tcp = match TcpStream::connect((host, 443)) {
            Ok(t) => t,
            Err(e) => {
                println!("{host}: connect failed: {e}");
                continue;
            }
        };
        tcp.set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        let mut stream = rustls::Stream::new(&mut conn, &mut tcp);
        let request = format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).unwrap();
        let mut head = [0u8; 12];
        stream.read_exact(&mut head).unwrap();
        println!(
            "{host}: {:?} {:?} {}",
            conn.protocol_version(),
            conn.negotiated_cipher_suite().map(|s| s.suite()),
            String::from_utf8_lossy(&head)
        );
        assert!(head.starts_with(b"HTTP/1."));
        ok += 1;
    }
    assert!(ok > 0, "no server was reachable");
}
