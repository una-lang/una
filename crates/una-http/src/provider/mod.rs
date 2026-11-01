//! The own TLS provider (ADR 0133 rule 17, issue #1100): `rustls`'s `CryptoProvider` built on
//! `una-crypto` alone. Pure safe Rust, no `ring`, no `cc`, no foreign code.
//!
//! It is a client provider. What it offers, and why:
//!
//! - **Cipher suites.** TLS 1.3 `TLS_CHACHA20_POLY1305_SHA256`, `TLS_AES_256_GCM_SHA384` and
//!   `TLS_AES_128_GCM_SHA256`; TLS 1.2 (the `tls12` feature of `rustls` is on) the ECDHE-ECDSA
//!   and ECDHE-RSA suites with the same three AEADs. ChaCha20-Poly1305 comes first in each
//!   version because the portable AES-GCM is an order of magnitude slower (ADR 0133 rule 14);
//!   a client's order is the order of its `ClientHello`.
//! - **Key exchange.** X25519 only. ECDH on P-256 and P-384 is not implemented (ADR 0133 open
//!   question 2), so a server that supports neither X25519 nor anything else we offer fails the
//!   handshake with an error, never with a weaker group.
//! - **Signatures** (certificate chains and `CertificateVerify`): ECDSA P-256 and P-384 with
//!   SHA-256 and SHA-384, Ed25519, RSA PKCS#1 v1.5 and PSS with SHA-256, SHA-384 and SHA-512
//!   (moduli of 2048 to 8192 bits), the list `rustls-webpki` documents for its default provider,
//!   each with `una-crypto`'s stricter key and signature policy.
//! - **Randomness.** The operating system's, through `getrandom` (the source `una-hash` uses).
//! - **Keys.** None: [`crypto_provider`]'s `KeyProvider` refuses every private key, because the
//!   client signs nothing (`with_no_client_auth`).
//!
//! QUIC is not offered (`Tls13CipherSuite::quic` is `None`), `una-http` speaks HTTP/1.1.

use rustls::Error;
use rustls::SupportedCipherSuite;
use rustls::crypto::{
    CryptoProvider, GetRandomFailed, KeyProvider, SecureRandom, SupportedKxGroup,
};
use rustls::pki_types::PrivateKeyDer;
use rustls::sign::SigningKey;
use std::sync::Arc;

mod aead;
mod hash;
mod kx;
mod suites;
mod verify;

pub use verify::{
    ECDSA_P256_SHA256, ECDSA_P256_SHA384, ECDSA_P384_SHA256, ECDSA_P384_SHA384, ED25519,
    RSA_PKCS1_SHA256, RSA_PKCS1_SHA256_ABSENT_PARAMS, RSA_PKCS1_SHA384,
    RSA_PKCS1_SHA384_ABSENT_PARAMS, RSA_PKCS1_SHA512, RSA_PKCS1_SHA512_ABSENT_PARAMS,
    RSA_PSS_SHA256, RSA_PSS_SHA384, RSA_PSS_SHA512,
};

/// The cipher suites of the provider, in the order of preference of a client.
pub static ALL_CIPHER_SUITES: &[SupportedCipherSuite] = &[
    // TLS 1.3
    suites::TLS13_CHACHA20_POLY1305_SHA256,
    suites::TLS13_AES_256_GCM_SHA384,
    suites::TLS13_AES_128_GCM_SHA256,
    // TLS 1.2
    suites::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
    suites::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
    suites::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
    suites::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
    suites::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
    suites::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
];

/// The key exchange groups of the provider: X25519.
pub static ALL_KX_GROUPS: &[&dyn SupportedKxGroup] = &[kx::X25519];

/// The provider: `ClientConfig::builder_with_provider(Arc::new(crypto_provider()))`.
///
/// Every part is `una-crypto` (or the operating system's random source); nothing falls back to
/// a process-wide default provider.
#[must_use]
pub fn crypto_provider() -> CryptoProvider {
    CryptoProvider {
        cipher_suites: ALL_CIPHER_SUITES.to_vec(),
        kx_groups: ALL_KX_GROUPS.to_vec(),
        signature_verification_algorithms: verify::SUPPORTED_SIG_ALGS,
        secure_random: &OsRandom,
        key_provider: &NoKeys,
    }
}

/// The same provider restricted to the given cipher suites (tests pin one suite at a time, and
/// a deployment may narrow the list). The other parts are those of [`crypto_provider`].
#[must_use]
pub fn crypto_provider_with_suites(suites: &[SupportedCipherSuite]) -> CryptoProvider {
    CryptoProvider {
        cipher_suites: suites.to_vec(),
        ..crypto_provider()
    }
}

/// Fills buffers from the operating system's cryptographic random source.
#[derive(Debug)]
struct OsRandom;

impl SecureRandom for OsRandom {
    fn fill(&self, buf: &mut [u8]) -> Result<(), GetRandomFailed> {
        getrandom::fill(buf).map_err(|_| GetRandomFailed)
    }
}

/// A client provider holds no private key.
#[derive(Debug)]
struct NoKeys;

impl KeyProvider for NoKeys {
    fn load_private_key(
        &self,
        _key_der: PrivateKeyDer<'static>,
    ) -> Result<Arc<dyn SigningKey>, Error> {
        Err(Error::General(
            "the own TLS provider is a client provider and holds no private key".to_owned(),
        ))
    }
}
