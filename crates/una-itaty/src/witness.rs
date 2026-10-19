//! A witness (ADR 0122 rules 36 and 36.1): before it cosigns a checkpoint, it replays every
//! entry since the last checkpoint it cosigned, checks that the log was only appended to, that
//! each entry's `previous` is its name's head just before it and that its name's count rises by
//! one, recomputes the name map, and compares both roots with the checkpoint's. Its timestamps
//! never decrease.
//!
//! A witness holds its own copy of the log's stored hashes and of the map, so the registry's
//! word is used for nothing: the entries it is sent either rebuild the checkpoint's roots or
//! are refused. Costs: O(E log L) for E new entries, plus the map's path of each, and a copy of
//! its state, O(L + M), so that a refusal changes nothing.

use std::collections::HashMap;

use una_project::tlog::map::Head;
use una_project::tlog::merkle::Hash;
use una_project::tlog::note::{Key, parse_checkpoint, parse_note};

use crate::entry::parse_entry;
use crate::namemap::NameMap;
use crate::registry::map_root_of;
use crate::sign::Signer;
use crate::tree::{Tree, root};

/// A witness of one log.
pub struct Witness {
    signer: Signer,
    log: Key,
    tree: Tree,
    map: NameMap,
    heads: HashMap<String, Head>,
    last_time: u64,
}

impl std::fmt::Debug for Witness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Witness")
            .field("key", &self.signer.key().name)
            .field("size", &self.tree.size())
            .finish()
    }
}

impl Witness {
    /// A witness signing with `signer`, of the log whose key is `log`, that has seen nothing.
    pub fn new(signer: Signer, log: Key) -> Witness {
        Witness {
            signer,
            log,
            tree: Tree::new(),
            map: NameMap::new(),
            heads: HashMap::new(),
            last_time: 0,
        }
    }

    /// The size of the last checkpoint it cosigned, from which it needs the entries.
    pub fn size(&self) -> u64 {
        self.tree.size()
    }

    /// The witness's public key.
    pub fn key(&self) -> &Key {
        self.signer.key()
    }

    /// Checks the signed checkpoint `document` against the `entries` logged since the last
    /// checkpoint it cosigned, and returns its cosignature line at `now` (or its previous time,
    /// if later). Nothing changes when it refuses.
    pub fn cosign(
        &mut self,
        document: &str,
        entries: &[Vec<u8>],
        now: u64,
    ) -> Result<String, String> {
        let note = parse_note(document)?;
        if !note.signed_by(&self.log) {
            return Err("the checkpoint is not signed by the log's key".to_owned());
        }
        let checkpoint = parse_checkpoint(&note.text)?;
        if checkpoint.origin != self.log.name {
            return Err("the checkpoint names another log".to_owned());
        }
        let expected = self.tree.size() + entries.len() as u64;
        if checkpoint.size != expected {
            return Err(format!(
                "the checkpoint's size {} is not the {} entries seen and the {} sent",
                checkpoint.size,
                self.tree.size(),
                entries.len()
            ));
        }
        let map_root =
            map_root_of(&checkpoint).ok_or("the checkpoint carries no `ita-map` line")?;
        let mut tree = self.tree.clone();
        let mut map = self.map.clone();
        let mut heads = self.heads.clone();
        for (offset, bytes) in entries.iter().enumerate() {
            let index = self.tree.size() + offset as u64;
            let entry =
                parse_entry(bytes).map_err(|e| format!("the entry {index} does not read: {e}"))?;
            let head = heads.get(&entry.chain).copied();
            if entry.previous != head.map(|h| (h.index, h.leaf)) {
                return Err(format!(
                    "the entry {index}'s `previous` is not the head of `{}`",
                    entry.chain
                ));
            }
            let leaf = entry.leaf();
            tree.append(leaf);
            let next = Head {
                index,
                leaf,
                count: head.map_or(0, |h| h.count) + 1,
            };
            heads.insert(entry.chain.clone(), next);
            map.set(&entry.chain, &next);
        }
        let size = tree.size();
        let log_root: Hash = root(&mut tree, size)?;
        if log_root != checkpoint.root {
            return Err(
                "the entries do not give the checkpoint's root: the log was not only appended to"
                    .to_owned(),
            );
        }
        if map.root() != map_root {
            return Err("the entries do not give the checkpoint's name map".to_owned());
        }
        let time = now.max(self.last_time);
        let line = self.signer.cosignature(&note.text, time);
        self.tree = tree;
        self.map = map;
        self.heads = heads;
        self.last_time = time;
        Ok(line)
    }
}
