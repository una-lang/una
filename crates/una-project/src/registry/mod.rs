//! Publishing, yanks, advisories, `una ita audit` and `una ita watch`, and bundles (ADR 0122
//! rules 33, 36.2 and 41 to 43; `spec/30 §15`), on the verified registry client
//! ([`crate::client`]).
//!
//! Nothing here opens a connection or starts a program. Documents come and go through a
//! [`crate::transport::Transport`]: a submission is written with its `put`, and a registry is
//! read verified by [`crate::client::Registry`], whatever place serves it, so every decision here
//! rests on verified bytes.

pub mod audit;
pub mod bundle;
pub mod keyfile;
pub mod submit;
pub mod watch;

use std::fmt::Write as _;

/// A read of `registry` that failed, coded right after it by the kind the client gave it
/// ([`crate::client::Registry::take_failure`]): U7013 when no place served what it needed,
/// U7010 otherwise. Never called after anything but the read whose failure it codes.
pub(crate) fn read_failure(
    registry: &mut crate::client::Registry<'_>,
    why: crate::transport::Refusal,
) -> (una_diag::Code, crate::transport::Refusal) {
    let code = match registry.take_failure() {
        Some(crate::client::Failure::Unserved) => una_diag::Code::U7013,
        Some(crate::client::Failure::Refused) | None => una_diag::Code::U7010,
    };
    (code, why)
}

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
