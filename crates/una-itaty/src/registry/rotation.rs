//! Rotating the registry's witnesses (ADR 0122 rule 35, `spec/30 §11` rules 8 and 9): the
//! text of the next root version, for the root keys to sign offline, and its installation.
//!
//! - **Revocation by key.** `witnesses-revoked` lists witness keys by fingerprint, the hex
//!   SHA-256 of the public key, never by name. Each version the tooling writes lists every
//!   fingerprint any version of the chain revoked, and a fingerprint once listed is never a
//!   witness again: the tooling refuses it, as every client refuses such a chain.
//! - **Compromise.** A rotation marked as caused by a compromise ([`Rotation::compromised`])
//!   lists the compromised witnesses' keys in `witnesses-revoked` by default. Rotating them out
//!   without listing them needs the operator's explicit confirmation
//!   ([`Rotation::confirmed_unlisted`]); there is no prompt in this library.
//! - **Raising the threshold.** Clients count every checkpoint by at least the last root's
//!   threshold, so a raised threshold makes older history unavailable wherever a kept
//!   checkpoint a result reads carries fewer cosignatures. Before a rotation raises it, every
//!   kept checkpoint the obligations name (each multiple of 64, and each request's start,
//!   deadline and pre-deadline checkpoints) MUST carry the new threshold of cosignatures that
//!   would count; the incoming witnesses cosign them first ([`Registry::backfill`]), in
//!   ascending order of size, and otherwise the tooling refuses and says which fall short.
//! - **Kept history.** Older checkpoints stay as they are, with their original cosignatures;
//!   the log signs its next size under the new root. Until then the newest checkpoint names the
//!   older version, which clients take as a newer checkpoint withheld (U7013), the fail-safe
//!   direction: an operator logs the next entry soon after installing a rotation.
//!
//! The log's key and the other key sets are kept as they are: this tooling rotates witnesses.
//!
//! Costs: O(L / 64 + R) kept checkpoints read to check a raised threshold, for a log of L
//! entries and R timed requests.

use std::collections::BTreeSet;

use una_project::client::CADENCE;
use una_project::tlog::note::{Key, Note, encode_base64, parse_checkpoint, parse_note};
use una_project::tlog::owner::fingerprint;
use una_project::tlog::root::{self as roots, Keys, Root, walk_all};

use super::{MAX_DOCUMENT, Refusal, Registry, checkpoint_path, invalid};
use crate::MAX_ENTRY;

/// A rotation of the registry's witnesses: what the next root version holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rotation {
    /// When the next root version expires, in seconds of witness time.
    pub expires: u64,
    /// Its witnesses.
    pub witnesses: Vec<Key>,
    /// How many of them must cosign a checkpoint.
    pub threshold: usize,
    /// The current witnesses rotated out because their keys are, or may be, compromised: a
    /// rotation caused by a compromise names them here.
    pub compromised: Vec<Key>,
    /// Whether the compromised witnesses' keys are listed in `witnesses-revoked`: the default.
    pub list_compromised: bool,
    /// The operator's explicit confirmation to rotate compromised witnesses out without
    /// listing them, which is refused without it.
    pub confirmed_unlisted: bool,
}

impl Rotation {
    /// A rotation to `witnesses` with `threshold`, expiring at `expires`, caused by no
    /// compromise.
    pub fn new(expires: u64, witnesses: Vec<Key>, threshold: usize) -> Rotation {
        Rotation {
            expires,
            witnesses,
            threshold,
            compromised: Vec::new(),
            list_compromised: true,
            confirmed_unlisted: false,
        }
    }

    /// The same rotation, caused by the compromise of the witnesses of `keys`, which are then
    /// listed in `witnesses-revoked`.
    pub fn compromise(mut self, keys: Vec<Key>) -> Rotation {
        self.compromised = keys;
        self
    }
}

/// The `key <name> <base64>` lines of `keys`.
fn key_lines(out: &mut String, keys: &[Key]) {
    for key in keys {
        out.push_str(&format!(
            "key {} {}\n",
            key.name,
            encode_base64(&key.public)
        ));
    }
}

/// A key set's threshold line and its key lines.
fn key_set(out: &mut String, label: &str, keys: &Keys) {
    out.push_str(&format!("{label} {}\n", keys.threshold));
    key_lines(out, &keys.keys);
}

impl Registry {
    /// Every fingerprint a version of the root chain revokes, in the order listed.
    fn revoked_fingerprints(&self) -> Vec<String> {
        let mut revoked: Vec<String> = Vec::new();
        for (root, _) in &self.chain {
            for fingerprint in &root.witnesses_revoked {
                if !revoked.contains(fingerprint) {
                    revoked.push(fingerprint.clone());
                }
            }
        }
        revoked
    }

    /// The kept checkpoints a raised threshold `k` would leave short, by size, with the
    /// cosignatures on each that would count once `witnesses` are the last root's witnesses
    /// and `revoked` lists the fingerprints revoked: a witness of `witnesses`, or a retired
    /// witness of the version the checkpoint names that is not revoked (`spec/30 §11` rule 9).
    /// The checkpoints read are those the obligations name: each multiple of 64, and each
    /// request's start, deadline and pre-deadline checkpoints.
    pub fn short_of(&self, witnesses: &[Key], revoked: &[String], k: usize) -> Vec<(u64, usize)> {
        let size = self.tree.size();
        let mut sizes: BTreeSet<u64> = (1..=size / CADENCE).map(|n| n * CADENCE).collect();
        for request in self.requests.values() {
            sizes.insert(request.index.saturating_add(1));
            if let Some(deadline) = request.deadline {
                sizes.insert(deadline);
                sizes.insert(deadline.saturating_sub(1));
            }
        }
        sizes.retain(|kept| *kept >= 1 && *kept <= size);
        let mut short = Vec::new();
        for kept in sizes {
            let signed: Option<Note> = self
                .store
                .get(&checkpoint_path(kept), MAX_DOCUMENT)
                .ok()
                .flatten()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .and_then(|document| parse_note(&document).ok());
            let Some(signed) = signed else {
                short.push((kept, 0));
                continue;
            };
            let version = parse_checkpoint(&signed.text)
                .ok()
                .and_then(|checkpoint| roots::named(&checkpoint.extensions).ok())
                .map_or(self.root.version, |(version, _)| version);
            let named: &Root = self
                .chain
                .iter()
                .map(|(root, _)| root)
                .find(|root| root.version == version)
                .unwrap_or(&self.root);
            let mut keys: Vec<&Key> = witnesses.iter().collect();
            for key in &named.witnesses.keys {
                if !keys.iter().any(|known| known.public == key.public) {
                    keys.push(key);
                }
            }
            let count = keys
                .into_iter()
                .filter(|key| !revoked.contains(&fingerprint(&key.public)))
                .filter(|key| signed.cosignature(key).is_some())
                .count();
            if count < k {
                short.push((kept, count));
            }
        }
        short
    }

    /// The refusal of a rotation that raises the threshold to `k` while `short` fall short.
    fn raise_refusal(short: &[(u64, usize)], k: usize) -> Refusal {
        let listed: Vec<String> = short
            .iter()
            .take(8)
            .map(|(size, count)| format!("{size} ({count})"))
            .collect();
        let more = short.len().saturating_sub(listed.len());
        let tail = if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        };
        invalid(format!(
            "raising the witness threshold to {k} would leave older history unavailable to clients: the kept checkpoints of sizes {}{tail} carry fewer cosignatures that would count; have the incoming witnesses cosign them first, in ascending order of size, then rotate",
            listed.join(", ")
        ))
    }

    /// The text of the next root version for `rotation`, for the root keys of the current
    /// version and of its own to sign offline: the current root's source, root keys, log key,
    /// security keys and Itaúna's keys, with `rotation`'s witnesses and threshold, and every
    /// fingerprint the chain revoked, with the compromised witnesses' added unless the operator
    /// confirmed otherwise. Refused, one sentence, when a witness is one the chain revoked, a
    /// compromised key is not a current witness or stays one, compromised witnesses go unlisted
    /// without confirmation, the threshold is not between 1 and the witnesses, or a raised
    /// threshold would leave kept checkpoints short ([`Registry::short_of`]).
    pub fn rotation_text(&self, rotation: &Rotation) -> Result<String, Refusal> {
        let witnesses = &rotation.witnesses;
        if rotation.threshold == 0 || rotation.threshold > witnesses.len() {
            return Err(invalid(format!(
                "the witness threshold {} is not between 1 and the {} witnesses",
                rotation.threshold,
                witnesses.len()
            )));
        }
        let mut revoked = self.revoked_fingerprints();
        if let Some(witness) = witnesses
            .iter()
            .find(|witness| revoked.contains(&fingerprint(&witness.public)))
        {
            return Err(invalid(format!(
                "the witness `{}` has a key a root version revoked, which is never a witness again",
                witness.name
            )));
        }
        for key in &rotation.compromised {
            let current = self
                .root
                .witnesses
                .keys
                .iter()
                .any(|witness| witness.public == key.public);
            let kept = witnesses.iter().any(|witness| witness.public == key.public);
            if !current || kept {
                return Err(invalid(format!(
                    "the compromised key `{}` is not a current witness rotated out",
                    key.name
                )));
            }
        }
        if !rotation.compromised.is_empty() && !rotation.list_compromised {
            if !rotation.confirmed_unlisted {
                return Err(invalid(
                    "a rotation caused by a compromise lists the compromised witnesses' keys in `witnesses-revoked`; rotating them out without listing them needs the operator's explicit confirmation, since their cosignatures would still count on older checkpoints",
                ));
            }
        } else {
            for key in &rotation.compromised {
                let listed = fingerprint(&key.public);
                if !revoked.contains(&listed) {
                    revoked.push(listed);
                }
            }
        }
        if rotation.threshold > self.root.witnesses.threshold {
            let short = self.short_of(witnesses, &revoked, rotation.threshold);
            if !short.is_empty() {
                return Err(Registry::raise_refusal(&short, rotation.threshold));
            }
        }
        let root = &self.root;
        let mut text = format!(
            "ita-root/1\nsource {}\nversion {}\nexpires {}\n",
            root.source,
            root.version.saturating_add(1),
            rotation.expires
        );
        key_set(&mut text, "root", &root.root);
        text.push_str(&format!(
            "log {} {}\n",
            root.log.name,
            encode_base64(&root.log.public)
        ));
        text.push_str(&format!("witnesses {}\n", rotation.threshold));
        key_lines(&mut text, witnesses);
        if !revoked.is_empty() {
            text.push_str(&format!("witnesses-revoked {}\n", revoked.join(" ")));
        }
        key_set(&mut text, "security", &root.security);
        key_set(&mut text, "itauna", &root.itauna);
        Ok(text)
    }

    /// Installs `document`, the next root version signed offline, as `root/<n + 1>`: it MUST
    /// verify as the next version of the chain (rule 35, every revoked key never a witness
    /// again), keep the log's key, and, raising the threshold, leave no kept checkpoint short
    /// ([`Registry::short_of`]). The checkpoints signed before it are kept as they are; the
    /// next size is signed under it.
    pub fn install_root(&mut self, document: &str) -> Result<(), Refusal> {
        self.ready()?;
        let (first, _) = self
            .chain
            .first()
            .cloned()
            .ok_or_else(|| invalid("the registry has no root chain"))?;
        let mut documents: Vec<String> = Vec::new();
        for version in first.version.saturating_add(1)..=self.root.version {
            let bytes = self
                .store
                .get(&format!("root/{version}"), MAX_ENTRY)
                .map_err(Refusal::Unavailable)?
                .ok_or_else(|| Refusal::Unavailable(format!("root/{version} is missing")))?;
            documents
                .push(String::from_utf8(bytes).map_err(|_| invalid("root metadata is not UTF-8"))?);
        }
        documents.push(document.to_owned());
        let rest: Vec<&str> = documents.iter().map(String::as_str).collect();
        let walked = walk_all(&first, &rest, self.clock.now())
            .map_err(|why| invalid(format!("the next root version does not verify: {why}")))?;
        let last = walked
            .last()
            .cloned()
            .ok_or_else(|| invalid("the root chain is empty"))?;
        let next = self.root.version.saturating_add(1);
        if last.version != next {
            return Err(invalid(format!(
                "the root metadata installed is version {}, not {next}",
                last.version
            )));
        }
        if last.log != self.root.log {
            return Err(invalid(
                "the next root version keeps the log's key: this tooling rotates witnesses",
            ));
        }
        if last.witnesses.threshold > self.root.witnesses.threshold {
            let revoked: Vec<String> = walked
                .iter()
                .flat_map(|root| root.witnesses_revoked.iter().cloned())
                .collect();
            let short = self.short_of(&last.witnesses.keys, &revoked, last.witnesses.threshold);
            if !short.is_empty() {
                return Err(Registry::raise_refusal(&short, last.witnesses.threshold));
            }
        }
        self.store
            .put(&format!("root/{next}"), document.as_bytes())
            .map_err(Refusal::Unavailable)?;
        self.chain = walked
            .into_iter()
            .map(|root| {
                let digest = root.digest.clone();
                (root, digest)
            })
            .collect();
        self.root = last;
        Ok(())
    }

    /// Takes the cosignature `line` of the kept checkpoint of `size` by `witness`, a witness of
    /// a rotation not installed yet, so that a raised threshold leaves no kept checkpoint short.
    /// Every check of [`Registry::cosign`] applies: the line verifies, the witness's sizes rise
    /// and its times never decrease, so it cosigns kept checkpoints in ascending order of size
    /// and the newest last, and no checkpoint is moved across a request's target. A key any
    /// root version revoked is refused.
    pub fn backfill(&mut self, size: u64, line: &str, witness: &Key) -> Result<(), Refusal> {
        if self
            .revoked_fingerprints()
            .contains(&fingerprint(&witness.public))
        {
            return Err(invalid(format!(
                "the key of `{}` is revoked, and never a witness again",
                witness.name
            )));
        }
        if let Some((last_size, _)) = self.witnesses.get(&witness.name)
            && size <= *last_size
        {
            return Err(invalid(format!(
                "`{}` cosigned the checkpoint {last_size} already: it cosigns kept checkpoints in ascending order of size",
                witness.name
            )));
        }
        self.add_cosignature(size, line, Some(witness))
    }
}
