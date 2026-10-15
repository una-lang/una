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
/// package with no checksum, a package with no source that has a checksum, and any difference
/// between the packages with no source and the workspace's `members`.
fn source_problems(packages: &[Package], members: &BTreeSet<(String, String)>) -> Vec<String> {
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
    for (name, version) in members.difference(&pathed) {
        problems.push(format!(
            "the workspace member {name} {version} is not a package of Cargo.lock"
        ));
    }
    problems
}

/// The record's lines after its header, each as name, version and checksum.
fn record() -> Vec<(String, String, String)> {
    let text = std::fs::read_to_string(root().join("supply-chain/crates.tsv")).unwrap();
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some(HEADER),
        "supply-chain/crates.tsv starts with its header"
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
                "supply-chain/crates.tsv: not a name, a version and a sha256 checksum: {line:?}"
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
    let problems = source_problems(&lock_packages(), &members());
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
        source_problems(&packages(&lock("")), &members),
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
        let problems = source_problems(&packages(&lock(&extra)), &members);
        assert_eq!(problems.len(), 1, "{extra}: {problems:?}");
    }
    // A member that Cargo.lock does not have.
    let more: BTreeSet<_> = [
        ("una-a".to_owned(), "1.0.0".to_owned()),
        ("una-b".to_owned(), "1.0.0".to_owned()),
    ]
    .into();
    assert_eq!(source_problems(&packages(&lock("")), &more).len(), 1);
}

#[test]
fn the_record_lists_exactly_the_crates_io_dependencies() {
    let recorded = record();
    let sorted: Vec<_> = recorded
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    assert_eq!(
        recorded, sorted,
        "supply-chain/crates.tsv is sorted by name and version, with no line twice"
    );
    let recorded: BTreeSet<_> = recorded.into_iter().collect();
    let locked: BTreeSet<_> = lock_packages()
        .into_iter()
        .filter(|package| package.source.as_deref() == Some(CRATES_IO))
        .map(|package| (package.name, package.version, package.checksum.unwrap()))
        .collect();
    let show = |set: Vec<&(String, String, String)>| {
        set.iter()
            .map(|(name, version, checksum)| format!("  {name}\t{version}\t{checksum}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let unreviewed: Vec<_> = locked.difference(&recorded).collect();
    let stale: Vec<_> = recorded.difference(&locked).collect();
    assert!(
        unreviewed.is_empty() && stale.is_empty(),
        "Cargo.lock and supply-chain/crates.tsv differ (docs/release-gates.md, the deps gate: \
         a crate enters the record only once it is reviewed)\n\
         in Cargo.lock, not in the record:\n{}\n\
         in the record, not in Cargo.lock:\n{}",
        show(unreviewed),
        show(stale)
    );
}
