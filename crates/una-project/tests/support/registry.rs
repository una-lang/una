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
use una_project::tlog::owner::fingerprint;
use una_project::tlog::root::digest;
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

/// A signed note of `text` by each of `signers`.
pub fn signed(text: &str, signers: &[&(SigningKey, Key)]) -> String {
    let mut document = format!("{text}\n");
    for (pair, key) in signers {
        document.push_str(&sign_line(pair, key, text));
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
        Keys {
            roots: (10..13)
                .map(|seed| keypair(seed, &format!("{source}.root.{seed}")))
                .collect(),
            log: keypair(20, &format!("{source}.example/log")),
            witnesses: (30..33)
                .map(|seed| keypair(seed, &format!("witness.{seed}")))
                .collect(),
            security: keypair(40, &format!("{source}.security")),
            itauna: keypair(50, &format!("{source}.itauna")),
        }
    }
}

/// The text of root metadata of `version`, with root keys `roots` (threshold 2), the log key,
/// three witnesses (threshold 2), one security key and one key of Itaúna.
pub fn root_text(source: &str, version: u64, expires: u64, keys: &Keys, roots: &[&Key]) -> String {
    let mut text = format!(
        "ita-root/1\nsource {source}\nversion {version}\nexpires {expires}\nroot 2\n"
    );
    for key in roots {
        text.push_str(&format!("key {}\n", key_text(key)));
    }
    text.push_str(&format!("log {}\n", key_text(&keys.log.1)));
    text.push_str("witnesses 2\n");
    for (_, key) in &keys.witnesses {
        text.push_str(&format!("key {}\n", key_text(key)));
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
}

impl TestRegistry {
    /// A registry of `source` with its first root metadata.
    pub fn new(source: &str) -> TestRegistry {
        let keys = Keys::new(source);
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
        };
        registry
            .files
            .insert("root/1".to_owned(), document.clone().into_bytes());
        registry.roots.push(document);
        registry
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
    pub fn append(&mut self, name: &str, document: &str) -> u64 {
        let index = self.entries.len() as u64;
        let leaf = leaf_hash(document.as_bytes());
        self.entries.push(document.as_bytes().to_vec());
        let count = self.heads.get(name).map_or(0, |h| h.count) + 1;
        let head = Head { index, leaf, count };
        self.heads.insert(name.to_owned(), head);
        index
    }

    /// Registers `owner` with `key`, signed by the log's key.
    pub fn register(&mut self, owner: &str, key: &Key) -> u64 {
        let text = format!(
            "ita-owner/1\nsource {}\nowner {owner}\n{}\nevent register\nkey {}",
            self.source,
            self.previous(owner),
            key_text(key)
        );
        let document = signed(&text, &[&self.keys.log]);
        self.append(owner, &document)
    }

    /// An owner's event, its lines after `event <kind>`, signed by `signers`.
    pub fn owner_event(
        &mut self,
        owner: &str,
        kind: &str,
        lines: &str,
        signers: &[&(SigningKey, Key)],
    ) -> u64 {
        let mut text = format!(
            "ita-owner/1\nsource {}\nowner {owner}\n{}\nevent {kind}",
            self.source,
            self.previous(owner)
        );
        if !lines.is_empty() {
            text.push('\n');
            text.push_str(lines);
        }
        let document = signed(&text, signers);
        self.append(owner, &document)
    }

    /// A recovery or a dispute of `owner` naming `key`, signed by the log's key and `more`.
    pub fn recover(
        &mut self,
        owner: &str,
        kind: &str,
        key: &Key,
        more: &[&(SigningKey, Key)],
    ) -> u64 {
        let text = format!(
            "ita-owner/1\nsource {}\nowner {owner}\n{}\nevent {kind}\nkey {}",
            self.source,
            self.previous(owner),
            key_text(key)
        );
        let log = (
            SigningKey::from_bytes(&self.keys.log.0.to_bytes()),
            self.keys.log.1.clone(),
        );
        let mut signers: Vec<&(SigningKey, Key)> = vec![&log];
        signers.extend_from_slice(more);
        let document = signed(&text, &signers);
        self.append(owner, &document)
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
        let manifest_text = format!("ita {name}\n  version {version}\n  edition 0.4\n");
        let module_text = format!("module {module}\n\npub answer() -> Int = 42\n");
        let source_path = format!("src/{module}.una");
        let (content, manifest) = self.blob(&[
            ("ita.una", manifest_text.as_str()),
            (source_path.as_str(), module_text.as_str()),
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
        let text = format!(
            "{}\n{size}\n{}\nita-map {}\n",
            self.keys.log.1.name,
            encode_base64(&top),
            encode_base64(&map_root)
        );
        let log_line = sign_line(&self.keys.log.0, &self.keys.log.1, &text);
        let mut document = format!("{text}\n{log_line}");
        for ((pair, key), time) in self.keys.witnesses.iter().zip(times) {
            if let Some(time) = time {
                document.push_str(&cosign_line(pair, key, &text, *time));
            }
        }
        self.files
            .insert(format!("checkpoint/{size}"), document.clone().into_bytes());
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

/// The fingerprint of a key, for `revoke` lines.
pub fn fingerprint_of(key: &Key) -> String {
    fingerprint(&key.public)
}
