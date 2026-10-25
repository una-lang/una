//! Minimal version selection (ADR 0122 rules 21 to 24): the classic cases, raises past yanks,
//! advisories and avoided releases, the cooldown, each bound, and the scaling test.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::result_large_err,
    clippy::unwrap_used
)]

use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

use pretty_assertions::assert_eq;
use una_project::resolve::{
    Bounds, COOLDOWN, Identity, Index, Need, Offline, Policy, Problem, Release, resolve,
};
use una_project::version::Version;

fn v(text: &str) -> Version {
    Version::parse(text).unwrap().version
}

fn need(name: &str, version: &str) -> Need {
    Need {
        source: "itaty".to_owned(),
        name: name.to_owned(),
        version: v(version),
    }
}

/// An index in memory: per name, its releases with their needs.
#[derive(Default)]
struct Registry {
    releases: HashMap<String, Vec<(Release, Vec<Need>)>>,
}

impl Registry {
    fn add(&mut self, name: &str, version: &str, needs: &[(&str, &str)]) -> &mut Release {
        let entry = self.releases.entry(name.to_owned()).or_default();
        entry.push((
            Release {
                version: v(version),
                yanked: false,
                advised: false,
                threshold_fix: false,
                logged_at: 0,
            },
            needs.iter().map(|(n, ver)| need(n, ver)).collect(),
        ));
        &mut entry.last_mut().unwrap().0
    }
}

impl Index for Registry {
    fn releases(&self, identity: &Identity) -> Result<Vec<Release>, String> {
        Ok(self
            .releases
            .get(&identity.name)
            .map(|list| list.iter().map(|(r, _)| r.clone()).collect())
            .unwrap_or_default())
    }

    fn needs(&self, identity: &Identity, version: &Version) -> Result<Vec<Need>, String> {
        self.releases
            .get(&identity.name)
            .and_then(|list| list.iter().find(|(r, _)| r.version == *version))
            .map(|(_, needs)| needs.clone())
            .ok_or_else(|| format!("no statement for {identity} {version}"))
    }
}

/// The policy at a resolution point long after every release, with the default cooldown.
fn policy() -> Policy {
    Policy {
        now: 1_000_000_000,
        cooldown: COOLDOWN,
        avoid: BTreeMap::new(),
    }
}

/// The build list as "name version" lines.
fn list(root: &[Need], registry: &Registry, policy: &Policy) -> Vec<String> {
    resolve(root, registry, policy, Bounds::default())
        .unwrap()
        .list
        .iter()
        .map(|(identity, version)| format!("{} {version}", identity.name))
        .collect()
}

#[test]
fn a_diamond_takes_the_highest_minimum_never_the_newest() {
    let mut r = Registry::default();
    r.add("a", "1.0.0", &[("c", "1.1")]);
    r.add("b", "1.0.0", &[("c", "1.3")]);
    for version in ["1.0.0", "1.1.0", "1.2.0", "1.3.0", "1.4.0"] {
        r.add("c", version, &[]);
    }
    assert_eq!(
        list(&[need("a", "1.0"), need("b", "1.0")], &r, &policy()),
        ["a 1.0.0", "b 1.0.0", "c 1.3.0"]
    );
}

#[test]
fn a_cycle_resolves_and_two_majors_live_side_by_side() {
    let mut r = Registry::default();
    r.add("a", "1.0.0", &[("b", "1.0")]);
    r.add("b", "1.0.0", &[("a", "1.0"), ("json", "2.0")]);
    r.add("json", "1.5.0", &[]);
    r.add("json", "2.0.0", &[]);
    r.add("json", "2.1.0", &[]);
    assert_eq!(
        list(&[need("a", "1.0"), need("json", "1.5")], &r, &policy()),
        ["a 1.0.0", "b 1.0.0", "json 1.5.0", "json 2.0.0"]
    );
}

#[test]
fn a_need_reaches_the_least_release_at_or_above_it() {
    let mut r = Registry::default();
    r.add("json", "2.0.3", &[]);
    r.add("json", "2.0.4", &[]);
    assert_eq!(list(&[need("json", "2.0")], &r, &policy()), ["json 2.0.3"]);
}

#[test]
fn a_later_release_changes_nothing() {
    let mut r = Registry::default();
    r.add("json", "2.1.0", &[]);
    let before = list(&[need("json", "2.1")], &r, &policy());
    r.add("json", "2.9.0", &[]);
    assert_eq!(list(&[need("json", "2.1")], &r, &policy()), before);
}

#[test]
fn yanks_advisories_and_avoids_raise_the_selection() {
    let mut r = Registry::default();
    r.add("json", "2.1.0", &[]).yanked = true;
    r.add("json", "2.1.1", &[]).advised = true;
    r.add("json", "2.1.2", &[]);
    r.add("json", "2.1.3", &[("text", "1.0")]);
    r.add("text", "1.0.0", &[]);
    assert_eq!(list(&[need("json", "2.1")], &r, &policy()), ["json 2.1.2"]);
    let mut avoiding = policy();
    let identity = need("json", "2.1").identity();
    avoiding
        .avoid
        .insert(identity, [v("2.1.2")].into_iter().collect());
    // The raise resumes with the needs of the release it takes.
    assert_eq!(
        list(&[need("json", "2.1")], &r, &avoiding),
        ["json 2.1.3", "text 1.0.0"]
    );
}

#[test]
fn a_raise_waits_the_cooldown_unless_now() {
    let mut r = Registry::default();
    let p = policy();
    r.add("json", "2.1.0", &[]).yanked = true;
    r.add("json", "2.1.1", &[]).logged_at = p.now - 3600;
    let held = resolve(&[need("json", "2.1")], &r, &p, Bounds::default()).unwrap_err();
    assert_eq!(
        held,
        Problem::NoRaise {
            identity: need("json", "2.1").identity(),
            from: v("2.1.0"),
            held: Some(v("2.1.1")),
        }
    );
    assert!(held.to_string().contains("`--now`"));
    let now = Policy {
        cooldown: 0,
        ..policy()
    };
    assert_eq!(list(&[need("json", "2.1")], &r, &now), ["json 2.1.1"]);
}

#[test]
fn only_a_threshold_advisory_lifts_the_cooldown() {
    let mut r = Registry::default();
    let p = policy();
    r.add("json", "2.1.0", &[]).advised = true;
    let fix = r.add("json", "2.1.1", &[]);
    fix.logged_at = p.now - 60;
    fix.threshold_fix = true;
    assert_eq!(list(&[need("json", "2.1")], &r, &p), ["json 2.1.1"]);
    // A fix named by one security key only is held like any other release.
    let mut single = Registry::default();
    single.add("json", "2.1.0", &[]).advised = true;
    single.add("json", "2.1.1", &[]).logged_at = p.now - 60;
    assert!(matches!(
        resolve(&[need("json", "2.1")], &single, &p, Bounds::default()),
        Err(Problem::NoRaise { held: Some(_), .. })
    ));
}

#[test]
fn a_raise_skips_pre_releases_unless_one_is_named() {
    let mut r = Registry::default();
    r.add("json", "2.1.0", &[]).yanked = true;
    r.add("json", "2.1.1-rc.1", &[]);
    r.add("json", "2.1.1", &[]);
    assert_eq!(list(&[need("json", "2.1")], &r, &policy()), ["json 2.1.1"]);
    let mut pre = Registry::default();
    pre.add("json", "2.2.0-rc.1", &[]);
    pre.add("json", "2.2.0-rc.2", &[]);
    assert_eq!(
        list(&[need("json", "2.2.0-rc.1")], &pre, &policy()),
        ["json 2.2.0-rc.1"]
    );
    assert!(matches!(
        resolve(&[need("json", "2.2")], &pre, &policy(), Bounds::default()),
        Err(Problem::NoRelease { .. })
    ));
}

#[test]
fn a_need_on_its_own_identity_is_refused() {
    let mut r = Registry::default();
    r.add("loop", "1.0.0", &[("loop", "1.1")]);
    r.add("loop", "1.1.0", &[]);
    assert!(matches!(
        resolve(&[need("loop", "1.0")], &r, &policy(), Bounds::default()),
        Err(Problem::SelfNeed(_))
    ));
}

#[test]
fn every_bound_holds() {
    let mut r = Registry::default();
    for i in 0..10 {
        r.add(&format!("p{i}"), "1.0.0", &[]);
    }
    let names: Vec<String> = (0..10).map(|i| format!("p{i}")).collect();
    r.add(
        "hub",
        "1.0.0",
        &names
            .iter()
            .map(|n| (n.as_str(), "1.0"))
            .collect::<Vec<_>>(),
    );
    let root = [need("hub", "1.0")];
    let tight = |bounds: Bounds| resolve(&root, &r, &policy(), bounds);
    let base = Bounds::default();
    assert!(tight(base).is_ok());
    assert!(matches!(
        tight(Bounds { visited: 5, ..base }),
        Err(Problem::Bound(_))
    ));
    assert!(matches!(
        tight(Bounds { needs: 5, ..base }),
        Err(Problem::Bound(_))
    ));
    assert!(matches!(
        tight(Bounds {
            packages: 5,
            ..base
        }),
        Err(Problem::Bound(_))
    ));
    // The root's needs are bounded as a release's are.
    let wide: Vec<Need> = names.iter().map(|n| need(n, "1.0")).collect();
    assert!(matches!(
        resolve(&wide, &r, &policy(), Bounds { needs: 5, ..base }),
        Err(Problem::Bound(_))
    ));
    let mut many = Registry::default();
    for patch in 0..20 {
        many.add("json", &format!("2.0.{patch}"), &[]);
    }
    assert!(matches!(
        resolve(
            &[need("json", "2.0")],
            &many,
            &policy(),
            Bounds {
                releases: 10,
                ..base
            }
        ),
        Err(Problem::Bound(_))
    ));
}

/// A deterministic generator for the random graphs.
struct XorShift(u64);

impl XorShift {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % n as u64).unwrap()
    }
}

/// Random graphs of n releases, one package per release, five needs each: the work counted is
/// within a constant factor of N + R at every size, and the time per edge stays within a
/// constant factor across sizes (ADR 0122, Costs).
#[test]
fn resolution_is_linear_in_the_graph() {
    let mut per_edge = Vec::new();
    for n in [1_000usize, 10_000, 100_000] {
        let mut rng = XorShift(0x2545_f491_4f6c_dd1d);
        let mut r = Registry::default();
        let names: Vec<String> = (0..n).map(|i| format!("p{i}")).collect();
        for i in 0..n {
            let needs: Vec<(&str, &str)> = (0..5)
                .map(|_| names[rng.below(n)].as_str())
                .filter(|name| *name != names[i])
                .map(|name| (name, "1.0"))
                .collect();
            r.add(&names[i], "1.0.0", &needs);
        }
        let root: Vec<Need> = (0..n).step_by(97).map(|i| need(&names[i], "1.0")).collect();
        let started = Instant::now();
        let unbounded = Bounds {
            visited: usize::MAX,
            packages: usize::MAX,
            needs: usize::MAX,
            ..Bounds::default()
        };
        let resolution = resolve(&root, &r, &policy(), unbounded).unwrap();
        let elapsed = started.elapsed().as_secs_f64();
        let edges = 5 * n;
        assert!(
            resolution.work <= 2 * (n + edges),
            "{n}: {} units of work for {} releases and edges",
            resolution.work,
            n + edges
        );
        per_edge.push(elapsed / (n + edges) as f64);
    }
    // Hash maps and the generous factor absorb a shared machine's noise; quadratic work
    // would grow the time per edge a hundred times from the first size to the last.
    assert!(
        per_edge[2] < per_edge[0] * 10.0,
        "time per edge grew: {per_edge:?}"
    );
}

fn release(version: &str) -> Release {
    Release {
        version: v(version),
        yanked: false,
        advised: false,
        threshold_fix: false,
        logged_at: 0,
    }
}

/// The offline index: local packages resolve as their directories hold them, an avoided local
/// version is raised and, with no later release, refused; a refused source refuses every
/// identity of it, so nothing resolves against a partial view of a registry.
#[test]
fn the_offline_index_resolves_local_packages_and_refuses_unheld_sources() {
    let local = |name: &str, version: &str| Need {
        source: format!("path deps/{name}"),
        name: name.to_owned(),
        version: v(version),
    };
    let mut offline = Offline::default();
    offline.add(local("util", "0.2.0").identity(), release("0.2.0"), vec![]);
    offline.add(local("text", "1.4.0").identity(), release("1.4.0"), vec![]);
    offline.refuse(
        "itaty",
        "the per-name state of `itaty` is not held".to_owned(),
    );
    let root = [local("util", "0.2.0"), local("text", "1.4.0")];
    let resolution = resolve(&root, &offline, &policy(), Bounds::default()).unwrap();
    let listed: Vec<String> = resolution
        .list
        .iter()
        .map(|(identity, version)| format!("{} {version}", identity.name))
        .collect();
    assert_eq!(listed, ["text 1.4.0", "util 0.2.0"]);
    let mut avoiding = policy();
    avoiding.avoid.insert(
        local("util", "0.2.0").identity(),
        [v("0.2.0")].into_iter().collect(),
    );
    assert!(matches!(
        resolve(&root, &offline, &avoiding, Bounds::default()),
        Err(Problem::NoRaise { held: None, .. })
    ));
    let registry = [need("json", "2.1")];
    assert_eq!(
        resolve(&registry, &offline, &policy(), Bounds::default()),
        Err(Problem::Index {
            source: Some("itaty".to_owned()),
            why: "the per-name state of `itaty` is not held".to_owned(),
        })
    );
}
