//! The ordered hash table (ADR 0119 rule 19), in safe Rust, shared by both engines and by the
//! arena index of ADR 0111 rule 7.
//!
//! - **The entries:** a dense vector of `(hash, key, value)` or tombstones, in insertion order.
//!   Listing walks it, skipping tombstones.
//! - **The index:** open addressing with linear probing, over slots that hold a position in the
//!   entries plus one (0 for an empty slot) and the upper 32 bits of the entry's hash, which a
//!   probe compares before it reads the entry. Its capacity is the smallest power of two at least
//!   twice the number of entries, tombstones included, and at least 8: a function of that count
//!   alone, never of the keys.
//! - **Removal** leaves a tombstone in the entries and its slot in the index; lookups pass over a
//!   slot whose entry is a tombstone.
//! - **Growth and compaction:** when the entries reach half the index's capacity the index
//!   doubles and is rebuilt from the stored hashes, with no key hashed again; when tombstones
//!   pass half the entries, the entries are compacted in order and the index rebuilt. Positions
//!   are 32 bits up to 2^32 - 1 entries, and the slots widen past that.
//! - **The index is never iterated:** only lookups read it, so no listing depends on where hashes
//!   land.
//!
//! The engines hash a key (`crate::hash`) and compare keys with their own `==`; the table never
//! sees a key's type.

/// One entry of the table, or a tombstone.
struct Entry<K, V> {
    hash: u64,
    key: K,
    value: V,
}

/// A slot that holds no entry.
const EMPTY: u64 = 0;

/// The index: narrow slots of 8 bytes while positions fit in 32 bits, wide ones past that.
enum Index {
    /// `(position + 1) | (upper 32 bits of the hash) << 32`, or 0.
    Narrow(Vec<u64>),
    /// `(position + 1, upper 32 bits of the hash)`, or `(0, 0)`.
    Wide(Vec<(u64, u32)>),
}

impl Index {
    /// An empty index of `capacity` slots. It holds at most `capacity / 2` entries before it
    /// grows, so its positions plus one fit in 32 bits while that does.
    fn with_capacity(capacity: usize) -> Index {
        if capacity / 2 <= u32::MAX as usize {
            Index::Narrow(vec![EMPTY; capacity])
        } else {
            Index::Wide(vec![(0, 0); capacity])
        }
    }

    fn capacity(&self) -> usize {
        match self {
            Index::Narrow(slots) => slots.len(),
            Index::Wide(slots) => slots.len(),
        }
    }

    /// The slot at `at`: its position plus one (0 when empty) and its hash tag.
    fn slot(&self, at: usize) -> (u64, u32) {
        match self {
            Index::Narrow(slots) => {
                let s = slots.get(at).copied().unwrap_or(EMPTY);
                (s & 0xffff_ffff, (s >> 32) as u32)
            }
            Index::Wide(slots) => slots.get(at).copied().unwrap_or((0, 0)),
        }
    }

    fn set(&mut self, at: usize, position: usize, tag: u32) {
        match self {
            Index::Narrow(slots) => {
                if let Some(s) = slots.get_mut(at) {
                    *s = (position as u64 + 1) | (u64::from(tag) << 32);
                }
            }
            Index::Wide(slots) => {
                if let Some(s) = slots.get_mut(at) {
                    *s = (position as u64 + 1, tag);
                }
            }
        }
    }
}

/// The capacity of the index for `entries` entries, tombstones included: the smallest power of
/// two at least twice the count, and at least 8.
pub fn index_capacity(entries: usize) -> usize {
    entries.saturating_mul(2).max(8).next_power_of_two()
}

/// The tag a slot keeps of a hash: its upper 32 bits.
fn tag(hash: u64) -> u32 {
    (hash >> 32) as u32
}

/// An ordered hash table from keys to values, listed in the order keys were first added.
pub struct OrderedTable<K, V> {
    entries: Vec<Option<Entry<K, V>>>,
    live: usize,
    index: Index,
}

impl<K, V> Default for OrderedTable<K, V> {
    fn default() -> Self {
        OrderedTable::new()
    }
}

impl<K, V> OrderedTable<K, V> {
    /// An empty table.
    pub fn new() -> OrderedTable<K, V> {
        OrderedTable {
            entries: Vec::new(),
            live: 0,
            index: Index::with_capacity(index_capacity(0)),
        }
    }

    /// The number of keys.
    pub fn len(&self) -> usize {
        self.live
    }

    /// Whether the table has no key.
    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// The index's capacity, a function of the entries' count alone (for the tests of rule 19).
    pub fn capacity(&self) -> usize {
        self.index.capacity()
    }

    /// The entries in insertion order, keys and values.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries.iter().flatten().map(|e| (&e.key, &e.value))
    }

    /// The position in the entries of the key with `hash` for which `same` holds, if present.
    /// Expected O(1) probes under a random key: at a load of at most one half, about 1.5 slots
    /// for a present key and 2.5 for an absent one.
    fn find(&self, hash: u64, same: &mut impl FnMut(&K) -> bool) -> Option<usize> {
        let capacity = self.index.capacity();
        let mask = capacity - 1;
        let mut at = (hash as usize) & mask;
        for _ in 0..capacity {
            #[cfg(feature = "test-hooks")]
            crate::hooks::probe();
            let (position, slot_tag) = self.index.slot(at);
            if position == 0 {
                return None;
            }
            if slot_tag == tag(hash) {
                let position = (position - 1) as usize;
                if let Some(Some(entry)) = self.entries.get(position)
                    && entry.hash == hash
                    && same(&entry.key)
                {
                    return Some(position);
                }
            }
            at = (at + 1) & mask;
        }
        None
    }

    /// The value of the key with `hash` for which `same` holds.
    pub fn get(&self, hash: u64, mut same: impl FnMut(&K) -> bool) -> Option<&V> {
        let position = self.find(hash, &mut same)?;
        self.entries.get(position)?.as_ref().map(|e| &e.value)
    }

    /// The value of that key, to change in place.
    pub fn get_mut(&mut self, hash: u64, mut same: impl FnMut(&K) -> bool) -> Option<&mut V> {
        let position = self.find(hash, &mut same)?;
        self.entries
            .get_mut(position)?
            .as_mut()
            .map(|e| &mut e.value)
    }

    /// Whether a key with `hash` for which `same` holds is present.
    pub fn contains(&self, hash: u64, mut same: impl FnMut(&K) -> bool) -> bool {
        self.find(hash, &mut same).is_some()
    }

    /// Sets the value of `key`. A present key keeps its place in the listing and gets the new
    /// value, and the old one is returned; a new key goes last. Expected O(1), amortized over
    /// the index's growth.
    pub fn insert(
        &mut self,
        hash: u64,
        key: K,
        value: V,
        mut same: impl FnMut(&K) -> bool,
    ) -> Option<V> {
        if let Some(position) = self.find(hash, &mut same) {
            let entry = self.entries.get_mut(position)?.as_mut()?;
            return Some(core::mem::replace(&mut entry.value, value));
        }
        if self.entries.len() + 1 > self.index.capacity() / 2 {
            self.rebuild(index_capacity(self.entries.len() + 1));
        }
        let position = self.entries.len();
        self.entries.push(Some(Entry { hash, key, value }));
        self.live += 1;
        self.place(position, hash);
        None
    }

    /// Removes the key with `hash` for which `same` holds, and gives back its key and value.
    /// Its entry becomes a tombstone; when tombstones pass half the entries, the entries are
    /// compacted in order. Expected O(1), amortized over the compactions.
    pub fn remove(&mut self, hash: u64, mut same: impl FnMut(&K) -> bool) -> Option<(K, V)> {
        let position = self.find(hash, &mut same)?;
        let entry = self.entries.get_mut(position)?.take()?;
        self.live -= 1;
        let tombstones = self.entries.len() - self.live;
        if tombstones * 2 > self.entries.len() {
            self.entries.retain(Option::is_some);
            self.rebuild(index_capacity(self.entries.len()));
        }
        Some((entry.key, entry.value))
    }

    /// Puts the entry at `position` into the index, at the first empty slot of its probe
    /// sequence.
    fn place(&mut self, position: usize, hash: u64) {
        let capacity = self.index.capacity();
        let mask = capacity - 1;
        let mut at = (hash as usize) & mask;
        for _ in 0..capacity {
            if self.index.slot(at).0 == 0 {
                self.index.set(at, position, tag(hash));
                return;
            }
            at = (at + 1) & mask;
        }
    }

    /// A new index of `capacity` slots from the stored hashes, in O(n), with no key hashed again.
    fn rebuild(&mut self, capacity: usize) {
        self.index = Index::with_capacity(capacity);
        let hashes: Vec<(usize, u64)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.as_ref().map(|e| (i, e.hash)))
            .collect();
        for (position, hash) in hashes {
            self.place(position, hash);
        }
    }
}

impl<K: Clone, V: Clone> Clone for OrderedTable<K, V> {
    fn clone(&self) -> Self {
        let entries = self
            .entries
            .iter()
            .map(|e| {
                e.as_ref().map(|e| Entry {
                    hash: e.hash,
                    key: e.key.clone(),
                    value: e.value.clone(),
                })
            })
            .collect();
        let index = match &self.index {
            Index::Narrow(slots) => Index::Narrow(slots.clone()),
            Index::Wide(slots) => Index::Wide(slots.clone()),
        };
        OrderedTable {
            entries,
            live: self.live,
            index,
        }
    }
}
