//! Signing notes (C2SP `signed-note` and `tlog-cosignature`, `spec/30 §11` rules 3 and 4):
//! the log's checkpoints and promises, a witness's cosignatures, and in tests the statements of
//! owners and security keys. Every signature made here verifies under the client's strict rule.

use ed25519_dalek::{Signer as _, SigningKey};
use una_project::tlog::note::{Key, encode_base64};

/// A named Ed25519 signing key.
pub struct Signer {
    key: Key,
    secret: SigningKey,
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signer")
            .field("key", &self.key.name)
            .finish()
    }
}

impl Signer {
    /// The signer of `name` from its 32-byte secret seed (RFC 8032).
    pub fn from_seed(name: &str, seed: &[u8; 32]) -> Signer {
        let secret = SigningKey::from_bytes(seed);
        let key = Key {
            name: name.to_owned(),
            public: secret.verifying_key().to_bytes(),
        };
        Signer { key, secret }
    }

    /// The public key, with its name.
    pub fn key(&self) -> &Key {
        &self.key
    }

    /// The signature line of `text` by this key: `— <name> <base64 of key hash and signature>`
    /// and a line feed.
    pub fn line(&self, text: &str) -> String {
        let mut bytes = self.key.id().to_vec();
        bytes.extend_from_slice(&self.secret.sign(text.as_bytes()).to_bytes());
        format!("\u{2014} {} {}\n", self.key.name, encode_base64(&bytes))
    }

    /// The cosignature/v1 line of the checkpoint `text` at `time`: the key hash of type `0x04`,
    /// the time as 8 bytes big-endian, and the signature of `cosignature/v1`, `time <t>`, then
    /// the text.
    pub fn cosignature(&self, text: &str, time: u64) -> String {
        let message = format!("cosignature/v1\ntime {time}\n{text}");
        let mut bytes = self.key.cosignature_id().to_vec();
        bytes.extend_from_slice(&time.to_be_bytes());
        bytes.extend_from_slice(&self.secret.sign(message.as_bytes()).to_bytes());
        format!("\u{2014} {} {}\n", self.key.name, encode_base64(&bytes))
    }
}

/// The signed note of `text` by each of `signers`, in order: the text, ending with a line feed
/// (one is added when it has none), a blank line, then the signature lines, each over the text
/// with its final line feed.
pub fn note(text: &str, signers: &[&Signer]) -> String {
    let text = if text.ends_with('\n') {
        text.to_owned()
    } else {
        format!("{text}\n")
    };
    let mut out = String::with_capacity(text.len() + 1 + signers.len() * 128);
    out.push_str(&text);
    out.push('\n');
    for signer in signers {
        out.push_str(&signer.line(&text));
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use una_project::tlog::note::parse_note;

    #[test]
    fn notes_and_cosignatures_verify_by_the_client() {
        let a = Signer::from_seed("a.example/one", &[1u8; 32]);
        let b = Signer::from_seed("b.example/two", &[2u8; 32]);
        let text = "origin\n3\nAAAA\n";
        let document = note(text, &[&a, &b]);
        let read = parse_note(&document).unwrap();
        assert!(read.signed_by(a.key()) && read.signed_by(b.key()));
        assert_eq!(read.signers(&[a.key().clone(), b.key().clone()]), 2);
        let cosigned = format!("{document}{}", b.cosignature(text, 77));
        let read = parse_note(&cosigned).unwrap();
        assert_eq!(read.cosignature(b.key()), Some(77));
        assert_eq!(read.cosignature(a.key()), None);
    }
}
