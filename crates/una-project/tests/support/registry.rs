//! A test registry (ADR 0122 rules 34 to 38), at the paths a registry serves: root metadata, a
//! log of signed entries served as tiles and entry bundles, the name map as content-addressed
//! tiles, checkpoints kept by size with their witnesses' cosignatures, and archives. It is built
//! in memory and written to a directory as a registry's store keeps it, the latest checkpoint at
//! `checkpoint/latest`, which the client reads as a mirror. Shared by the tests of `una-project`
//! and `una-cli`.

// Each test crate uses a part of it.
#![allow(dead_code)]
// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::Path;

use ed25519_dalek::{Signer, SigningKey};
use una_project::content::{File, archive, digest_of};
use una_project::provenance::sha256_digest;
use una_project::tlog::map::{Head, build_tiles};
use una_project::tlog::merkle::{Hash, leaf_hash, root as tree_root};
use una_project::tlog::note::{Key, encode_base64};
use una_project::tlog::owner::{chain_name as owner_chain, fingerprint};
use una_project::tlog::root::{digest, text_digest};
use una_project::tlog::tile::{WIDTH, entries_path, tile_path};

/// A signing key and its public key, from a seed.
pub fn keypair(seed: u8, name: &str) -> (SigningKey, Key) {
    let pair = SigningKey::from_bytes(&[seed; 32]);
    let public: [u8; 32] = pair.verifying_key().to_bytes();
    (
        pair,
        Key {
            name: name.to_owned(),
            public,
        },
    )
}

/// A note's Ed25519 signature line by `key`.
pub fn sign_line(pair: &SigningKey, key: &Key, text: &str) -> String {
    let mut bytes = key.id().to_vec();
    bytes.extend_from_slice(&pair.sign(text.as_bytes()).to_bytes());
    format!("\u{2014} {} {}\n", key.name, encode_base64(&bytes))
}

/// A cosignature/v1 line by `key` at `time`.
pub fn cosign_line(pair: &SigningKey, key: &Key, text: &str, time: u64) -> String {
    let message = format!("cosignature/v1\ntime {time}\n{text}");
    let mut bytes = key.cosignature_id().to_vec();
    bytes.extend_from_slice(&time.to_be_bytes());
    bytes.extend_from_slice(&pair.sign(message.as_bytes()).to_bytes());
    format!("\u{2014} {} {}\n", key.name, encode_base64(&bytes))
}

/// The two companion keys of a fixture owner's key `pair` ([`TestRegistry::register`]): derived
/// from it, named after it with `.c1` and `.c2`.
pub fn companions(pair: &(SigningKey, Key)) -> [(SigningKey, Key); 2] {
    [1u8, 2].map(|n| {
        let mut seed = pair.0.to_bytes();
        for byte in &mut seed {
            *byte ^= 0x5a ^ n;
        }
        let key = SigningKey::from_bytes(&seed);
        let public = key.verifying_key().to_bytes();
        (
            key,
            Key {
                name: format!("{}.c{n}", pair.1.name),
                public,
            },
        )
    })
}

/// A signed note of `text` by each of `signers`. A note's text ends with a line feed and is
/// followed by a blank line, then the signature lines, each over that exact text: a `text` the
/// builders form without its last line feed gets one here.
pub fn signed(text: &str, signers: &[&(SigningKey, Key)]) -> String {
    let text = if text.ends_with('\n') {
        text.to_owned()
    } else {
        format!("{text}\n")
    };
    let mut document = format!("{text}\n");
    for (pair, key) in signers {
        document.push_str(&sign_line(pair, key, &text));
    }
    document
}

/// `<name> <base64 key>`.
pub fn key_text(key: &Key) -> String {
    format!("{} {}", key.name, encode_base64(&key.public))
}

/// `sha256:` and the hex of a hash.
pub fn hash_hex(hash: &Hash) -> String {
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// The local clock of the tests, in seconds.
pub const NOW: u64 = 1_900_000_000;
/// The root's expiry.
pub const EXPIRES: u64 = 4_000_000_000;

/// The keys of a test registry.
pub struct Keys {
    pub roots: Vec<(SigningKey, Key)>,
    pub log: (SigningKey, Key),
    pub witnesses: Vec<(SigningKey, Key)>,
    pub security: (SigningKey, Key),
    pub itauna: (SigningKey, Key),
}

impl Keys {
    pub fn new(source: &str) -> Keys {
        Keys::offset(source, 0)
    }

    /// Other keys, every seed moved by `offset`: another registry under the same source name.
    pub fn offset(source: &str, offset: u8) -> Keys {
        Keys {
            roots: (10..13)
                .map(|seed| keypair(seed + offset, &format!("{source}.root.{seed}")))
                .collect(),
            log: keypair(20 + offset, &format!("{source}.example/log")),
            witnesses: (30..33)
                .map(|seed| keypair(seed + offset, &format!("witness.{seed}")))
                .collect(),
            security: keypair(40 + offset, &format!("{source}.security")),
            itauna: keypair(50 + offset, &format!("{source}.itauna")),
        }
    }
}

/// The text of root metadata of `version`, with root keys `roots` (threshold 2), the log key,
/// three witnesses (threshold 2), one security key and one key of Itaúna.
pub fn root_text(source: &str, version: u64, expires: u64, keys: &Keys, roots: &[&Key]) -> String {
    root_text_with(source, version, expires, keys, roots, 2, &[])
}

/// [`root_text`] with the witness threshold `k` and the witnesses `revoked`.
pub fn root_text_with(
    source: &str,
    version: u64,
    expires: u64,
    keys: &Keys,
    roots: &[&Key],
    k: usize,
    revoked: &[String],
) -> String {
    let mut text =
        format!("ita-root/1\nsource {source}\nversion {version}\nexpires {expires}\nroot 2\n");
    for key in roots {
        text.push_str(&format!("key {}\n", key_text(key)));
    }
    text.push_str(&format!("log {}\n", key_text(&keys.log.1)));
    text.push_str(&format!("witnesses {k}\n"));
    for (_, key) in &keys.witnesses {
        text.push_str(&format!("key {}\n", key_text(key)));
    }
    if !revoked.is_empty() {
        text.push_str(&format!("witnesses-revoked {}\n", revoked.join(" ")));
    }
    text.push_str(&format!(
        "security 1\nkey {}\nitauna 1\nkey {}",
        key_text(&keys.security.1),
        key_text(&keys.itauna.1)
    ));
    text
}

/// A test registry being built.
pub struct TestRegistry {
    pub source: String,
    pub keys: Keys,
    pub roots: Vec<String>,
    pub entries: Vec<Vec<u8>>,
    pub heads: BTreeMap<String, Head>,
    pub files: BTreeMap<String, Vec<u8>>,
    pub sizes: Vec<u64>,
    /// Whether the checkpoints signed below a checkpoint's size go without cosignatures, as a
    /// registry that breaks the cadence rule leaves them; by default they carry the same
    /// cosignatures as the checkpoint that follows them.
    pub bare: bool,
    /// Whether witnesses cosign as a registry that keeps the rules but no more asks them to:
    /// exactly each request's start checkpoint, its deadline checkpoint and the one before it,
    /// and one checkpoint per 64 sizes, each size with a time of its own, increasing between
    /// the times the checkpoints are asked at. The latest checkpoint, as served, is always
    /// cosigned; its kept copy only when one of these asks it.
    pub sparse: bool,
    /// The log indices of the recoveries, disputes and co-signer changes logged.
    pub requests: Vec<u64>,
    /// The sparse mode's state: each size's time and text, the last size and time asked at,
    /// and each request's deadline target and whether its deadline checkpoint was cosigned.
    sparse_times: BTreeMap<u64, u64>,
    sparse_texts: BTreeMap<u64, String>,
    sparse_last: Option<(u64, u64)>,
    sparse_deadlines: Vec<(u64, bool)>,
    /// In the sparse mode, how far apart in time the witnesses cosign one checkpoint: the
    /// first this much before, the last this much after.
    pub spread: u64,
    /// The witness threshold of the root versions it signs.
    pub witness_k: usize,
    /// The fixture owners' keys registered by [`TestRegistry::register`], each with its first
    /// companion, which countersigns every document the key signs ([`TestRegistry::append`]).
    pub countersign: Vec<((SigningKey, Key), (SigningKey, Key))>,
}

impl TestRegistry {
    /// A registry of `source` with its first root metadata.
    pub fn new(source: &str) -> TestRegistry {
        TestRegistry::with_keys(source, Keys::new(source))
    }

    /// A registry of `source` whose root metadata asks `k` witnesses of its three.
    pub fn with_witness_threshold(source: &str, k: usize) -> TestRegistry {
        let mut registry = TestRegistry::new(source);
        let roots: Vec<&Key> = registry.keys.roots.iter().map(|(_, key)| key).collect();
        let text = root_text_with(source, 1, EXPIRES, &registry.keys, &roots, k, &[]);
        let document = signed(&text, &[&registry.keys.roots[0], &registry.keys.roots[1]]);
        registry
            .files
            .insert("root/1".to_owned(), document.clone().into_bytes());
        registry.roots = vec![document];
        registry.witness_k = k;
        registry
    }

    /// A registry of `source` with its first root metadata under `keys`.
    pub fn with_keys(source: &str, keys: Keys) -> TestRegistry {
        let roots: Vec<&Key> = keys.roots.iter().map(|(_, key)| key).collect();
        let text = root_text(source, 1, EXPIRES, &keys, &roots);
        let document = signed(&text, &[&keys.roots[0], &keys.roots[1]]);
        let mut registry = TestRegistry {
            source: source.to_owned(),
            keys,
            roots: Vec::new(),
            entries: Vec::new(),
            heads: BTreeMap::new(),
            files: BTreeMap::new(),
            sizes: Vec::new(),
            bare: false,
            sparse: false,
            requests: Vec::new(),
            sparse_times: BTreeMap::new(),
            sparse_texts: BTreeMap::new(),
            sparse_last: None,
            sparse_deadlines: Vec::new(),
            spread: 0,
            witness_k: 2,
            countersign: Vec::new(),
        };
        registry
            .files
            .insert("root/1".to_owned(), document.clone().into_bytes());
        registry.roots.push(document);
        registry
    }

    /// The extension line of a checkpoint naming the newest root metadata, which it is
    /// signed under: `ita-root <version> sha256:<hex>`.
    pub fn root_line(&self) -> String {
        let document = self.roots.last().map_or("", String::as_str);
        format!("ita-root {} {}", self.roots.len(), digest(document))
    }

    /// Rotates the witness of seed 32 to a new key, by the next root version, signed by two root
    /// keys of the current one and of its own: the checkpoints signed from then on name it and
    /// carry the new witness's cosignature.
    pub fn rotate_witness(&mut self) {
        self.rotate_witnesses(1, false);
    }

    /// Rotates the last `count` witnesses to new keys by the next root version, which lists
    /// them as revoked when `revoke`.
    pub fn rotate_witnesses(&mut self, count: usize, revoke: bool) {
        let version = self.roots.len() as u64 + 1;
        let mut revoked = Vec::new();
        let n = self.keys.witnesses.len();
        for (at, witness) in self.keys.witnesses.iter_mut().enumerate() {
            if at + count >= n {
                revoked.push(fingerprint(&witness.1.public));
                let seed = 90 + (version as u8) * 4 + at as u8;
                *witness = keypair(seed, &format!("witness.rotated.{version}.{at}"));
            }
        }
        if !revoke {
            revoked.clear();
        }
        let roots: Vec<&Key> = self.keys.roots.iter().map(|(_, key)| key).collect();
        let text = root_text_with(
            &self.source,
            version,
            EXPIRES,
            &self.keys,
            &roots,
            self.witness_k,
            &revoked,
        );
        let document = signed(&text, &[&self.keys.roots[0], &self.keys.roots[1]]);
        self.files
            .insert(format!("root/{version}"), document.clone().into_bytes());
        self.roots.push(document);
    }

    /// The digest of the first root metadata: the pin of `sources`.
    pub fn pin(&self) -> String {
        digest(&self.roots[0])
    }

    /// The `previous` line of the next entry about `name`.
    pub fn previous(&self, name: &str) -> String {
        match self.heads.get(name) {
            Some(head) => format!("previous {} {}", head.index, hash_hex(&head.leaf)),
            None => "previous none".to_owned(),
        }
    }

    /// Appends the signed note `document` as an entry about `name`, and returns its index.
    ///
    /// **Implicit countersigning.** A document signed by a fixture owner's key
    /// ([`TestRegistry::register`]) gets its first companion's signature too, unless it has it
    /// already, so that the one key a test signs with meets the owner's threshold of 2. Every
    /// document so signed, a "thief's" included, is then signed by two of the owner's keys: a
    /// test of what one stolen key can sign alone, or of which keys a claim taints, registers
    /// with [`TestRegistry::register_keys`] and names every signer, or names the companion
    /// ([`companions`]) explicitly.
    pub fn append(&mut self, name: &str, document: &str) -> u64 {
        let mut document = document.to_owned();
        if let Some(cut) = document.find("\n\n") {
            let text = document[..=cut].to_owned();
            for ((pair, key), (other, companion)) in &self.countersign {
                let line = sign_line(other, companion, &text);
                if document.contains(&sign_line(pair, key, &text)) && !document.contains(&line) {
                    document.push_str(&line);
                }
            }
        }
        let document = document.as_str();
        let index = self.entries.len() as u64;
        let leaf = leaf_hash(document.as_bytes());
        self.entries.push(document.as_bytes().to_vec());
        let count = self.heads.get(name).map_or(0, |h| h.count) + 1;
        let head = Head { index, leaf, count };
        self.heads.insert(name.to_owned(), head);
        index
    }

    /// Registers `owner` at the key floor with the key of `pair` and its two [`companions`],
    /// threshold 2, signed by the three: the first entry of the chain `owner:<owner>`. Every
    /// document appended later that `pair` signs is countersigned by the first companion
    /// ([`TestRegistry::append`]), so a test signs with `pair` alone as an owner of one key
    /// would; a test that chooses each signer registers with [`TestRegistry::register_keys`].
    pub fn register(&mut self, owner: &str, pair: &(SigningKey, Key)) -> u64 {
        let chain = owner_chain(owner);
        let [c1, c2] = companions(pair);
        let text = format!(
            "ita-owner/1\nsource {}\nowner {owner}\n{}\nthreshold 2\nkey {}\nkey {}\nkey {}",
            self.source,
            self.previous(&chain),
            key_text(&pair.1),
            key_text(&c1.1),
            key_text(&c2.1)
        );
        let document = signed(&text, &[pair, &c1, &c2]);
        let primary = (SigningKey::from_bytes(&pair.0.to_bytes()), pair.1.clone());
        self.countersign.push((primary, c1));
        self.append(&chain, &document)
    }

    /// Registers `owner` with the keys of `pairs` and `threshold`, and the recovery cosigners
    /// `cosigners` with their threshold `k` when there are any, signed by every key registered.
    pub fn register_keys(
        &mut self,
        owner: &str,
        pairs: &[&(SigningKey, Key)],
        threshold: usize,
        cosigners: &[&(SigningKey, Key)],
        k: usize,
    ) -> u64 {
        let chain = owner_chain(owner);
        let mut text = format!(
            "ita-owner/1\nsource {}\nowner {owner}\n{}\nthreshold {threshold}",
            self.source,
            self.previous(&chain)
        );
        for (_, key) in pairs {
            text.push_str(&format!("\nkey {}", key_text(key)));
        }
        for (_, key) in cosigners {
            text.push_str(&format!("\ncosigner {}", key_text(key)));
        }
        if !cosigners.is_empty() {
            text.push_str(&format!("\ncosigners {k}"));
        }
        let document = signed(&text, pairs);
        self.append(&chain, &document)
    }

    /// A key event of `owner`: its action line, and the threshold after it, signed by
    /// `signers`.
    pub fn owner_event(
        &mut self,
        owner: &str,
        action: &str,
        threshold: usize,
        signers: &[&(SigningKey, Key)],
    ) -> u64 {
        let chain = owner_chain(owner);
        let text = format!(
            "ita-key/1\nsource {}\nowner {owner}\n{}\n{action}\nthreshold {threshold}",
            self.source,
            self.previous(&chain)
        );
        let document = signed(&text, signers);
        let index = self.append(&chain, &document);
        let request = ["recover ", "dispute ", "cosigners "]
            .iter()
            .any(|kind| action.starts_with(kind));
        if request {
            self.requests.push(index);
        }
        index
    }

    /// A recovery or a dispute (`kind`) of `owner` naming `key`, signed by the log's key and
    /// `more`.
    pub fn recover(
        &mut self,
        owner: &str,
        kind: &str,
        key: &Key,
        more: &[&(SigningKey, Key)],
    ) -> u64 {
        let log = (
            SigningKey::from_bytes(&self.keys.log.0.to_bytes()),
            self.keys.log.1.clone(),
        );
        let mut signers: Vec<&(SigningKey, Key)> = vec![&log];
        signers.extend_from_slice(more);
        let action = format!("{kind} {}", key_text(key));
        self.owner_event(owner, &action, 1, &signers)
    }

    /// Stores the archive of `files` as a blob and returns its digest and its ita.una's digest.
    pub fn blob(&mut self, files: &[(&str, &str)]) -> (String, String) {
        let mut files: Vec<File> = files
            .iter()
            .map(|(path, text)| File {
                path: (*path).to_owned(),
                bytes: text.as_bytes().to_vec(),
            })
            .collect();
        let manifest = files
            .iter()
            .find(|f| f.path == "ita.una")
            .map(|f| sha256_digest(&f.bytes))
            .unwrap();
        let content = digest_of(&mut files).unwrap();
        let bytes = archive(&mut files).unwrap();
        let hex = content.strip_prefix("sha256:").unwrap();
        self.files.insert(format!("blobs/sha256/{hex}"), bytes);
        (content, manifest)
    }

    /// Publishes a release of `name` at `version` with `authority` and `trust` lines, its
    /// archive holding `ita.una` and one module, signed by `signers`. Returns its index and its
    /// content digest.
    pub fn release(
        &mut self,
        name: &str,
        version: &str,
        authority: &str,
        trust: &str,
        signers: &[&(SigningKey, Key)],
    ) -> (u64, String) {
        let module = name.rsplit('/').next().unwrap();
        let module_text = format!("module {module}\n\npub answer() -> Int = 42\n");
        self.release_with(name, version, authority, trust, &module_text, signers)
    }

    /// A release whose archive holds `manifest_text` as its ita.una and `module_text` as its
    /// one module, and whose statement gives `needs` lines as written. Returns its index and its
    /// content digest.
    #[allow(clippy::too_many_arguments)]
    pub fn release_custom(
        &mut self,
        name: &str,
        version: &str,
        manifest_text: &str,
        needs: &[&str],
        authority: &str,
        module_text: &str,
        signers: &[&(SigningKey, Key)],
    ) -> (u64, String) {
        let module = name.rsplit('/').next().unwrap();
        let source_path = format!("src/{module}.una");
        let (content, manifest) = self.blob(&[
            ("ita.una", manifest_text),
            (source_path.as_str(), module_text),
        ]);
        let mut text = format!(
            "ita-release/1\nsource {}\nname {name}\nversion {version}\n{}\ndigest {content}\nmanifest {manifest}\nedition 0.4",
            self.source,
            self.previous(name),
        );
        for need in needs {
            text.push_str(&format!("\nneeds {need}"));
        }
        text.push_str(&format!("\nauthority {authority}\ntrust none"));
        let document = signed(&text, signers);
        (self.append(name, &document), content)
    }

    /// [`TestRegistry::release`] whose one module is `module_text`.
    pub fn release_with(
        &mut self,
        name: &str,
        version: &str,
        authority: &str,
        trust: &str,
        module_text: &str,
        signers: &[&(SigningKey, Key)],
    ) -> (u64, String) {
        let module = name.rsplit('/').next().unwrap();
        let manifest_text = format!("ita {name}\n  version {version}\n  edition 0.4\n");
        let source_path = format!("src/{module}.una");
        let (content, manifest) = self.blob(&[
            ("ita.una", manifest_text.as_str()),
            (source_path.as_str(), module_text),
        ]);
        let text = format!(
            "ita-release/1\nsource {}\nname {name}\nversion {version}\n{}\ndigest {content}\nmanifest {manifest}\nedition 0.4\nauthority {authority}\ntrust {trust}",
            self.source,
            self.previous(name),
        );
        let document = signed(&text, signers);
        (self.append(name, &document), content)
    }

    /// Publishes a checkpoint at the current size, cosigned by each witness at the time
    /// `times` gives it (`None`: no cosignature); then its tiles, entry bundles and map tiles.
    pub fn checkpoint_with(&mut self, times: &[Option<u64>]) -> u64 {
        let size = self.entries.len() as u64;
        let leaves: Vec<Hash> = self.entries.iter().map(|e| leaf_hash(e)).collect();
        let top = tree_root(&leaves);
        let heads: Vec<(String, Head)> = self
            .heads
            .iter()
            .map(|(name, head)| (name.clone(), *head))
            .collect();
        let (map_root, tiles) = build_tiles(&heads);
        if self.sparse {
            let time = times.iter().flatten().copied().max().unwrap_or_default();
            self.sparse_kept(size, time, &leaves, &map_root);
        }
        // A registry signs a checkpoint after every entry: every size below this one that has
        // none gets one the log signs, cosigned as this one is, the witnesses cosigning them
        // together; with `bare`, cosigned by none.
        for kept in 1..size {
            let path = format!("checkpoint/{kept}");
            if self.files.contains_key(&path) {
                continue;
            }
            let prefix = leaves.get(..kept as usize).unwrap_or_default();
            let text = format!(
                "{}\n{kept}\n{}\nita-map {}\n{}\n",
                self.keys.log.1.name,
                encode_base64(&tree_root(prefix)),
                encode_base64(&map_root),
                self.root_line()
            );
            let line = sign_line(&self.keys.log.0, &self.keys.log.1, &text);
            let mut document = format!("{text}\n{line}");
            if !self.bare {
                for ((pair, key), time) in self.keys.witnesses.iter().zip(times) {
                    if let Some(time) = time {
                        document.push_str(&cosign_line(pair, key, &text, *time));
                    }
                }
            }
            self.files.insert(path, document.into_bytes());
        }
        let text = format!(
            "{}\n{size}\n{}\nita-map {}\n{}\n",
            self.keys.log.1.name,
            encode_base64(&top),
            encode_base64(&map_root),
            self.root_line()
        );
        let log_line = sign_line(&self.keys.log.0, &self.keys.log.1, &text);
        let mut document = format!("{text}\n{log_line}");
        for ((pair, key), time) in self.keys.witnesses.iter().zip(times) {
            if let Some(time) = time {
                document.push_str(&cosign_line(pair, key, &text, *time));
            }
        }
        if !self.sparse {
            self.files
                .insert(format!("checkpoint/{size}"), document.clone().into_bytes());
        }
        self.files
            .insert("checkpoint".to_owned(), document.into_bytes());
        if self.sizes.last() != Some(&size) {
            self.sizes.push(size);
        }
        self.tiles(&leaves);
        // Map tiles are named by their content: every checkpoint's stay served.
        for (path, bytes) in tiles {
            self.files.insert(path, bytes);
        }
        size
    }

    /// The sparse mode's kept checkpoints of the sizes up to `size`, asked at `time`: each new
    /// size gets a time of its own, increasing from the last size asked at, and witnesses
    /// cosign only the start and deadline checkpoints of requests, the checkpoint before each
    /// deadline, and one per 64 sizes.
    fn sparse_kept(&mut self, size: u64, time: u64, leaves: &[Hash], map_root: &Hash) {
        let (from, start) = self.sparse_last.unwrap_or((0, time.saturating_sub(size)));
        let mut previous = start;
        let mut cosign: Vec<u64> = Vec::new();
        for kept in from + 1..=size {
            let span = size - from;
            let share = time.saturating_sub(start).saturating_mul(kept - from) / span.max(1);
            let at = (start + share).max(previous + 1).min(time);
            previous = at;
            self.sparse_times.insert(kept, at);
            let prefix = leaves.get(..kept as usize).unwrap_or_default();
            let text = format!(
                "{}\n{kept}\n{}\nita-map {}\n{}\n",
                self.keys.log.1.name,
                encode_base64(&tree_root(prefix)),
                encode_base64(map_root),
                self.root_line()
            );
            self.sparse_texts.insert(kept, text);
            if kept % 64 == 0 {
                cosign.push(kept);
            }
            let times = self.witness_times(at);
            if self.requests.contains(&(kept - 1)) {
                cosign.push(kept);
                // The start: the k-th earliest of its cosignatures.
                let start = times.get(self.witness_k - 1).copied().unwrap_or(at);
                self.sparse_deadlines.push((start + 7 * 24 * 3600, false));
            }
            // The witness time: the k-th latest of its cosignatures.
            let witness_time = times
                .iter()
                .rev()
                .nth(self.witness_k - 1)
                .copied()
                .unwrap_or(at);
            for (target, done) in &mut self.sparse_deadlines {
                if !*done && witness_time >= *target {
                    *done = true;
                    cosign.push(kept);
                    cosign.push(kept - 1);
                }
            }
        }
        for kept in from + 1..=size {
            self.write_sparse(kept, cosign.contains(&kept));
        }
        // A size before this call cosigned now, as the one before a deadline.
        for kept in cosign.iter().filter(|kept| **kept <= from) {
            self.write_sparse(*kept, true);
        }
        self.sparse_last = Some((size, time));
    }

    /// The times the witnesses cosign a checkpoint of time `at` in the sparse mode, in their
    /// order, ascending: `spread` apart around it.
    fn witness_times(&self, at: u64) -> Vec<u64> {
        let n = self.keys.witnesses.len() as u64;
        (0..n)
            .map(|i| (at + i * self.spread).saturating_sub((n - 1) * self.spread / 2))
            .collect()
    }

    /// Writes the sparse mode's kept checkpoint of `size`, cosigned at its time when asked.
    fn write_sparse(&mut self, size: u64, cosigned: bool) {
        let (Some(text), Some(at)) = (self.sparse_texts.get(&size), self.sparse_times.get(&size))
        else {
            return;
        };
        let mut document = format!(
            "{text}\n{}",
            sign_line(&self.keys.log.0, &self.keys.log.1, text)
        );
        if cosigned {
            let times = self.witness_times(*at);
            for ((pair, key), time) in self.keys.witnesses.iter().zip(times) {
                document.push_str(&cosign_line(pair, key, text, time));
            }
        }
        self.files
            .insert(format!("checkpoint/{size}"), document.into_bytes());
    }

    /// [`TestRegistry::checkpoint_with`], every witness cosigning at `time`.
    pub fn checkpoint(&mut self, time: u64) -> u64 {
        let times = vec![Some(time); self.keys.witnesses.len()];
        self.checkpoint_with(&times)
    }

    /// The hash tiles and entry bundles of the tree of `leaves`.
    fn tiles(&mut self, leaves: &[Hash]) {
        let size = leaves.len() as u64;
        let mut level = 0u64;
        loop {
            let span = 1u64 << (8 * level);
            let nodes = size / span;
            if nodes == 0 {
                break;
            }
            let mut index = 0;
            while index * WIDTH < nodes {
                let width = (nodes - index * WIDTH).min(WIDTH);
                let mut bytes = Vec::new();
                for at in 0..width {
                    let node = index * WIDTH + at;
                    let start = (node * span) as usize;
                    let end = ((node + 1) * span) as usize;
                    bytes.extend_from_slice(&tree_root(&leaves[start..end]));
                }
                self.files.insert(tile_path(level, index, width), bytes);
                index += 1;
            }
            level += 1;
        }
        let mut index = 0;
        while index * WIDTH < size {
            let width = (size - index * WIDTH).min(WIDTH);
            let mut bytes = Vec::new();
            for at in 0..width {
                let entry = &self.entries[(index * WIDTH + at) as usize];
                bytes.extend_from_slice(&(entry.len() as u16).to_be_bytes());
                bytes.extend_from_slice(entry);
            }
            self.files.insert(entries_path(index, width), bytes);
            index += 1;
        }
    }

    /// Writes every file below `dir`, as a registry's store keeps it: the latest checkpoint at
    /// `checkpoint/latest`.
    pub fn write(&self, dir: &Path) {
        for (path, bytes) in &self.files {
            let file = if path == "checkpoint" {
                dir.join("checkpoint").join("latest")
            } else {
                dir.join(path)
            };
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
    }
}

/// One root version of a walked chain, for the root-version floor's store, from a made-up
/// `digest` that stands for both the digest of its text and the digest of its whole note.
pub fn walked(version: u64, digest: &str) -> una_project::itaoca::WalkedRoot {
    una_project::itaoca::WalkedRoot {
        version,
        text: digest.to_owned(),
        note: digest.to_owned(),
    }
}

/// The root metadata `document` of `version` as one version of a walked chain: the digest of
/// its text, which the floor stores, and the digest of its whole signed note.
pub fn walked_root(version: u64, document: &str) -> una_project::itaoca::WalkedRoot {
    una_project::itaoca::WalkedRoot {
        version,
        text: text_digest(document).unwrap(),
        note: digest(document),
    }
}

/// The fingerprint of a key, for `revoke` lines.
pub fn fingerprint_of(key: &Key) -> String {
    fingerprint(&key.public)
}
