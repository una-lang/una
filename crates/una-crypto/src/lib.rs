//! Una's own cryptography for TLS (ADR 0133, issue #1100, epic #1097): pure Rust, no C, no
//! assembly, no foreign function interface, `unsafe` forbidden.
//!
//! Slice 1 holds the hash, MAC, key-derivation, AEAD and key-agreement primitives of the
//! TLS 1.3 suites `TLS_CHACHA20_POLY1305_SHA256` and the SHA-2 transcript hashes:
//!
//! | Module | Specification |
//! | --- | --- |
//! | [`sha2`] | FIPS 180-4: SHA-256, SHA-384, SHA-512 |
//! | [`hmac`] | RFC 2104 and FIPS 198-1 |
//! | [`hkdf`] | RFC 5869 |
//! | [`chacha20poly1305`] | RFC 8439 |
//! | [`x25519`] | RFC 7748 |
//!
//! **Constant time.** No branch and no memory index depends on a secret value; secret
//! comparisons go through [`ct::eq`]; every secret-holding state is wiped on drop. The rules,
//! their limits and how they are tested are ADR 0133 rules 5 to 9.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod chacha20poly1305;
pub mod ct;
pub mod hkdf;
pub mod hmac;
pub mod sha2;
pub mod x25519;

mod chacha20;
mod poly1305;
mod sha2_consts;

pub use sha2::Hash;
