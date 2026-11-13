//! A publisher's signing key (ADR 0122 rule 38), which signs release statements and yanks, and
//! the signed notes it writes.
//!
//! A key file in `~/.una/keys/` is, in this release, the form written with `--no-passphrase`:
//!
//! ```text
//! ita-keyfile/1
//! name acme.publisher
//! secret <base64 of the 32-byte Ed25519 secret>
//! ```
//!
//! A key file encrypted with a passphrase, the default of rule 38, is not read yet:
//! [`SigningKey::parse`] refuses it by its first line, `ita-keyfile-encrypted/1`, saying that
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
pub const FORMAT: &str = "ita-keyfile/1";
/// The first line of a key file encrypted with a passphrase.
pub const ENCRYPTED: &str = "ita-keyfile-encrypted/1";

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
        let probe = b"ita-keyfile/1 probe";
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
        let mut decoded = lines
            .next()
            .and_then(|line| line.strip_prefix("secret "))
            .and_then(base64)
            .unwrap_or_default();
        let mut secret = [0u8; 32];
        let fits = decoded.len() == secret.len();
        if fits {
            secret.copy_from_slice(&decoded);
        }
        wipe(&mut decoded);
        if !fits {
            return Err("a key file's third line is `secret` and 32 bytes of base64".to_owned());
        }
        let key = if lines.next().is_some() {
            Err("a key file has three lines".to_owned())
        } else {
            SigningKey::from_secret(name, &secret)
        };
        wipe(&mut secret);
        key
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

/// Overwrites `bytes`, a copy of a secret, with zeros once it is no longer needed: a best effort
/// in safe code only, `fill(0)` then [`std::hint::black_box`] so the writes are not optimized
/// away. It cannot reach copies the allocator or the signing library made, which is why a key
/// file encrypted with a passphrase is the default of rule 38.
pub fn wipe(bytes: &mut [u8]) {
    bytes.fill(0);
    std::hint::black_box(&bytes);
}

/// The most bytes of a key file.
pub const MAX_KEY_FILE: u64 = 4096;

/// Reads the key file at `path` (rule 38): opened with no link followed, and checked on the
/// opened descriptor to be a regular file of at most [`MAX_KEY_FILE`] bytes, read through it at
/// most one byte more, and on Unix readable and writable by its owner only; in the form
/// [`SigningKey::parse`] reads. Every copy of the file's bytes is wiped ([`wipe`]).
pub fn read_key_file(path: &Path) -> Result<SigningKey, String> {
    read_key_file_on(path).map(|(key, _)| key)
}

/// [`read_key_file`], with the device the opened file is on.
fn read_key_file_on(path: &Path) -> Result<(SigningKey, u64), String> {
    let shown = path.display();
    let file = crate::tree::read_user_file(path, MAX_KEY_FILE)
        .map_err(|why| format!("the key file `{shown}`: {why}"))?;
    let device = file.device;
    if !file.private {
        let mut bytes = file.bytes;
        wipe(&mut bytes);
        return Err(format!(
            "the key file `{shown}` can be read by others; make it readable by its owner only (mode 0600)"
        ));
    }
    let text = String::from_utf8(file.bytes).map_err(|error| {
        let mut bytes = error.into_bytes();
        wipe(&mut bytes);
        format!("the key file `{shown}` is not UTF-8")
    })?;
    let key = SigningKey::parse(&text).map_err(|why| format!("the key file `{shown}`: {why}"));
    let mut bytes = text.into_bytes();
    wipe(&mut bytes);
    key.map(|key| (key, device))
}

/// The keys of `owner` in `dir`, `~/.una/keys/`, that sign for it (rule 38), read strictly:
///
/// - the owner's key files are the regular files named `<owner>-<id>.key`, `<id>` lowercase
///   hexadecimal, as `una ita key new` writes them; every one of them must read
///   ([`read_key_file`]): one that is a link, can be read by others, is encrypted or does not
///   parse refuses the whole choice, so no key is silently left out or taken;
/// - a key signs only when its name and public key are those of a key of `registered`, the
///   owner's keys valid at the head of its chain, or of `former`, the owner's earlier keys, so
///   its signature line carries the name and key hash a client looks for (`spec/30 §11` rule
///   3); the others are not used. A former key's signature counts only where a claim of
///   compromise makes it a key again, which the chain as read does not show (`spec/30 §14`
///   rule 4 of the owner's keys);
/// - at least `threshold` distinct registered keys must be found, and every one found signs.
///
/// The directory is checked as a best-effort bound on a local attacker who can already write
/// `~/.una`: its device and inode before it is listed and after every read, and each key file's
/// device. The check becomes relative to the directory's descriptor later.
///
/// Returns the keys in the order of their file names.
pub fn owner_keys(
    dir: &Path,
    owner: &str,
    registered: &[Key],
    former: &[Key],
    threshold: usize,
) -> Result<Vec<SigningKey>, String> {
    let prefix = format!("{owner}-");
    match std::fs::symlink_metadata(dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "`{}` holds no key of `{owner}`; a release is signed by the owner's keys",
                dir.display()
            ));
        }
        _ => {}
    }
    // A best-effort bound on a local attacker who can already write `~/.una`: the directory's
    // device and inode, read with no link followed, are recorded before it is listed, each key
    // file opened must be on that device, and after every read the directory must still be the
    // same one and still private. Any change refuses. The check becomes relative to the
    // directory's descriptor later, which closes the window between the checks and the opens.
    let before = crate::tree::private_dir_identity(dir)?;
    let entries = std::fs::read_dir(dir)
        .map_err(|error| format!("cannot list `{}`: {error}", dir.display()))?;
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
    let mut earlier: Vec<SigningKey> = Vec::new();
    for name in names {
        let (key, device) = read_key_file_on(&dir.join(&name))?;
        if cfg!(unix) && device != before.0 {
            return Err(format!(
                "the key file `{name}` is not on the device of `{}`",
                dir.display()
            ));
        }
        let is_registered = registered.contains(key.key());
        let seen = chosen
            .iter()
            .chain(&earlier)
            .any(|c| c.key().public == key.key().public);
        if is_registered && !seen {
            chosen.push(key);
        } else if former.contains(key.key()) && !seen {
            earlier.push(key);
        }
    }
    if crate::tree::private_dir_identity(dir)? != before {
        return Err(format!(
            "`{}` changed while its key files were read",
            dir.display()
        ));
    }
    if threshold == 0 || chosen.len() < threshold {
        return Err(format!(
            "`{}` holds {} of the keys registered for `{owner}`, fewer than its threshold of {threshold}",
            dir.display(),
            chosen.len()
        ));
    }
    chosen.extend(earlier);
    Ok(chosen)
}
