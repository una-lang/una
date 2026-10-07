//! The client over TLS against a local `rustls` server (`spec/24 §3` rule 4, ADR 0074 H8): a
//! test certificate authority, made at test time, is trusted through the debug-only hook, never
//! in release builds. The repository holds no private key (N-31).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
#[cfg(debug_assertions)]
use std::time::Instant;

use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
#[cfg(debug_assertions)]
use una_http::client::trust_test_roots;
use una_http::client::{Error, Request, Settings, send};
use una_http::grant::{Grant, Method};
use una_http::parse::Limits;

/// The certificates of this test process, made once: a CA, whose certificate is written to
/// `ca.pem` for the hook, and three server certificates with their keys. `localhost` (for
/// `localhost` and `127.0.0.1`) and `other` (for `other.test`) are signed by the CA; `self`,
/// for `localhost`, is self-signed.
struct Authority {
    // Read by the hook, which release builds do not have.
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    ca: PathBuf,
    servers: BTreeMap<&'static str, (CertificateDer<'static>, Vec<u8>)>,
}

fn authority() -> &'static Authority {
    static AUTHORITY: OnceLock<Authority> = OnceLock::new();
    AUTHORITY.get_or_init(|| {
        let ca_key = KeyPair::generate().unwrap();
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca_cert = ca_params.self_signed(&ca_key).unwrap();
        let issuer = Issuer::new(ca_params, ca_key);
        let mut servers = BTreeMap::new();
        for (name, sans) in [
            ("localhost", vec!["localhost", "127.0.0.1"]),
            ("other", vec!["other.test"]),
            ("self", vec!["localhost"]),
        ] {
            let key = KeyPair::generate().unwrap();
            let params =
                CertificateParams::new(sans.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
                    .unwrap();
            let cert = if name == "self" {
                params.self_signed(&key).unwrap()
            } else {
                params.signed_by(&key, &issuer).unwrap()
            };
            servers.insert(name, (cert.der().clone(), key.serialize_der()));
        }
        let dir = std::env::temp_dir().join(format!("una-http-tls-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ca = dir.join("ca.pem");
        std::fs::write(&ca, ca_cert.pem()).unwrap();
        Authority { ca, servers }
    })
}

/// Serves TLS with the certificate `name` (`localhost`, `other` or `self`), answering every
/// request with `answer`; returns the port.
fn serve(name: &str, answer: &'static str) -> u16 {
    let (cert, key) = authority().servers[name].clone();
    let certs = vec![cert];
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key));
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let config = Arc::clone(&config);
            std::thread::spawn(move || {
                let Ok(connection) = rustls::ServerConnection::new(config) else {
                    return;
                };
                let mut tls = rustls::StreamOwned::new(connection, stream);
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    match tls.read(&mut byte) {
                        Ok(1) => request.push(byte[0]),
                        _ => return,
                    }
                }
                let _ = tls.write_all(answer.as_bytes());
                tls.conn.send_close_notify();
                let _ = tls.flush();
            });
        }
    });
    port
}

fn get(url: &str) -> Request<'_> {
    Request {
        method: Method::Get,
        url,
        headers: &[],
        body: &[],
        bearer: None,
    }
}

fn settings() -> Settings {
    Settings {
        limits: Limits::default(),
        timeout: Duration::from_secs(5),
    }
}

const OK: &str = "HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nsecure";

#[cfg(debug_assertions)]
#[test]
fn https_reaches_a_trusted_server_by_address_and_by_name() {
    trust_test_roots(&authority().ca);
    let port = serve("localhost", OK);
    for host in ["127.0.0.1", "localhost"] {
        let url = format!("https://{host}:{port}/");
        let grant = Grant::parse(&format!("http:https://{host}:{port}")).unwrap();
        let response = send(&get(&url), &[grant], &settings()).unwrap();
        assert_eq!(
            (response.status, response.body.as_str()),
            (200, "secure"),
            "{host}"
        );
    }
}

#[cfg(debug_assertions)]
#[test]
fn untrusted_or_misnamed_certificates_fail_with_tls() {
    trust_test_roots(&authority().ca);
    // Self-signed: not under any trusted root.
    let port = serve("self", OK);
    let url = format!("https://localhost:{port}/");
    let grant = Grant::parse(&format!("http:https://localhost:{port}")).unwrap();
    assert_eq!(send(&get(&url), &[grant], &settings()), Err(Error::Tls));
    // Signed by the trusted root, but for another name.
    let port = serve("other", OK);
    let url = format!("https://localhost:{port}/");
    let grant = Grant::parse(&format!("http:https://localhost:{port}")).unwrap();
    assert_eq!(send(&get(&url), &[grant], &settings()), Err(Error::Tls));
}

#[cfg(debug_assertions)]
#[test]
fn a_handshake_that_never_comes_meets_the_deadline() {
    trust_test_roots(&authority().ca);
    // TCP accepted, and then nothing.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming() {
            held.push(stream);
        }
    });
    let url = format!("https://127.0.0.1:{port}/");
    let grant = Grant::parse(&format!("http:https://127.0.0.1:{port}")).unwrap();
    let started = Instant::now();
    let quick = Settings {
        timeout: Duration::from_millis(600),
        ..settings()
    };
    assert_eq!(send(&get(&url), &[grant], &quick), Err(Error::TimedOut));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

/// In a release build there is no hook: the test authority is never trusted, so a server it
/// signed fails with `Tls`, as any unknown one does (N-28). `scripts/local-ci.sh` runs this file
/// in release too.
#[cfg(not(debug_assertions))]
#[test]
fn a_release_build_never_trusts_the_test_authority() {
    let port = serve("localhost", OK);
    let url = format!("https://127.0.0.1:{port}/");
    let grant = Grant::parse(&format!("http:https://127.0.0.1:{port}")).unwrap();
    assert_eq!(send(&get(&url), &[grant], &settings()), Err(Error::Tls));
}
