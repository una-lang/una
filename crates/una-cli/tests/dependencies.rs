//! The supply-chain review record (#770, `docs/release-gates.md`, the `deps` gate):
//! `supply-chain/crates.tsv` lists every crate that `Cargo.lock` takes from crates.io, by name,
//! version and checksum, and nothing else. A crate added, moved to another version, or whose
//! checksum changed fails here until the record names it, so a new dependency is reviewed before
//! it merges.
//!
//! A package of `Cargo.lock` with no source is a path dependency, which neither the record nor
//! cargo-deny's sources check sees. So the packages with no source must be exactly the
//! workspace's members, by name and version, and every member's manifest must be inside the
//! repository: a path dependency outside the workspace, or a vendored copy, fails.
//!
//! The fuzz targets (`fuzz/`, #766) are a crate outside the workspace with a `Cargo.lock` of
//! their own. Its crates.io packages are each in the record above or, when `Cargo.lock` does
//! not have them at that version and checksum, in `supply-chain/fuzz-crates.tsv`, which lists
//! exactly those; its packages with no source are the fuzz crate and members of the workspace.
//!
//! The test reads the repository's files and runs `cargo metadata --no-deps --offline --locked`,
//! so it runs offline.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The repository root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// The only registry a dependency may come from.
const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// The record's header line.
const HEADER: &str = "name\tversion\tchecksum";

/// One package of `Cargo.lock`: its name, version, source and checksum, when it has them.
struct Package {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
}

/// The value of a `key = "value"` line, when `line` is one for `key`.
fn value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line
        .strip_prefix(key)?
        .trim_start()
        .strip_prefix('=')?
        .trim();
    rest.strip_prefix('"')?.strip_suffix('"')
}

/// The packages of a `Cargo.lock` text.
fn packages(lock: &str) -> Vec<Package> {
    let mut packages = Vec::new();
    for block in lock.split("[[package]]").skip(1) {
        let mut package = Package {
            name: String::new(),
            version: String::new(),
            source: None,
            checksum: None,
        };
        for line in block.lines() {
            if let Some(name) = value(line, "name") {
                package.name = name.to_owned();
            } else if let Some(version) = value(line, "version") {
                package.version = version.to_owned();
            } else if let Some(source) = value(line, "source") {
                package.source = Some(source.to_owned());
            } else if let Some(checksum) = value(line, "checksum") {
                package.checksum = Some(checksum.to_owned());
            }
        }
        assert!(
            !package.name.is_empty() && !package.version.is_empty(),
            "a package of Cargo.lock has no name or version:{block}"
        );
        packages.push(package);
    }
    packages
}

/// The packages of the repository's `Cargo.lock`.
fn lock_packages() -> Vec<Package> {
    packages(&std::fs::read_to_string(root().join("Cargo.lock")).unwrap())
}

/// The packages of the fuzz targets' `fuzz/Cargo.lock`.
fn fuzz_lock_packages() -> Vec<Package> {
    packages(&std::fs::read_to_string(root().join("fuzz/Cargo.lock")).unwrap())
}

/// The fuzz crate of `fuzz/Cargo.toml`, by name and version.
fn fuzz_crate() -> (String, String) {
    ("una-rt-fuzz".to_owned(), "0.0.0".to_owned())
}

/// The crates.io packages of `packages`, by name, version and checksum.
fn from_crates_io(packages: Vec<Package>) -> BTreeSet<(String, String, String)> {
    packages
        .into_iter()
        .filter(|package| package.source.as_deref() == Some(CRATES_IO))
        .map(|package| (package.name, package.version, package.checksum.unwrap()))
        .collect()
}

/// The workspace's members, by name and version, from `cargo metadata`, each checked to have its
/// manifest inside the repository.
fn members() -> BTreeSet<(String, String)> {
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--no-deps",
            "--offline",
            "--locked",
            "--format-version",
            "1",
        ])
        .current_dir(root())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cargo metadata: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let root = root();
    let mut members = BTreeSet::new();
    for package in metadata["packages"].as_array().unwrap() {
        let name = package["name"].as_str().unwrap().to_owned();
        let version = package["version"].as_str().unwrap().to_owned();
        let manifest = Path::new(package["manifest_path"].as_str().unwrap())
            .canonicalize()
            .unwrap();
        assert!(
            manifest.starts_with(&root),
            "the workspace member {name} has its manifest outside the repository: {}",
            manifest.display()
        );
        members.insert((name, version));
    }
    assert!(!members.is_empty(), "cargo metadata lists no member");
    members
}

/// The problems with the sources of `packages`: a source other than crates.io, a crates.io
/// package with no checksum, a package with no source that has a checksum, a package with no
/// source that is not one of `members`, and, when `every_member` is set, a member that is not a
/// package with no source.
fn source_problems(
    packages: &[Package],
    members: &BTreeSet<(String, String)>,
    every_member: bool,
) -> Vec<String> {
    let mut problems = Vec::new();
    let mut pathed = BTreeSet::new();
    for package in packages {
        let id = format!("{} {}", package.name, package.version);
        match &package.source {
            None => {
                if package.checksum.is_some() {
                    problems.push(format!("{id} has a checksum but no source"));
                }
                pathed.insert((package.name.clone(), package.version.clone()));
            }
            Some(source) if source == CRATES_IO => {
                if package.checksum.is_none() {
                    problems.push(format!("{id} comes from crates.io with no checksum"));
                }
            }
            Some(source) => problems.push(format!("{id} comes from {source}")),
        }
    }
    for (name, version) in pathed.difference(members) {
        problems.push(format!(
            "{name} {version} has no source and is not a member of the workspace (a path dependency outside it, or a vendored copy)"
        ));
    }
    if every_member {
        for (name, version) in members.difference(&pathed) {
            problems.push(format!(
                "the workspace member {name} {version} is not a package of Cargo.lock"
            ));
        }
    }
    problems
}

/// The lines after the header of the record `name` of `supply-chain/`, each as name, version
/// and checksum.
fn record(name: &str) -> Vec<(String, String, String)> {
    let text = std::fs::read_to_string(root().join("supply-chain").join(name)).unwrap();
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some(HEADER),
        "supply-chain/{name} starts with its header"
    );
    lines
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            assert!(
                fields.len() == 3
                    && fields[2].len() == 64
                    && fields[2]
                        .chars()
                        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
                "supply-chain/{name}: not a name, a version and a sha256 checksum: {line:?}"
            );
            (
                fields[0].to_owned(),
                fields[1].to_owned(),
                fields[2].to_owned(),
            )
        })
        .collect()
}

#[test]
fn every_dependency_comes_from_crates_io_or_is_a_workspace_member() {
    let problems = source_problems(&lock_packages(), &members(), true);
    assert!(
        problems.is_empty(),
        "the sources of Cargo.lock:\n{}",
        problems.join("\n")
    );
}

#[test]
fn a_path_dependency_outside_the_workspace_is_a_problem() {
    let members: BTreeSet<_> = [("una-a".to_owned(), "1.0.0".to_owned())].into();
    let checksum = "a".repeat(64);
    let lock = |extra: &str| {
        format!(
            "version = 4\n\n[[package]]\nname = \"una-a\"\nversion = \"1.0.0\"\ndependencies = [\n \"dep\",\n]\n\n[[package]]\nname = \"dep\"\nversion = \"0.1.0\"\nsource = \"{CRATES_IO}\"\nchecksum = \"{checksum}\"\n{extra}"
        )
    };
    // The workspace and a crates.io package: no problem.
    assert_eq!(
        source_problems(&packages(&lock("")), &members, true),
        Vec::<String>::new()
    );
    // Each of these is one problem.
    for extra in [
        // A path dependency outside the workspace, or a vendored copy: no source, no checksum.
        "\n[[package]]\nname = \"outside\"\nversion = \"0.1.0\"\n".to_owned(),
        // A vendored copy of a member's name at another version.
        "\n[[package]]\nname = \"una-a\"\nversion = \"0.9.0\"\n".to_owned(),
        // A git dependency.
        "\n[[package]]\nname = \"g\"\nversion = \"0.1.0\"\nsource = \"git+https://example.org/g#abc\"\n"
            .to_owned(),
        // A crates.io package with no checksum.
        format!("\n[[package]]\nname = \"n\"\nversion = \"0.1.0\"\nsource = \"{CRATES_IO}\"\n"),
    ] {
        let problems = source_problems(&packages(&lock(&extra)), &members, true);
        assert_eq!(problems.len(), 1, "{extra}: {problems:?}");
    }
    // A member that Cargo.lock does not have.
    let more: BTreeSet<_> = [
        ("una-a".to_owned(), "1.0.0".to_owned()),
        ("una-b".to_owned(), "1.0.0".to_owned()),
    ]
    .into();
    assert_eq!(source_problems(&packages(&lock("")), &more, true).len(), 1);
    // Unless every member need not be one, as for the fuzz crate's lock.
    assert_eq!(
        source_problems(&packages(&lock("")), &more, false),
        Vec::<String>::new()
    );
}

#[test]
fn the_record_lists_exactly_the_crates_io_dependencies() {
    check_record("crates.tsv", &from_crates_io(lock_packages()), "Cargo.lock");
}

#[test]
fn the_fuzz_crate_takes_its_path_dependencies_from_the_workspace() {
    let mut allowed = members();
    allowed.insert(fuzz_crate());
    let packages = fuzz_lock_packages();
    let problems = source_problems(&packages, &allowed, false);
    assert!(
        problems.is_empty(),
        "the sources of fuzz/Cargo.lock:\n{}",
        problems.join("\n")
    );
    let (name, version) = fuzz_crate();
    assert!(
        packages
            .iter()
            .any(|package| package.name == name && package.version == version),
        "fuzz/Cargo.lock has no package {name} {version}"
    );
}

#[test]
fn the_fuzz_crate_fuzzes_the_workspace_crates_at_their_place() {
    // The fuzz lock names its path packages by name and version only, which a path elsewhere
    // with the same name and version would also satisfy. So `cargo metadata` says where they
    // are: the fuzz crate's own manifest is `fuzz/Cargo.toml`, and each of its path
    // dependencies is the manifest of a workspace member under `crates/`.
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--manifest-path",
            "fuzz/Cargo.toml",
            "--no-deps",
            "--offline",
            "--locked",
            "--format-version",
            "1",
        ])
        .current_dir(root())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cargo metadata of fuzz/Cargo.toml: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let root = root();
    let packages = metadata["packages"].as_array().unwrap();
    assert_eq!(packages.len(), 1, "fuzz/Cargo.toml is one package");
    let package = &packages[0];
    let (name, version) = fuzz_crate();
    assert_eq!(package["name"].as_str(), Some(name.as_str()));
    assert_eq!(package["version"].as_str(), Some(version.as_str()));
    assert_eq!(
        Path::new(package["manifest_path"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        root.join("fuzz/Cargo.toml").canonicalize().unwrap()
    );
    let members: BTreeSet<String> = members().into_iter().map(|(name, _)| name).collect();
    let mut paths = 0;
    for dependency in package["dependencies"].as_array().unwrap() {
        let Some(path) = dependency["path"].as_str() else {
            continue;
        };
        paths += 1;
        let name = dependency["name"].as_str().unwrap();
        let place = Path::new(path).canonicalize().unwrap();
        assert!(
            members.contains(name)
                && place == root.join("crates").join(name).canonicalize().unwrap(),
            "the fuzz crate's path dependency {name} is at {}, not the workspace's crates/{name}",
            place.display()
        );
    }
    assert!(
        paths >= 1,
        "the fuzz crate has no path dependency on the workspace"
    );
}

#[test]
fn the_fuzz_record_lists_exactly_the_fuzz_crates_not_in_the_record() {
    let fuzz_only = fuzz_only(lock_packages(), fuzz_lock_packages());
    check_record(
        "fuzz-crates.tsv",
        &fuzz_only,
        "fuzz/Cargo.lock (less the packages Cargo.lock has)",
    );
}

/// A crate, by name, version and checksum.
type Crate = (String, String, String);

/// The crates.io packages of the fuzz lock `fuzz` that the main lock `main` does not have at the
/// same version and checksum: what `supply-chain/fuzz-crates.tsv` must list.
fn fuzz_only(main: Vec<Package>, fuzz: Vec<Package>) -> BTreeSet<Crate> {
    let main = from_crates_io(main);
    from_crates_io(fuzz).difference(&main).cloned().collect()
}

/// The crates of `locked` that `recorded` does not list (unreviewed), and the lines of
/// `recorded` that `locked` does not have (stale).
fn differences<'a>(
    recorded: &'a BTreeSet<Crate>,
    locked: &'a BTreeSet<Crate>,
) -> (Vec<&'a Crate>, Vec<&'a Crate>) {
    (
        locked.difference(recorded).collect(),
        recorded.difference(locked).collect(),
    )
}

#[test]
fn each_fuzz_crate_is_in_exactly_one_record() {
    let checksum = |c: char| c.to_string().repeat(64);
    let lock = |crates: &[(&str, &str, char)]| {
        let mut text = "version = 4\n".to_owned();
        for (name, version, c) in crates {
            text.push_str(&format!(
                "\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\nsource = \"{CRATES_IO}\"\nchecksum = \"{}\"\n",
                checksum(*c)
            ));
        }
        packages(&text)
    };
    let one =
        |name: &str, version: &str, c: char| (name.to_owned(), version.to_owned(), checksum(c));
    let fuzz = || lock(&[("a", "1.0.0", 'a'), ("b", "1.0.0", 'b')]);
    let record: BTreeSet<_> = [one("b", "1.0.0", 'b')].into();
    // `a` is in the main lock, at the same version and checksum: the fuzz record lists `b`.
    let only = fuzz_only(lock(&[("a", "1.0.0", 'a')]), fuzz());
    assert_eq!(differences(&record, &only), (vec![], vec![]));
    // The main lock drops `a`, or moves it to another version: `a` is unreviewed until the fuzz
    // record lists it.
    for main in [lock(&[]), lock(&[("a", "2.0.0", 'c')])] {
        let only = fuzz_only(main, fuzz());
        assert_eq!(
            differences(&record, &only),
            (vec![&one("a", "1.0.0", 'a')], vec![])
        );
    }
    // A line of the fuzz record that the main lock now covers is stale: each crate is in one
    // record only.
    let both: BTreeSet<_> = [one("a", "1.0.0", 'a'), one("b", "1.0.0", 'b')].into();
    let only = fuzz_only(lock(&[("a", "1.0.0", 'a')]), fuzz());
    assert_eq!(
        differences(&both, &only),
        (vec![], vec![&one("a", "1.0.0", 'a')])
    );
}

/// Checks that the record `name` of `supply-chain/` is sorted, with no line twice, and lists
/// exactly `locked`, the crates.io packages of the lock that `what` names.
fn check_record(name: &str, locked: &BTreeSet<Crate>, what: &str) {
    let recorded = record(name);
    let sorted: Vec<_> = recorded
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    assert_eq!(
        recorded, sorted,
        "supply-chain/{name} is sorted by name and version, with no line twice"
    );
    let recorded: BTreeSet<_> = recorded.into_iter().collect();
    let show = |set: Vec<&(String, String, String)>| {
        set.iter()
            .map(|(name, version, checksum)| format!("  {name}\t{version}\t{checksum}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let (unreviewed, stale) = differences(&recorded, locked);
    assert!(
        unreviewed.is_empty() && stale.is_empty(),
        "{what} and supply-chain/{name} differ (docs/release-gates.md, the deps gate: \
         a crate enters the record only once it is reviewed)\n\
         in {what}, not in the record:\n{}\n\
         in the record, not in {what}:\n{}",
        show(unreviewed),
        show(stale)
    );
}

#[test]
fn ncsa_is_allowed_for_libfuzzer_alone() {
    // The license of the fuzz targets' vendored libFuzzer (#766): cargo-deny admits NCSA only
    // through the one exception for libfuzzer-sys, at its version, and never in the allowlist,
    // so any other crate that carries it fails the deps gate.
    let deny = std::fs::read_to_string(root().join("deny.toml")).unwrap();
    let lines: Vec<&str> = deny
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect();
    let mentions: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("NCSA"))
        .map(|(at, _)| at)
        .collect();
    assert_eq!(mentions.len(), 1, "deny.toml names NCSA once: {mentions:?}");
    let at = mentions[0];
    assert_eq!(lines[at].trim(), "allow = [\"NCSA\"]");
    assert_eq!(lines[at - 1].trim(), "crate = \"libfuzzer-sys@0.4.13\"");
    assert_eq!(lines[at - 2].trim(), "[[licenses.exceptions]]");
}
