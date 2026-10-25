//! A publisher's signing key (ADR 0122 rule 38), which signs release statements and yanks, and
//! the signed notes it writes.
//!
//! A key file in `~/.una/keys/` is, in this release, the form written with `--no-passphrase`:
//!
//! ```text
//! ita-key/1
//! name acme.publisher
//! secret <base64 of the 32-byte Ed25519 secret>
//! ```
//!
//! A key file encrypted with a passphrase, the default of rule 38, needs a key derivation
//! function and a cipher that are not yet in the trusted base; [`SigningKey::parse`] refuses it
//! by its first line, `ita-key-encrypted/1`, naming why, and never reads it as clear. A key is
//! accepted only when its public key passes the strict rule ([`crate::tlog::ed25519`]) on a
//! signature of its own. Signing is O(1) per signature, about 20 µs.

use std::fmt;

use ed25519_dalek::Signer as _;

use crate::tlog::ed25519;
use crate::tlog::note::{Key, base64, encode_base64};

/// The first line of a key file without a passphrase.
pub const FORMAT: &str = "ita-key/1";
/// The first line of a key file encrypted with a passphrase.
pub const ENCRYPTED: &str = "ita-key-encrypted/1";

/// A signing key and its name. Its secret is never printed: `Debug` shows the name only.
pub struct SigningKey {
    key: Key,
    secret: ed25519_dalek::SigningKey,
}

impl fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SigningKey")
            .field("name", &self.key.name)
            .finish_non_exhaustive()
    }
}

/// Whether `name` is a key name of a signed note: not empty, at most 255 bytes, no space, no
/// control character and no `+`.
fn is_key_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.contains('+')
        && !name.chars().any(|c| c.is_whitespace() || c.is_control())
}

impl SigningKey {
    /// The key of `secret`, named `name`, when its public key passes the strict rule.
    pub fn from_secret(name: &str, secret: &[u8; 32]) -> Result<SigningKey, String> {
        if !is_key_name(name) {
            return Err(format!("`{name}` is not a key name"));
        }
        let secret = ed25519_dalek::SigningKey::from_bytes(secret);
        let key = Key {
            name: name.to_owned(),
            public: secret.verifying_key().to_bytes(),
        };
        let probe = b"ita-key/1 probe";
        let signature = secret.sign(probe).to_bytes();
        ed25519::verify(&key.public, probe, &signature)
            .map_err(|refusal| format!("the key `{name}` is refused: {refusal:?}"))?;
        Ok(SigningKey { key, secret })
    }

    /// Reads a key file, as this module states.
    pub fn parse(text: &str) -> Result<SigningKey, String> {
        if text.starts_with(ENCRYPTED) {
            return Err(
                "the key file is encrypted with a passphrase, which this release cannot read yet; use a key written with `--no-passphrase` on a machine whose disk is protected otherwise"
                    .to_owned(),
            );
        }
        let mut lines = text
            .strip_suffix('\n')
            .ok_or("a key file ends with a line feed")?
            .split('\n');
        if lines.next() != Some(FORMAT) {
            return Err(format!("a key file starts with `{FORMAT}`"));
        }
        let name = lines
            .next()
            .and_then(|line| line.strip_prefix("name "))
            .ok_or("a key file's second line is `name <key name>`")?;
        let secret: [u8; 32] = lines
            .next()
            .and_then(|line| line.strip_prefix("secret "))
            .and_then(base64)
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
            .ok_or("a key file's third line is `secret` and 32 bytes of base64")?;
        if lines.next().is_some() {
            return Err("a key file has three lines".to_owned());
        }
        SigningKey::from_secret(name, &secret)
    }

    /// The key file of this key, in the form [`SigningKey::parse`] reads.
    pub fn encode(&self) -> String {
        format!(
            "{FORMAT}\nname {}\nsecret {}\n",
            self.key.name,
            encode_base64(&self.secret.to_bytes())
        )
    }

    /// The public key and its name.
    pub fn key(&self) -> &Key {
        &self.key
    }

    /// A signature line of a note whose text is `text` (C2SP `signed-note`): `— `, the key's
    /// name, a space, and the base64 of its 4-byte key hash and the signature, then a line feed.
    pub fn sign_line(&self, text: &str) -> String {
        let mut bytes = self.key.id().to_vec();
        bytes.extend_from_slice(&self.secret.sign(text.as_bytes()).to_bytes());
        format!("\u{2014} {} {}\n", self.key.name, encode_base64(&bytes))
    }
}

/// The signed note of `text`, which ends with a line feed and has no blank line, signed by each
/// of `keys` in order. Refused for a text that a note cannot carry, and for no key.
pub fn sign_note(text: &str, keys: &[&SigningKey]) -> Result<String, String> {
    if !text.ends_with('\n') || text.contains("\n\n") || text.starts_with('\n') {
        return Err("a note's text ends with a line feed and has no blank line".to_owned());
    }
    if text
        .chars()
        .any(|c| (c.is_control() && c != '\n') || c == '\u{feff}')
    {
        return Err("a note's text has a control character".to_owned());
    }
    if keys.is_empty() {
        return Err("a note is signed by at least one key".to_owned());
    }
    let mut note = format!("{text}\n");
    for key in keys {
        note.push_str(&key.sign_line(text));
    }
    if note.len() > crate::tlog::note::MAX_NOTE {
        return Err(format!(
            "the note is longer than {} bytes",
            crate::tlog::note::MAX_NOTE
        ));
    }
    Ok(note)
}
