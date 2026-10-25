//! What publishing, `una ita audit` and `una ita watch` read from the verified registry
//! client (ADR 0122 rules 35 to 39): a name's state and an owner's key events, verified at a
//! checkpoint fresh by rule 39.
//!
//! This module names the interface only. The client that implements [`Registry`] walks the
//! root chain, accepts a checkpoint with its witnesses' cosignatures and a consistency proof
//! from the one stored, verifies the name map's proof and the name's chain with the count of
//! rule 36.1, each entry's inclusion, and every signature against the keys valid at its log
//! position; it reports a failure of any of these as [`Failure::Refused`] (U7010), never as a
//! state. So nothing that reads a [`NameState`] or an [`OwnerState`] takes a registry's or a
//! mirror's word for it.

use std::collections::BTreeMap;

use crate::tlog::note::Key;
use crate::tlog::release::Statement;
use crate::tlog::state::{Advisory, ReleaseState};
use crate::version::Version;

/// An entry of a name's chain, verified: its log index, its leaf hash, its format line (such as
/// `ita-release/1` or `ita-yank/1`) and its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainEntry {
    /// Its log index.
    pub index: u64,
    /// Its leaf hash, `sha256:` and hex.
    pub leaf: String,
    /// Its format line.
    pub format: String,
    /// The note's text.
    pub text: String,
}

/// The state of a package's name at a checkpoint, verified (rules 24, 36.1 and 41).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NameState {
    /// The source.
    pub source: String,
    /// The name.
    pub name: String,
    /// The checkpoint's size, which is the resolution point it gives.
    pub checkpoint_size: u64,
    /// The checkpoint's witness time (rule 36.4), in seconds.
    pub checkpoint_time: u64,
    /// The name's count of entries in the map.
    pub count: u64,
    /// The head of the name's chain, its log index and leaf hash, or none for a name with no
    /// entry, as the map's proof of absence shows.
    pub head: Option<(u64, String)>,
    /// Its releases, folded oldest first ([`crate::tlog::state::fold`]).
    pub releases: BTreeMap<Version, ReleaseState>,
    /// The release statement of each release, by version, for rule 26.
    pub statements: BTreeMap<Version, Statement>,
    /// The advisories of the name, by identifier.
    pub advisories: BTreeMap<String, Advisory>,
    /// The entries after the first `held` of the chain that the caller asked for, oldest
    /// first.
    pub entries: Vec<ChainEntry>,
}

/// An owner's key events at a checkpoint, verified from the owner's chain (rules 38 and 38.1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OwnerState {
    /// The source.
    pub source: String,
    /// The owner, or `itauna` for the names without one.
    pub owner: String,
    /// The checkpoint's witness time (rule 36.4), in seconds.
    pub checkpoint_time: u64,
    /// The owner's count of entries in the map.
    pub count: u64,
    /// The owner's keys valid at the head of its chain, each with the name its signature lines
    /// carry (for `itauna`, the keys of the root metadata).
    pub keys: Vec<Key>,
    /// How many of them must sign a release, a yank or a key event (rule 38).
    pub threshold: usize,
    /// Keys revoked with "compromised since log entry p": the key's `signed` form,
    /// `ed25519:<owner>:<fingerprint>`, and p.
    pub compromised: Vec<(String, u64)>,
    /// The log indexes of the recoveries pending: logged, neither cancelled nor in effect.
    pub pending_recoveries: Vec<u64>,
    /// The log indexes of the key events: a key added, rotated or revoked, a recovery in
    /// effect, a dispute.
    pub key_events: Vec<u64>,
    /// The transfers the owner's chain refers to: their log index and the package's name.
    pub transfers: Vec<(u64, String)>,
    /// The owner's package names that the registry lists. Only a hint of which chains to walk:
    /// each chain is verified by itself.
    pub packages: Vec<String>,
    /// The entries after the first `held` of the chain that the caller asked for, oldest
    /// first.
    pub entries: Vec<ChainEntry>,
}

/// Why a state is not given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The client cannot reach a verified state: no client, a refusal of service, no fresh
    /// checkpoint. The text names why.
    Unavailable(String),
    /// Something failed verification: evidence against the registry or a mirror (U7010).
    Refused(String),
}

/// The verified registry client, as publishing, audit and watch use it.
pub trait Registry {
    /// The state of `name` at `source`, at a checkpoint fresh by rule 39, with the entries of its
    /// chain after the first `held`.
    fn name(&self, source: &str, name: &str, held: u64) -> Result<NameState, Failure>;

    /// The key events of `owner` at `source`, at a checkpoint fresh by rule 39, with the entries
    /// of its chain after the first `held`.
    fn owner(&self, source: &str, owner: &str, held: u64) -> Result<OwnerState, Failure>;

    /// The log's key of `source`, from its last root metadata, verified (rule 35): the key
    /// that signs checkpoints and the registry's promises (rule 36.2).
    fn log_key(&self, source: &str) -> Result<Key, Failure>;
}

/// The owner of a package's name: the part before `/`, or `itauna` for a name without one
/// (rule 38.1).
pub fn owner_of(name: &str) -> &str {
    name.split_once('/').map_or("itauna", |(owner, _)| owner)
}
