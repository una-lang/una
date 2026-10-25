//! Itaty, the registry service of Itá (ADR 0122 rules 34 to 46, `spec/30 §16`).
//!
//! The service keeps one append-only transparency log of every release statement, yank,
//! advisory and owner key event, with the name map of rule 36.1 committed in every checkpoint,
//! and serves both as static files: the log as C2SP `tlog-tiles`, the map as content-addressed
//! tiles, the archives by digest. Everything a client reads is verified by the client against
//! the root metadata; the service verifies everything it is sent before it logs it, and refuses
//! anything it cannot verify.
//!
//! It speaks the formats of the client in `una-project` (`tlog`, `content`) by reusing that
//! code: the Merkle tree, strict Ed25519, signed notes, checkpoints and cosignatures, root
//! metadata, release statements, yanks, advisories, the owner formats and the replay of an
//! owner's chain, the name map, the archive and the content digest. What only a registry does
//! is here: the log's storage and tiles, the map's tiles, the checks of rules 26, 37, 38 and
//! 43 to 45 at submission, cosignature collection, signed promises, and the HTTP endpoints.
//!
//! The parts:
//! - [`store`]: the file-backed store behind a trait, which is also what HTTP serves;
//! - [`tree`] and [`tiles`]: the log's hashes, proofs and C2SP tiles;
//! - [`namemap`]: the incremental name map and its tiles;
//! - [`entry`]: the entries of the log, read by the client's reader;
//! - [`names`]: the name rules of rules 44 and 45 at registration;
//! - [`authority`]: a package's authority and trust, computed from its archive (rule 37);
//! - [`registry`]: the state of the registry and the checks at submission;
//! - [`timing`]: the witness obligations, and the deadlines of owners' requests;
//! - [`witness`]: a witness, which replays the log and the map before it cosigns;
//! - [`http`]: the HTTP/1.1 endpoints, on the standard library alone;
//! - [`sign`]: signing notes, for the log, witnesses and tests.

pub mod authority;
pub mod entry;
pub mod http;
pub mod namemap;
pub mod names;
pub mod registry;
pub mod sign;
pub mod store;
pub mod tiles;
pub mod timing;
pub mod tree;
pub mod witness;

use std::fmt::Write as _;

/// The largest entry the log takes: a signed note of at most 64 KiB (ADR 0122 rule 23), less
/// one byte, since an entry bundle of `tlog-tiles` gives each entry a 16-bit length.
pub const MAX_ENTRY: usize = (64 << 10) - 1;

/// The largest archive the service takes: a package's 256 MiB (rule 27) with its listing,
/// whose 10,000 entries of at most 255 bytes of path, 20 of length and 64 of hash are bounded
/// by 4 MiB.
pub const MAX_ARCHIVE: usize = (256 << 20) + (4 << 20);

/// The most needs a release states (rule 23).
pub const MAX_NEEDS: usize = 1_000;

/// The most keys an owner registers at once, or holds.
pub const MAX_OWNER_KEYS: usize = 32;

/// The most releases a name holds (rule 23: 10,000 releases read per name).
pub const MAX_RELEASES_PER_NAME: usize = 10_000;

/// The time a promise gives the registry to log a submission (rule 36.2), in seconds.
pub const PROMISE_WINDOW: u64 = 3_600;

/// Lowercase hexadecimal.
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// The digest form of a hash: `sha256:` and its lowercase hexadecimal.
pub fn digest_string(hash: &[u8; 32]) -> String {
    format!("sha256:{}", hex(hash))
}

/// Reads a digest `sha256:<64 lowercase hex>` back into its hash.
pub fn parse_digest(text: &str) -> Option<[u8; 32]> {
    let hexits = text.strip_prefix("sha256:")?;
    if hexits.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (slot, pair) in out.iter_mut().zip(hexits.as_bytes().chunks(2)) {
        let mut value = 0u8;
        for c in pair {
            let digit = match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                _ => return None,
            };
            value = (value << 4) | digit;
        }
        *slot = value;
    }
    Some(out)
}

/// A decimal without a leading zero, at most 20 digits, as every format of the registry writes
/// numbers.
pub fn decimal(text: &str) -> Option<u64> {
    let valid = !text.is_empty()
        && text.len() <= 20
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    valid.then(|| text.parse().ok()).flatten()
}

/// Reads a file of a registry's tree by its path: its bytes, or `None` when it is missing.
pub type Fetch<'f> = dyn FnMut(&str) -> Result<Option<Vec<u8>>, String> + 'f;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digests_read_back() {
        let hash = [0xabu8; 32];
        assert_eq!(parse_digest(&digest_string(&hash)), Some(hash));
        assert_eq!(parse_digest("sha256:AB"), None);
        assert_eq!(parse_digest(&digest_string(&hash).to_uppercase()), None);
    }

    #[test]
    fn decimals_are_canonical() {
        assert_eq!(decimal("0"), Some(0));
        assert_eq!(decimal("10"), Some(10));
        assert_eq!(decimal("010"), None);
        assert_eq!(decimal(""), None);
        assert_eq!(decimal("-1"), None);
        assert_eq!(decimal("99999999999999999999"), None);
    }
}
