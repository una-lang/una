//! The registry client's side of publishing, yanks, advisories, `una ita audit` and
//! `una ita watch`, mirrors, private sources and bundles (ADR 0122 rules 33, 36.2 and 40 to 43;
//! `spec/30 §14`).
//!
//! Nothing here opens a connection or starts a program. Documents come and go through a
//! [`transport::Transport`]; this release implements only the directory transport, and refuses
//! every URL whose scheme is neither `https` nor `file` when it is read
//! ([`transport::location`]). What the verified client knows of a registry (a name's state at a
//! fresh checkpoint, its verified chain, an owner's key events) comes through [`view`], which the
//! verification of roots, checkpoints, the name map and proofs implements; so every decision here
//! rests on verified bytes, wherever they were fetched from.

pub mod audit;
pub mod bundle;
pub mod entries;
pub mod keyfile;
pub mod mirrors;
pub mod submit;
pub mod transport;
pub mod view;
pub mod watch;

use std::fmt::Write as _;

/// Lowercase hexadecimal.
pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// A decimal without a leading zero, at most 20 digits.
pub(crate) fn decimal(text: &str) -> Option<u64> {
    let valid = !text.is_empty()
        && text.len() <= 20
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    valid.then(|| text.parse().ok()).flatten()
}

/// The hex of a digest `sha256:<hex>`, when it is one.
pub(crate) fn digest_hex(digest: &str) -> Option<&str> {
    if crate::ita::is_digest(digest) {
        digest.get("sha256:".len()..)
    } else {
        None
    }
}
