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
//! A key file encrypted with a passphrase, the default of rule 38, is not read yet:
//! [`SigningKey::parse`] refuses it by its first line, `ita-key-encrypted/1`, saying that
//! passphrase encryption is not implemented yet, and never reads it as clear. A key is accepted
//! only when its public key passes the strict rule ([`crate::tlog::ed25519`]) on a signature of
//! its own. [`owner_keys`] chooses, among the files of `~/.una/keys/`, the owner's keys that
//! sign. Signing is O(1) per signature, about 20 µs; choosing is O(f log f) for f files.

use std::fmt;
use std::path::Path;

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
                "the key file is encrypted with a passphrase, and passphrase encryption is not implemented yet; use a key written with `--no-passphrase` on a machine whose disk is protected otherwise"
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

/// The most bytes of a key file.
pub const MAX_KEY_FILE: u64 = 4096;

/// Reads the key file at `path`: a regular file, not a link, of at most [`MAX_KEY_FILE`] bytes,
/// and on Unix readable and writable by its owner only (rule 38), in the form
/// [`SigningKey::parse`] reads.
pub fn read_key_file(path: &Path) -> Result<SigningKey, String> {
    let shown = path.display();
    let meta = std::fs::symlink_metadata(path)
        .map_err(|error| format!("cannot read the key file `{shown}`: {error}"))?;
    if !meta.is_file() {
        return Err(format!("the key file `{shown}` is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(format!(
                "the key file `{shown}` can be read by others; make it readable by its owner only (mode 0600)"
            ));
        }
    }
    if meta.len() > MAX_KEY_FILE {
        return Err(format!("the key file `{shown}` is too long"));
    }
    let bytes = crate::tree::read_unlinked(path)?;
    let text =
        String::from_utf8(bytes).map_err(|_| format!("the key file `{shown}` is not UTF-8"))?;
    SigningKey::parse(&text).map_err(|why| format!("the key file `{shown}`: {why}"))
}

/// The keys of `owner` in `dir`, `~/.una/keys/`, that sign for it (rule 38), read strictly:
///
/// - the owner's key files are the regular files named `<owner>-<id>.key`, `<id>` lowercase
///   hexadecimal, as `una ita key new` writes them; every one of them must read
///   ([`read_key_file`]): one that is a link, can be read by others, is encrypted or does not
///   parse refuses the whole choice, so no key is silently left out or taken;
/// - a key signs only when its name and public key are those of a key of `registered`, the
///   owner's keys valid at the head of its chain, so its signature line carries the name and
///   key hash a client looks for (`spec/30 §11` rule 3); the others are not used;
/// - at least `threshold` distinct registered keys must be found, and every one found signs.
///
/// Returns the keys in the order of their file names.
pub fn owner_keys(
    dir: &Path,
    owner: &str,
    registered: &[Key],
    threshold: usize,
) -> Result<Vec<SigningKey>, String> {
    let prefix = format!("{owner}-");
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "`{}` holds no key of `{owner}`; a release is signed by the owner's keys",
                dir.display()
            ));
        }
        Err(error) => return Err(format!("cannot list `{}`: {error}", dir.display())),
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| {
            name.strip_prefix(&prefix)
                .and_then(|rest| rest.strip_suffix(".key"))
                .is_some_and(|id| {
                    !id.is_empty()
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
        })
        .collect();
    names.sort();
    let mut chosen: Vec<SigningKey> = Vec::new();
    for name in names {
        let key = read_key_file(&dir.join(&name))?;
        let is_registered = registered.contains(key.key());
        let seen = chosen.iter().any(|c| c.key().public == key.key().public);
        if is_registered && !seen {
            chosen.push(key);
        }
    }
    if threshold == 0 || chosen.len() < threshold {
        return Err(format!(
            "`{}` holds {} of the keys registered for `{owner}`, fewer than its threshold of {threshold}",
            dir.display(),
            chosen.len()
        ));
    }
    Ok(chosen)
}
