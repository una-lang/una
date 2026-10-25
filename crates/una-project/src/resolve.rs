//! Minimal version selection (ADR 0122 rules 21 to 24): the build list of a program, from its
//! root's needs and the needs of the releases they reach, at one resolution point.
//!
//! For each identity (source, name, major line), the selection is the highest of the minimum
//! versions any reached release asks for, never a newer one, except where rule 24 raises it
//! past a yanked, advised or avoided release, to the least release above it that is none of
//! these and was logged at least the cooldown before the resolution point.
//!
//! Which identities admit a pre-release (rule 22) is a fixed point: a pass resolves with a fixed
//! set of them, and is run again from the root while a release it reached names a pre-release
//! of an identity not in the set ([`resolve`]).
//!
//! Costs: O(N + R) time and space per pass for N releases reached and R need edges among them,
//! with hash maps for the selection and the releases queued, plus O(log K) to find a release
//! among the K of its identity, pre-releases stepped over in O(1) through an index of the next
//! release that is not one, and O(S) per raise for the S releases it examines, each counted as
//! work; O(P log P) to sort the build list of P packages. A pass is repeated only when it
//! discovers an identity named with a pre-release beyond the root. There is no backtracking
//! and no search. Every bound of rule 23 is checked as the walk goes, each distinct release
//! visited counted once over every pass and bounding the stack too, so no input makes it do
//! more than the bounds allow. A pass that fails reports its least failure by identity and
//! version, never the first one its walk met.

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
    /// the cooldown holds back.
    NoRaise {
        /// The identity.
        identity: Identity,
        /// The release that must be left.
        from: Version,
        /// The newest candidate within the cooldown, if any.
        held: Option<Version>,
    },
    /// The admission of pre-releases settled on a set that lets the build list hold a
    /// pre-release of an identity that no requirement of the last pass names with one (rule
    /// 22): refused rather than selected.
    PreRelease {
        /// The identity.
        identity: Identity,
        /// The pre-release selected.
        version: Version,
    },
    /// A bound of rule 23 is exceeded.
    Bound(String),
    /// The index could not answer.
    Index(String),
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
                "{identity} must be raised past {from}, and {held}, the release that would replace it, was logged within the cooldown; review it with `una ita audit`, or write a need of a version past {from}"
            ),
            Problem::NoRaise {
                identity,
                from,
                held: None,
            } => write!(
                f,
                "{identity} must be raised past {from}, which is yanked, under an advisory or avoided, and no later release qualifies"
            ),
            Problem::PreRelease { identity, version } => write!(
                f,
                "the pre-release {version} of {identity} would be selected, though no need the resolution reaches names a pre-release of it; write a need of the version to take"
            ),
            Problem::Bound(what) => write!(f, "the resolution exceeds a bound: {what}"),
            Problem::Index(why) => f.write_str(why),
        }
    }
}

/// A resolution: the build list, sorted by identity, and the work it took.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// The selected version of every identity reachable from the root.
    pub list: BTreeMap<Identity, Version>,
    /// Release visits, need edges walked, release lookups and releases examined by raises,
    /// over every pass, for the scaling test.
    pub work: usize,
}

/// Resolves `root` (rules 22 to 24). The result depends only on the needs and the per-name
/// state, never on the order of any list.
///
/// Which identities admit a pre-release (rule 22) is a fixed point, computed before the
/// selection that is returned: a pass resolves with a set of admitted identities, starting with
/// those the root's needs name with a pre-release; when a release it reached has a need that
/// names a pre-release of an identity not in the set, the identity is added and the pass is run
/// again from the root, until a pass adds none. Each pass has a fixed set, so its selection is
/// a function of the needs and the per-name state, and so is the last one's: a release visited
/// under one set never mixes with the selection made under the next. The set only grows, so it
/// may admit an identity that only a release of an earlier pass named; when the build list then
/// holds a pre-release of an identity that no need of the last pass names with one, there is
/// no resolution ([`Problem::PreRelease`]).
///
/// Costs: O(N + R) per pass, plus O(log K) per visit to find a release and O(S) per raise for
/// the S releases it examines; a pass is repeated only when it discovers a pre-release named
/// beyond the root, at most once per such identity, and the bound of releases visited counts
/// each distinct release once over every pass, so no input does more than the bounds allow.
/// A need that names a pre-release admits pre-releases of its identity itself, in every pass.
/// A pass that fails and admits no new identity reports its least failure by identity and
/// version; one that admits a new identity is run again, since the failure may be a need that
/// only an admitted pre-release meets.
pub fn resolve(
    root: &[Need],
    index: &dyn Index,
    policy: &Policy,
    bounds: Bounds,
) -> Result<Resolution, Problem> {
    if root.len() > bounds.needs {
        return Err(Problem::Bound(format!(
            "the root has more than {} needs",
            bounds.needs
        )));
    }
    let mut resolver = Resolver {
        index,
        policy,
        bounds,
        sel: HashMap::new(),
        releases: HashMap::new(),
        needs: HashMap::new(),
        pre_allowed: root
            .iter()
            .filter(|need| need.version.is_pre())
            .map(Need::identity)
            .collect(),
        visited: HashSet::new(),
        work: 0,
    };
    let named = loop {
        let pass = resolver.pass(root)?;
        let before = resolver.pre_allowed.len();
        resolver.pre_allowed.extend(pass.named.iter().cloned());
        if resolver.pre_allowed.len() > before {
            // A pass that admits more is run again, even when this one failed: the failure
            // may be a need that only an admitted pre-release meets.
            continue;
        }
        if let Some((_, problem)) = pass.failed {
            return Err(problem);
        }
        break pass.named;
    };
    let resolution = resolver.list(root)?;
    // A set only grows, so it may hold an identity that only a release of an earlier pass
    // named; a pre-release of it is never selected on that ground (rule 22).
    if let Some((identity, version)) = resolution
        .list
        .iter()
        .find(|(identity, version)| version.is_pre() && !named.contains(*identity))
    {
        return Err(Problem::PreRelease {
            identity: identity.clone(),
            version: version.clone(),
        });
    }
    Ok(resolution)
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

/// What one pass of [`resolve`] found: the identities its needs name with a pre-release, and
/// its least failure, if any.
struct Pass {
    named: HashSet<Identity>,
    failed: Option<((Identity, Version), Problem)>,
}

/// Keeps `problem`, met at `at`, when it is the least failure of the pass so far.
fn fail(
    failed: &mut Option<((Identity, Version), Problem)>,
    at: (Identity, Version),
    problem: Problem,
) {
    if failed.as_ref().is_none_or(|(least, _)| at < *least) {
        *failed = Some((at, problem));
    }
}

/// The releases of an identity, read once: ascending, on its line, each version once.
struct Held {
    releases: Vec<Release>,
    /// For each index `i`, and `releases.len()`: the least index at or after `i` whose release
    /// is not a pre-release, or `releases.len()`. Skipping pre-releases is then O(1) per step.
    next_final: Vec<usize>,
}

impl Held {
    fn new(mut releases: Vec<Release>, line: &Line) -> Held {
        releases.retain(|release| release.version.line() == *line);
        releases.sort_by(|a, b| a.version.cmp(&b.version));
        releases.dedup_by(|a, b| a.version == b.version);
        let mut next_final = Vec::with_capacity(releases.len() + 1);
        let mut next = releases.len();
        next_final.push(next);
        for (at, release) in releases.iter().enumerate().rev() {
            if !release.version.is_pre() {
                next = at;
            }
            next_final.push(next);
        }
        next_final.reverse();
        Held {
            releases,
            next_final,
        }
    }

    /// The index of the first release at or after `at` that a need may reach: any release when
    /// pre-releases are admitted, else the next one that is not a pre-release. O(1).
    fn candidate(&self, at: usize, pre_allowed: bool) -> usize {
        let end = self.releases.len();
        if pre_allowed {
            at.min(end)
        } else {
            self.next_final.get(at).copied().unwrap_or(end)
        }
    }
}

struct Resolver<'a> {
    index: &'a dyn Index,
    policy: &'a Policy,
    bounds: Bounds,
    /// The selection of the current pass.
    sel: HashMap<Identity, Version>,
    /// The releases of each identity read, kept across passes.
    releases: HashMap<Identity, Held>,
    /// The needs of each release read, kept across passes.
    needs: HashMap<(Identity, Version), Vec<Need>>,
    /// The identities whose pre-releases a need may reach in this pass (rule 22).
    pre_allowed: HashSet<Identity>,
    /// Each (identity, version) visited, over every pass, for the bound of rule 23.
    visited: HashSet<(Identity, Version)>,
    /// Visits, need edges, lookups and releases examined by raises, for the scaling test.
    work: usize,
}

impl Resolver<'_> {
    /// One pass of minimal version selection with the admitted pre-releases fixed: the
    /// selection in `sel`, and the identities that a need of the root or of a release reached
    /// names with a pre-release, which the next pass admits.
    fn pass(&mut self, root: &[Need]) -> Result<Pass, Problem> {
        self.sel.clear();
        let mut named: HashSet<Identity> = HashSet::new();
        // The least failure of the pass, by identity and version: a pass does not stop at the
        // first failure it meets, so which one it reports never depends on the order of the
        // walk.
        let mut failed: Option<((Identity, Version), Problem)> = None;
        // Each (identity, version) is queued once a pass, so the stack never holds more entries
        // than releases the bound lets be visited.
        let mut queued: HashSet<(Identity, Version)> = HashSet::new();
        let mut work: Vec<(Identity, Version)> = Vec::new();
        for need in root {
            if need.version.is_pre() {
                named.insert(need.identity());
            }
            self.queue(&mut work, &mut queued, need.identity(), need.version.clone())?;
        }
        let mut dirty: BTreeSet<Identity> = BTreeSet::new();
        loop {
            while let Some((identity, version)) = work.pop() {
                self.work += 1;
                let release = match self.release(&identity, &version) {
                    Ok(release) => release,
                    Err(problem) => {
                        fail(&mut failed, (identity, version), problem);
                        continue;
                    }
                };
                let raise = match self.sel.get(&identity) {
                    Some(current) => *current < release.version,
                    None => true,
                };
                if raise {
                    self.sel.insert(identity.clone(), release.version.clone());
                    dirty.insert(identity.clone());
                }
                let needs = match self.needs_of(&identity, &release.version) {
                    Ok(needs) => needs,
                    Err(problem) => {
                        fail(&mut failed, (identity, release.version), problem);
                        continue;
                    }
                };
                for need in needs {
                    self.work += 1;
                    let next = need.identity();
                    if next == identity {
                        let problem = Problem::SelfNeed(identity.clone());
                        fail(
                            &mut failed,
                            (identity.clone(), release.version.clone()),
                            problem,
                        );
                        continue;
                    }
                    if need.version.is_pre() {
                        named.insert(next.clone());
                    }
                    self.queue(&mut work, &mut queued, next, need.version)?;
                }
            }
            // Rule 24: raise each changed selection that may not stand.
            let changed = std::mem::take(&mut dirty);
            for identity in changed {
                let Some(current) = self.sel.get(&identity).cloned() else {
                    continue;
                };
                match self.raise(&identity, &current) {
                    Ok(Some(target)) => self.queue(&mut work, &mut queued, identity, target)?,
                    Ok(None) => {}
                    Err(problem) => fail(&mut failed, (identity, current), problem),
                }
            }
            if work.is_empty() {
                break;
            }
        }
        Ok(Pass { named, failed })
    }

    /// Queues a visit of `version` of `identity`, once a pass. Every entry queued is visited;
    /// the bound of releases visited counts each distinct (identity, version) once, over every
    /// pass, so a pass run again visits what it visited before at no cost to the bound, and the
    /// bound limits the stack too.
    fn queue(
        &mut self,
        work: &mut Vec<(Identity, Version)>,
        queued: &mut HashSet<(Identity, Version)>,
        identity: Identity,
        version: Version,
    ) -> Result<(), Problem> {
        let entry = (identity, version);
        if queued.contains(&entry) {
            return Ok(());
        }
        if !self.visited.contains(&entry) {
            if self.visited.len() >= self.bounds.visited {
                return Err(Problem::Bound(format!(
                    "more than {} releases visited",
                    self.bounds.visited
                )));
            }
            self.visited.insert(entry.clone());
        }
        queued.insert(entry.clone());
        work.push(entry);
        Ok(())
    }

    /// The releases of `identity`, read once and sorted ascending.
    fn releases_of(&mut self, identity: &Identity) -> Result<&Held, Problem> {
        if !self.releases.contains_key(identity) {
            let releases = self.index.releases(identity).map_err(Problem::Index)?;
            if releases.len() > self.bounds.releases {
                return Err(Problem::Bound(format!(
                    "more than {} releases of {identity}",
                    self.bounds.releases
                )));
            }
            self.releases
                .insert(identity.clone(), Held::new(releases, &identity.line));
        }
        self.releases
            .get(identity)
            .ok_or_else(|| Problem::Index(format!("the releases of {identity} were not kept")))
    }

    /// The release a need of `version` reaches: the least release of the identity at or
    /// above it, a pre-release only when the need names one or the pass admits one (rule 22).
    /// O(log K).
    fn release(&mut self, identity: &Identity, version: &Version) -> Result<Release, Problem> {
        // A need that names a pre-release admits pre-releases itself (rule 22).
        let pre_allowed = version.is_pre() || self.pre_allowed.contains(identity);
        self.work += 1;
        let held = self.releases_of(identity)?;
        let from = held.releases.partition_point(|r| r.version < *version);
        held.releases
            .get(held.candidate(from, pre_allowed))
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
            .map_err(Problem::Index)?;
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
    /// release above it that is not bad, not a pre-release (unless the pass admits one), and
    /// logged at least the cooldown before the resolution point, or a threshold fix. Returns the
    /// new selection, or `None` when `current` stands. O(log K + S) for K releases, of which the
    /// raise examines S, each counted as work; pre-releases not admitted are stepped over in
    /// O(1). A selection only rises, so the raises of one identity in a pass examine disjoint
    /// runs of its releases.
    fn raise(
        &mut self,
        identity: &Identity,
        current: &Version,
    ) -> Result<Option<Version>, Problem> {
        let policy = self.policy;
        // A pre-release selected was admitted by the need that reached it (rule 22).
        let pre_allowed = current.is_pre() || self.pre_allowed.contains(identity);
        self.work += 1;
        let held = self.releases_of(identity)?;
        let at = held
            .releases
            .binary_search_by(|r| r.version.cmp(current))
            .map_err(|_| Problem::Index(format!("{identity} {current} is not a release")))?;
        let Some(here) = held.releases.get(at) else {
            return Ok(None);
        };
        if !bad(policy, identity, here) {
            return Ok(None);
        }
        let mut examined = 0usize;
        let mut held_back: Option<Version> = None;
        let mut target: Option<Version> = None;
        let mut next = held.candidate(at + 1, pre_allowed);
        while let Some(release) = held.releases.get(next) {
            examined += 1;
            if !bad(policy, identity, release) {
                let aged = release.logged_at.saturating_add(policy.cooldown) <= policy.now;
                if aged || release.threshold_fix {
                    target = Some(release.version.clone());
                    break;
                }
                held_back = Some(release.version.clone());
            }
            next = held.candidate(next + 1, pre_allowed);
        }
        self.work = self.work.saturating_add(examined);
        match target {
            Some(version) => {
                self.sel.insert(identity.clone(), version.clone());
                Ok(Some(version))
            }
            None => Err(Problem::NoRaise {
                identity: identity.clone(),
                from: current.clone(),
                held: held_back,
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
                let next = need.identity();
                if !reached.contains(&next) {
                    stack.push(next);
                }
            }
            list.insert(identity, version);
        }
        Ok(Resolution {
            list,
            work: self.work,
        })
    }
}
