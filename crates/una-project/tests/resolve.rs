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
    // A raise past a yank or an advisory points to the audit and to the need, never to a flag
    // that skips the wait.
    let message = held.to_string();
    assert!(
        !message.contains("--now") && message.contains("`una ita audit`"),
        "{message}"
    );
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
/// within a constant factor of N + R at every size, and the time, the least of three runs, at
/// most 3 times as long for each doubling of the graph, with a small allowance for a shared
/// machine's noise: quadratic work would take four times as long (ADR 0122, Costs).
#[test]
fn resolution_is_linear_in_the_graph() {
    let sizes = [5_000usize, 10_000, 20_000, 40_000];
    let mut times = Vec::new();
    for n in sizes {
        let mut rng = XorShift(0x2545_f491_4f6c_dd1d);
        let mut r = Registry::default();
        let names: Vec<String> = (0..n).map(|i| format!("p{i}")).collect();
        for name in &names {
            let needs: Vec<(&str, &str)> = (0..5)
                .map(|_| names[rng.below(n)].as_str())
                .filter(|other| *other != name.as_str())
                .map(|other| (other, "1.0"))
                .collect();
            r.add(name, "1.0.0", &needs);
        }
        let root: Vec<Need> = (0..n).step_by(97).map(|i| need(&names[i], "1.0")).collect();
        let unbounded = Bounds {
            visited: usize::MAX,
            packages: usize::MAX,
            needs: usize::MAX,
            total: usize::MAX,
            ..Bounds::default()
        };
        let mut best = f64::MAX;
        for _ in 0..3 {
            let started = Instant::now();
            let resolution = resolve(&root, &r, &policy(), unbounded).unwrap();
            best = best.min(started.elapsed().as_secs_f64());
            let edges = 5 * n;
            assert!(
                resolution.work <= 2 * (n + edges),
                "{n}: {} units of work for {} releases and edges",
                resolution.work,
                n + edges
            );
        }
        times.push(best);
    }
    for (pair, sizes_pair) in times.windows(2).zip(sizes.windows(2)) {
        assert!(
            pair[1] <= 3.0 * pair[0] + 0.05,
            "{} took {}s, {} took {}s",
            sizes_pair[1],
            pair[1],
            sizes_pair[0],
            pair[0]
        );
    }
}

/// Many releases per name, nearly all pre-releases that no need names, with a raise past a
/// yanked release for every name and lookups that land before a run of pre-releases: the work
/// does not grow with the pre-releases, so a scan of them per visit or per raise, which would
/// make it grow with visits times pre-releases, fails here.
#[test]
fn pre_releases_and_raises_cost_nothing_per_skipped_release() {
    let n = 200;
    let work_with = |pre: usize| {
        let mut r = Registry::default();
        let names: Vec<String> = (0..n).map(|i| format!("p{i}")).collect();
        let versions: Vec<String> = (1..=5).map(|patch| format!("1.0.{patch}")).collect();
        for (i, name) in names.iter().enumerate() {
            let next = names[(i + 1) % n].as_str();
            let at = versions[i % versions.len()].as_str();
            r.add(name, "1.0.0", &[(next, at)]).yanked = true;
            for k in 0..pre {
                r.add(name, &format!("1.1.0-p.{k}"), &[]);
            }
            r.add(name, "1.1.0", &[(next, at)]);
        }
        let root: Vec<Need> = names.iter().map(|name| need(name, "1.0")).collect();
        let resolution = resolve(&root, &r, &policy(), Bounds::default()).unwrap();
        assert!(
            resolution
                .list
                .values()
                .all(|version| *version == v("1.1.0")),
            "{:?}",
            resolution.list
        );
        resolution.work
    };
    let few = work_with(4);
    let many = work_with(2_000);
    assert_eq!(few, many, "the work grew with the pre-releases skipped");
    assert!(few <= 20 * n, "{few} units of work for {n} names");
}

/// The case of rule 22 whose answer depended on the order of the root's needs: `x 1.0.0` needs
/// `a 1.1.0-beta`, so a pre-release of `a` is admitted, and `a 1.0` reaches `1.1.0-beta`
/// whichever need is walked first.
#[test]
fn pre_release_admission_does_not_depend_on_the_order_of_needs() {
    let mut r = Registry::default();
    r.add("x", "1.0.0", &[("a", "1.1.0-beta")]);
    r.add("a", "1.1.0-beta", &[]);
    r.add("a", "1.2.0", &[]);
    let expected = ["a 1.1.0-beta", "x 1.0.0"];
    assert_eq!(
        list(&[need("x", "1.0"), need("a", "1.0")], &r, &policy()),
        expected
    );
    assert_eq!(
        list(&[need("a", "1.0"), need("x", "1.0")], &r, &policy()),
        expected
    );
}

/// A transitive need that names a pre-release reaches it when only pre-releases meet it:
/// `x 1.0.0` needs `b 2.0.0-rc.1`, b's only release.
#[test]
fn a_transitive_pre_release_need_resolves() {
    let mut r = Registry::default();
    r.add("x", "1.0.0", &[("b", "2.0.0-rc.1")]);
    r.add("b", "2.0.0-rc.1", &[]);
    assert_eq!(
        list(&[need("x", "1.0")], &r, &policy()),
        ["b 2.0.0-rc.1", "x 1.0.0"]
    );
}

/// The bound of releases visited counts each distinct release once, so the pass that rule 22
/// runs again costs it nothing.
#[test]
fn a_pass_run_again_does_not_eat_the_bound() {
    let mut r = Registry::default();
    r.add("x", "1.0.0", &[("a", "1.1.0-beta")]);
    r.add("a", "1.1.0-beta", &[]);
    r.add("a", "1.2.0", &[]);
    let root = [need("a", "1.0"), need("x", "1.0")];
    let bounds = |visited| Bounds {
        visited,
        ..Bounds::default()
    };
    // Three distinct releases are visited, a 1.0, x 1.0 and a 1.1.0-beta, in each of the two
    // passes: three is enough, two is not.
    assert!(resolve(&root, &r, &policy(), bounds(3)).is_ok());
    assert!(matches!(
        resolve(&root, &r, &policy(), bounds(2)),
        Err(Problem::Bound(_))
    ));
}

/// A chain of pre-releases, each reached only once its identity admits pre-releases and naming
/// the next identity's, forces a pass per link: the visits of every pass together are capped,
/// and the chain is refused at the cap rather than walked quadratically, while it resolves
/// under the default bounds and a two-pass graph stays well under a small cap.
#[test]
fn passes_forced_by_a_chain_of_pre_releases_are_capped() {
    let links = 50;
    let mut r = Registry::default();
    let names: Vec<String> = (0..links).map(|i| format!("c{i}")).collect();
    for (i, name) in names.iter().enumerate() {
        r.add(name, "1.0.5-rc.1", &[]);
        let next: Vec<(&str, &str)> = names
            .get(i + 1)
            .map(|next| vec![(next.as_str(), "1.0.5-rc.1")])
            .unwrap_or_default();
        r.add(name, "1.1.0-rc.1", &next);
        r.add(name, "1.2.0", &[]);
    }
    // `c0 1.0.5-rc.1` admits c0, whose `1.0.6` need then reaches its `1.1.0-rc.1`, which names
    // c1, admitted in the next pass, and so on.
    let mut root = vec![need("c0", "1.0.5-rc.1")];
    root.extend(names.iter().map(|name| need(name, "1.0.6")));
    let resolution = resolve(&root, &r, &policy(), Bounds::default()).unwrap();
    assert!(
        resolution
            .list
            .values()
            .all(|version| *version == v("1.1.0-rc.1")),
        "{:?}",
        resolution.list
    );
    let capped = Bounds {
        total: 1_000,
        ..Bounds::default()
    };
    let refused = resolve(&root, &r, &policy(), capped);
    assert!(
        matches!(
            &refused,
            Err(Problem::Bound(message)) if message.contains("more than 1000 visits")
        ),
        "{refused:?}"
    );
    // Two passes over three releases: six visits, under a cap of twelve.
    let mut two = Registry::default();
    two.add("x", "1.0.0", &[("a", "1.1.0-beta")]);
    two.add("a", "1.1.0-beta", &[]);
    two.add("a", "1.2.0", &[]);
    let small = Bounds {
        visited: 3,
        total: 12,
        ..Bounds::default()
    };
    let root = [need("a", "1.0"), need("x", "1.0")];
    assert!(resolve(&root, &two, &policy(), small).is_ok());
}

/// A pass that fails reports its least failure, by identity and version, whatever the order of
/// the needs.
#[test]
fn the_failure_reported_does_not_depend_on_the_order_of_needs() {
    let r = Registry::default();
    let expected = Problem::NoRelease {
        identity: need("p", "1.0").identity(),
        version: v("1.0"),
    };
    for root in [
        [need("p", "1.0"), need("q", "1.0")],
        [need("q", "1.0"), need("p", "1.0")],
    ] {
        assert_eq!(
            resolve(&root, &r, &policy(), Bounds::default()),
            Err(expected.clone())
        );
    }
}

/// A release of [`a_shuffled_graph_resolves_the_same`]: its version, whether it is yanked, and
/// its needs, each the index of a name and a version.
type Shuffled = (&'static str, bool, Vec<(usize, &'static str)>);

/// A fixed random graph in two layers, resolved with its root's needs, each release's needs and
/// the order of each name's releases shuffled by a fixed seed: every order resolves, to the
/// same build list (rule 23). The upper layer has no pre-releases, yanked releases but its
/// last, and needs on both layers, those on the lower layer naming pre-releases too; the lower
/// layer has pre-releases, yanked releases but its last, no needs, and one name whose only
/// releases are pre-releases, needed only by pre-release versions. So every need is met, every
/// raise has somewhere to go, and the needs a pass reaches never depend on which pre-releases
/// it admits.
#[test]
fn a_shuffled_graph_resolves_the_same() {
    let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
    let upper = 20;
    let lower = 20;
    // The names: the upper layer, the lower layer, then the pre-release-only one.
    let names: Vec<String> = (0..upper)
        .map(|i| format!("u{i}"))
        .chain((0..lower).map(|i| format!("l{i}")))
        .chain(["pre".to_owned()])
        .collect();
    let only_pre = upper + lower;
    let finals = ["1.0.0", "1.1.0", "1.2.0", "1.3.0"];
    let mixed = [
        "1.0.0",
        "1.1.0-beta.1",
        "1.1.0",
        "1.2.0-rc.1",
        "1.2.0-rc.2",
        "1.2.0",
        "1.3.0",
    ];
    let pre_only = ["1.1.0-beta.1", "1.2.0-rc.1"];
    let mut graph: Vec<Vec<Shuffled>> = Vec::new();
    for i in 0..names.len() {
        let mut releases = Vec::new();
        if i < upper {
            for (at, version) in finals.iter().enumerate() {
                let yanked = at + 1 < finals.len() && rng.below(5) == 0;
                let mut needs: Vec<(usize, &'static str)> = Vec::new();
                for _ in 0..2 {
                    let to = rng.below(upper);
                    if to != i {
                        needs.push((to, finals[rng.below(finals.len())]));
                    }
                }
                needs.push((upper + rng.below(lower), mixed[rng.below(mixed.len())]));
                if rng.below(3) == 0 {
                    needs.push((only_pre, pre_only[rng.below(pre_only.len())]));
                }
                releases.push((*version, yanked, needs));
            }
        } else if i < only_pre {
            for (at, version) in mixed.iter().enumerate() {
                let yanked = at + 1 < mixed.len() && rng.below(5) == 0;
                releases.push((*version, yanked, Vec::new()));
            }
        } else {
            for version in pre_only {
                releases.push((version, false, Vec::new()));
            }
        }
        graph.push(releases);
    }
    let root: Vec<Need> = (0..upper)
        .step_by(3)
        .map(|i| need(&names[i], finals[rng.below(finals.len())]))
        .collect();
    let mut shuffle = |len: usize| -> Vec<usize> {
        let mut order: Vec<usize> = (0..len).collect();
        for at in (1..len).rev() {
            order.swap(at, rng.below(at + 1));
        }
        order
    };
    let mut results = Vec::new();
    for round in 0..12 {
        let mut r = Registry::default();
        for i in shuffle(names.len()) {
            for at in shuffle(graph[i].len()) {
                let (version, yanked, needs) = &graph[i][at];
                let mut order: Vec<(&str, &str)> = needs
                    .iter()
                    .map(|(to, version)| (names[*to].as_str(), *version))
                    .collect();
                if round % 2 == 1 {
                    order.reverse();
                }
                r.add(&names[i], version, &order).yanked = *yanked;
            }
        }
        let order = shuffle(root.len());
        let shuffled: Vec<Need> = order.iter().map(|at| root[*at].clone()).collect();
        let result = resolve(&shuffled, &r, &policy(), Bounds::default()).map(|r| r.list);
        assert!(result.is_ok(), "round {round}: {result:?}");
        results.push(result);
    }
    for result in &results[1..] {
        assert_eq!(result, &results[0]);
    }
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
