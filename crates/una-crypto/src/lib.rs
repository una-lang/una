//! Una's own cryptography for TLS (ADR 0133, issue #1100, epic #1097): pure Rust, no C, no
//! assembly, no foreign function interface, `unsafe` forbidden.
//!
//! Slices 1 to 3, 5 and 6 hold the hash, MAC, key-derivation, AEAD, key-agreement and the Ed25519,
//! ECDSA and RSA verification primitives of the TLS 1.3 suites and their transcript hashes:
//!
//! | Module | Specification |
//! | --- | --- |
//! | [`sha2`] | FIPS 180-4: SHA-256, SHA-384, SHA-512 |
//! | [`hmac`] | RFC 2104 and FIPS 198-1 |
//! | [`hkdf`] | RFC 5869 |
//! | [`chacha20poly1305`] | RFC 8439 |
//! | [`aes_gcm`] | FIPS 197 and NIST SP 800-38D: AES-128/256-GCM |
//! | [`ecdsa`] | FIPS 186-5, SEC 1: P-256 and P-384 signature verification |
//! | [`ed25519`] | RFC 8032: strict signature verification |
//! | [`rsa`] | RFC 8017: PKCS#1 v1.5 and PSS signature verification, 2048 to 8192 bits |
//! | [`x25519`] | RFC 7748 |
//!
//! **Constant time.** No branch and no memory index depends on a secret value; secret
//! comparisons go through [`ct::eq`]; every secret-holding state is wiped on drop. The rules,
//! their limits and how they are tested are ADR 0133 rules 5 to 9.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod aes_gcm;
pub mod chacha20poly1305;
pub mod ct;
pub mod ecdsa;
pub mod ed25519;
pub mod hkdf;
pub mod hmac;
pub mod rsa;
pub mod sha2;
pub mod x25519;

mod aes;
mod chacha20;
mod fe25519;
mod ghash;
mod mont;
mod poly1305;
mod sha2_consts;

pub use sha2::Hash;
