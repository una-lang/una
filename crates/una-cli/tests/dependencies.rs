//! The supply-chain review record (#770, `docs/release-gates.md`, the `deps` gate):
//! `supply-chain/crates.tsv` lists every crate that `Cargo.lock` takes from crates.io, by name,
//! version and checksum, and nothing else. A crate added, moved to another version, or whose
//! checksum changed fails here until the record names it, so a new dependency is reviewed before
//! it merges. The test reads only files of the repository, so it runs offline.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The repository root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
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

/// The packages of `Cargo.lock`.
fn lock_packages() -> Vec<Package> {
    let lock = std::fs::read_to_string(root().join("Cargo.lock")).unwrap();
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
fn every_dependency_comes_from_crates_io_or_the_workspace() {
    let mut others = Vec::new();
    for package in lock_packages() {
        match &package.source {
            // A package with no source is a crate of the workspace.
            None => assert!(
                package.checksum.is_none(),
                "{} {} has a checksum but no source",
                package.name,
                package.version
            ),
            Some(source) if source == CRATES_IO => assert!(
                package.checksum.is_some(),
                "{} {} comes from crates.io with no checksum",
                package.name,
                package.version
            ),
            Some(source) => others.push(format!(
                "{} {} from {source}",
                package.name, package.version
            )),
        }
    }
    assert!(
        others.is_empty(),
        "dependencies from a source other than crates.io:\n{}",
        others.join("\n")
    );
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
