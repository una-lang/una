//! Minimal version selection (ADR 0122 rules 21 to 24): the build list of a program, from its
//! root's needs and the needs of the releases they reach, at one resolution point.
//!
//! For each identity (source, name, major line), the selection is the highest of the minimum
//! versions any reached release asks for, never a newer one, except where rule 24 raises it
//! past a yanked, advised or avoided release, to the least release above it that is none of
//! these and was logged at least the cooldown before the resolution point.
//!
//! Costs: O(N + R) time and space for N releases reached and R need edges among them, with
//! hash maps for the selection and the releases seen, plus O(log K + S) per raise for an
//! identity of K releases of which S are skipped, and O(P log P) to sort the build list of P
//! packages. There is no backtracking and no search. Every bound of rule 23 is checked as the
//! walk goes, so no input makes it do more than the bounds allow.

// A `Problem` ends a resolution, once: its size costs nothing on the path that succeeds.
#![allow(clippy::result_large_err)]

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;

use crate::version::{Line, Version};

/// What makes two packages the same (ADR 0122 rule 21): the source, the name, the major line.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Identity {
    /// The source: `itaty`, a source of the root, or the origin of a local package.
    pub source: String,
    /// The package's name, `name` or `owner/name`.
    pub name: String,
    /// The major line.
    pub line: Line,
}

impl fmt::Display for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} ({})", self.name, self.line, self.source)
    }
}

/// A need: an identity and the least version admitted on its line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Need {
    /// The source.
    pub source: String,
    /// The name.
    pub name: String,
    /// The least version.
    pub version: Version,
}

impl Need {
    /// The identity this need names.
    pub fn identity(&self) -> Identity {
        Identity {
            source: self.source.clone(),
            name: self.name.clone(),
            line: self.version.line(),
        }
    }
}

/// A release of an identity, as the per-name state at the resolution point gives it
/// (ADR 0122 rules 24 and 36.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// Its version.
    pub version: Version,
    /// Whether it is yanked at the resolution point.
    pub yanked: bool,
    /// Whether an advisory not fixed in it names it.
    pub advised: bool,
    /// Whether an advisory signed by the threshold of rule 41 names it, by digest, as its fix,
    /// which lifts the cooldown for it.
    pub threshold_fix: bool,
    /// When it was logged, in witness time (rule 36.4), in seconds.
    pub logged_at: u64,
}

/// What resolution reads: the releases of an identity and the needs of a release, from the
/// release statements and the per-name state at the resolution point (rules 23, 37).
pub trait Index {
    /// The releases of `identity`, in any order; an unknown identity has none.
    fn releases(&self, identity: &Identity) -> Result<Vec<Release>, String>;
    /// The needs of the release `version` of `identity`.
    fn needs(&self, identity: &Identity, version: &Version) -> Result<Vec<Need>, String>;
}

/// The bounds of rule 23.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    /// Releases visited.
    pub visited: usize,
    /// Packages in the build list.
    pub packages: usize,
    /// Needs of one release.
    pub needs: usize,
    /// Releases read for one name.
    pub releases: usize,
}

impl Default for Bounds {
    fn default() -> Bounds {
        Bounds {
            visited: 100_000,
            packages: 10_000,
            needs: 1_000,
            releases: 10_000,
        }
    }
}

/// The choices that resolution makes for the root (rules 24 and 25).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    /// The witness time of the resolution point, in seconds.
    pub now: u64,
    /// The cooldown, in seconds: 72 hours by default, 0 with `--now`.
    pub cooldown: u64,
    /// The versions the root avoids, by identity.
    pub avoid: BTreeMap<Identity, BTreeSet<Version>>,
}

/// The cooldown of rule 25, 72 hours, in seconds.
pub const COOLDOWN: u64 = 72 * 3600;

/// Why there is no resolution (U7013 where the command reports it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// No release of the identity meets a need: none at or above its version.
    NoRelease {
        /// The identity.
        identity: Identity,
        /// The least version needed.
        version: Version,
    },
    /// A release needs its own identity.
    SelfNeed(Identity),
    /// A selection must be raised, and no release qualifies; `held` is the newest candidate
    /// the cooldown holds back, which `--now` takes.
    NoRaise {
        /// The identity.
        identity: Identity,
        /// The release that must be left.
        from: Version,
        /// The newest candidate within the cooldown, if any.
        held: Option<Version>,
    },
    /// A bound of rule 23 is exceeded.
    Bound(String),
    /// The index could not answer.
    Index {
        /// The source whose index failed to answer, when the failure is a read of it; `None`
        /// when it is resolution's own record of what it read.
        source: Option<String>,
        /// Why, one sentence.
        why: String,
    },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::NoRelease { identity, version } => {
                write!(f, "no release of {identity} is {version} or later")
            }
            Problem::SelfNeed(identity) => {
                write!(f, "a release of {identity} needs its own identity")
            }
            Problem::NoRaise {
                identity,
                from,
                held: Some(held),
            } => write!(
                f,
                "{identity} must be raised past {from}, and {held} is within the cooldown; `--now` takes it"
            ),
            Problem::NoRaise {
                identity,
                from,
                held: None,
            } => write!(
                f,
                "{identity} must be raised past {from}, which is yanked, under an advisory or avoided, and no later release qualifies"
            ),
            Problem::Bound(what) => write!(f, "the resolution exceeds a bound: {what}"),
            Problem::Index { why, .. } => f.write_str(why),
        }
    }
}

/// A resolution: the build list, sorted by identity, and the work it took.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// The selected version of every identity reachable from the root.
    pub list: BTreeMap<Identity, Version>,
    /// Release visits plus need edges walked, for the scaling test.
    pub work: usize,
}

/// Resolves `root` (rules 23 and 24). The result depends only on the needs and the per-name
/// state, never on the order of any list.
pub fn resolve(
    root: &[Need],
    index: &dyn Index,
    policy: &Policy,
    bounds: Bounds,
) -> Result<Resolution, Problem> {
    let mut resolver = Resolver {
        index,
        policy,
        bounds,
        sel: HashMap::new(),
        seen: HashSet::new(),
        releases: HashMap::new(),
        needs: HashMap::new(),
        pre_allowed: HashSet::new(),
        work: 0,
    };
    if root.len() > bounds.needs {
        return Err(Problem::Bound(format!(
            "the root has more than {} needs",
            bounds.needs
        )));
    }
    let mut work: Vec<(Identity, Version)> = Vec::new();
    for need in root {
        resolver.note_pre(need);
        work.push((need.identity(), need.version.clone()));
    }
    let mut dirty: BTreeSet<Identity> = BTreeSet::new();
    loop {
        while let Some((identity, version)) = work.pop() {
            if !resolver.seen.insert((identity.clone(), version.clone())) {
                continue;
            }
            if resolver.seen.len() > bounds.visited {
                return Err(Problem::Bound(format!(
                    "more than {} releases visited",
                    bounds.visited
                )));
            }
            resolver.work += 1;
            let release = resolver.release(&identity, &version)?;
            let raise = match resolver.sel.get(&identity) {
                Some(current) => *current < release.version,
                None => true,
            };
            if raise {
                resolver
                    .sel
                    .insert(identity.clone(), release.version.clone());
                dirty.insert(identity.clone());
            }
            let needs = resolver.needs_of(&identity, &release.version)?;
            for need in needs {
                resolver.work += 1;
                let next = need.identity();
                if next == identity {
                    return Err(Problem::SelfNeed(identity));
                }
                resolver.note_pre(&need);
                work.push((next, need.version));
            }
        }
        // Rule 24: raise each changed selection that may not stand.
        let changed = std::mem::take(&mut dirty);
        for identity in changed {
            let Some(current) = resolver.sel.get(&identity).cloned() else {
                continue;
            };
            if let Some(target) = resolver.raise(&identity, &current)? {
                work.push((identity, target));
            }
        }
        if work.is_empty() {
            break;
        }
    }
    resolver.list(root)
}

/// The releases a command holds offline (ADR 0122 rules 23 and 25): each local package as its
/// directory holds it, one release of its identity with the needs its `ita.una` writes; and,
/// for each source whose per-name state is not held, why resolution there is refused. A source
/// is refused as a whole, so nothing is ever resolved against a partial view of a registry:
/// a release missing from the view could be a lower one that minimal version selection takes,
/// and a yank or an advisory missing from it could leave a release that must be raised.
#[derive(Clone, Debug, Default)]
pub struct Offline {
    releases: HashMap<Identity, Vec<(Release, Vec<Need>)>>,
    refused: BTreeMap<String, String>,
}

impl Offline {
    /// Adds the release `release` of `identity`, with its needs. A second release of the same
    /// version replaces nothing: the first one added stays.
    pub fn add(&mut self, identity: Identity, release: Release, needs: Vec<Need>) {
        let releases = self.releases.entry(identity).or_default();
        if !releases.iter().any(|(r, _)| r.version == release.version) {
            releases.push((release, needs));
        }
    }

    /// Refuses resolution at `source`, for `reason`, one sentence.
    pub fn refuse(&mut self, source: &str, reason: String) {
        self.refused.entry(source.to_owned()).or_insert(reason);
    }

    fn refusal(&self, identity: &Identity) -> Result<(), String> {
        match self.refused.get(&identity.source) {
            Some(reason) => Err(reason.clone()),
            None => Ok(()),
        }
    }
}

impl Index for Offline {
    fn releases(&self, identity: &Identity) -> Result<Vec<Release>, String> {
        self.refusal(identity)?;
        Ok(self
            .releases
            .get(identity)
            .map(|list| list.iter().map(|(release, _)| release.clone()).collect())
            .unwrap_or_default())
    }

    fn needs(&self, identity: &Identity, version: &Version) -> Result<Vec<Need>, String> {
        self.refusal(identity)?;
        self.releases
            .get(identity)
            .and_then(|list| list.iter().find(|(release, _)| release.version == *version))
            .map(|(_, needs)| needs.clone())
            .ok_or_else(|| format!("no release {version} of {identity} is held"))
    }
}

/// Whether a release may not stand as a selection (rule 24): yanked, under an advisory not
/// fixed in it, or avoided by the root.
fn bad(policy: &Policy, identity: &Identity, release: &Release) -> bool {
    release.yanked
        || release.advised
        || policy
            .avoid
            .get(identity)
            .is_some_and(|avoid| avoid.contains(&release.version))
}

struct Resolver<'a> {
    index: &'a dyn Index,
    policy: &'a Policy,
    bounds: Bounds,
    sel: HashMap<Identity, Version>,
    seen: HashSet<(Identity, Version)>,
    /// The releases of each identity read, ascending.
    releases: HashMap<Identity, Vec<Release>>,
    /// The needs of each release read.
    needs: HashMap<(Identity, Version), Vec<Need>>,
    /// The identities some requirement names a pre-release of (rule 22).
    pre_allowed: HashSet<Identity>,
    work: usize,
}

/// The index failed to answer for `identity`: a read of its source.
fn read_failed(identity: &Identity, why: String) -> Problem {
    Problem::Index {
        source: Some(identity.source.clone()),
        why,
    }
}

impl Resolver<'_> {
    fn note_pre(&mut self, need: &Need) {
        if need.version.is_pre() {
            self.pre_allowed.insert(need.identity());
        }
    }

    /// The releases of `identity`, read once and sorted ascending.
    fn releases_of(&mut self, identity: &Identity) -> Result<&Vec<Release>, Problem> {
        if !self.releases.contains_key(identity) {
            let mut releases = self
                .index
                .releases(identity)
                .map_err(|why| read_failed(identity, why))?;
            if releases.len() > self.bounds.releases {
                return Err(Problem::Bound(format!(
                    "more than {} releases of {identity}",
                    self.bounds.releases
                )));
            }
            releases.retain(|release| release.version.line() == identity.line);
            releases.sort_by(|a, b| a.version.cmp(&b.version));
            releases.dedup_by(|a, b| a.version == b.version);
            self.releases.insert(identity.clone(), releases);
        }
        self.releases
            .get(identity)
            .ok_or_else(|| Problem::Index {
                source: None,
                why: format!("the releases of {identity} were not kept"),
            })
    }

    /// The release a need of `version` reaches: the least release of the identity at or
    /// above it, a pre-release only when a requirement names one (rule 22). O(log K + S).
    fn release(&mut self, identity: &Identity, version: &Version) -> Result<Release, Problem> {
        let pre_allowed = self.pre_allowed.contains(identity);
        let releases = self.releases_of(identity)?;
        let from = releases.partition_point(|r| r.version < *version);
        releases
            .iter()
            .skip(from)
            .find(|r| !r.version.is_pre() || pre_allowed)
            .cloned()
            .ok_or_else(|| Problem::NoRelease {
                identity: identity.clone(),
                version: version.clone(),
            })
    }

    fn needs_of(&mut self, identity: &Identity, version: &Version) -> Result<Vec<Need>, Problem> {
        let key = (identity.clone(), version.clone());
        if let Some(needs) = self.needs.get(&key) {
            return Ok(needs.clone());
        }
        let needs = self
            .index
            .needs(identity, version)
            .map_err(|why| read_failed(identity, why))?;
        if needs.len() > self.bounds.needs {
            return Err(Problem::Bound(format!(
                "{identity} {version} has more than {} needs",
                self.bounds.needs
            )));
        }
        self.needs.insert(key, needs.clone());
        Ok(needs)
    }

    /// Raises the selection of `identity` past `current` when it may not stand, to the least
    /// release above it that is not bad, not a pre-release (unless one is named), and logged
    /// at least the cooldown before the resolution point, or a threshold fix. Returns the new
    /// selection, or `None` when `current` stands. O(log K + S) for K releases, S skipped.
    fn raise(
        &mut self,
        identity: &Identity,
        current: &Version,
    ) -> Result<Option<Version>, Problem> {
        let policy = self.policy;
        let pre_allowed = self.pre_allowed.contains(identity);
        self.releases_of(identity)?;
        let Some(releases) = self.releases.get(identity) else {
            return Ok(None);
        };
        let at = releases
            .binary_search_by(|r| r.version.cmp(current))
            .map_err(|_| Problem::Index {
                source: None,
                why: format!("{identity} {current} is not a release"),
            })?;
        let Some(here) = releases.get(at) else {
            return Ok(None);
        };
        if !bad(policy, identity, here) {
            return Ok(None);
        }
        let mut held: Option<Version> = None;
        let mut target: Option<Version> = None;
        for release in releases.iter().skip(at + 1) {
            if bad(policy, identity, release) || (release.version.is_pre() && !pre_allowed) {
                continue;
            }
            let aged = release.logged_at.saturating_add(policy.cooldown) <= policy.now;
            if aged || release.threshold_fix {
                target = Some(release.version.clone());
                break;
            }
            held = Some(release.version.clone());
        }
        match target {
            Some(version) => {
                self.sel.insert(identity.clone(), version.clone());
                Ok(Some(version))
            }
            None => Err(Problem::NoRaise {
                identity: identity.clone(),
                from: current.clone(),
                held,
            }),
        }
    }

    /// The identities reachable from the root by the needs of the selected releases, with
    /// their selections. O(P + R) for the walk, O(P log P) to sort.
    fn list(&mut self, root: &[Need]) -> Result<Resolution, Problem> {
        let mut list: BTreeMap<Identity, Version> = BTreeMap::new();
        let mut stack: Vec<Identity> = root.iter().map(Need::identity).collect();
        let mut reached: HashSet<Identity> = HashSet::new();
        while let Some(identity) = stack.pop() {
            if !reached.insert(identity.clone()) {
                continue;
            }
            let Some(version) = self.sel.get(&identity).cloned() else {
                continue;
            };
            if reached.len() > self.bounds.packages {
                return Err(Problem::Bound(format!(
                    "more than {} packages in the build list",
                    self.bounds.packages
                )));
            }
            for need in self.needs_of(&identity, &version)? {
                stack.push(need.identity());
            }
            list.insert(identity, version);
        }
        Ok(Resolution {
            list,
            work: self.work,
        })
    }
}
