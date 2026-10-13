//! Editions are cumulative (`spec/13 §7`): every edition `0.2` package of the conformance suite,
//! checked as edition `0.3`, reports byte-identical `una --json check` output and exit status.
//! The rules that target edition `0.3` will change some of them; each such package is listed in
//! `EXPECTED_DIFFERENCES`, and a listed package that does not differ fails the test, so the list
//! stays exact.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The edition `0.2` packages, relative to `tests/conformance`, whose output in edition `0.3`
/// differs by design, each because of a rule that targets edition `0.3`.
const EXPECTED_DIFFERENCES: &[&str] = &[];

const EDITION_0_2: &str = "edition = \"0.2\"";
const EDITION_0_3: &str = "edition = \"0.3\"";

fn conformance() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance")
}

/// Every package directory under `dir`, in a stable order.
fn packages(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    entries.sort();
    for entry in entries {
        if entry.join("una.toml").is_file() {
            out.push(entry);
        } else {
            packages(&entry, out);
        }
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

/// The exit status and output of `una --json check .` on a fresh copy of `package` at `work`,
/// with its manifest's edition line replaced by `edition`.
fn check_as(package: &Path, work: &Path, edition: &str) -> (Option<i32>, Vec<u8>, Vec<u8>) {
    if work.exists() {
        std::fs::remove_dir_all(work).unwrap();
    }
    copy_dir(package, work);
    let manifest = work.join("una.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, text.replace(EDITION_0_2, edition)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .args(["--json", "check", "."])
        .current_dir(work)
        .output()
        .unwrap();
    std::fs::remove_dir_all(work).unwrap();
    (output.status.code(), output.stdout, output.stderr)
}

#[test]
fn every_edition_0_2_package_checks_the_same_in_edition_0_3() {
    let root = conformance();
    let mut all = Vec::new();
    packages(&root, &mut all);
    let cases: Vec<(String, PathBuf)> = all
        .into_iter()
        .filter(|package| {
            std::fs::read_to_string(package.join("una.toml"))
                .unwrap()
                .lines()
                .any(|line| line.trim() == EDITION_0_2)
        })
        .map(|package| {
            let name = package
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (name, package)
        })
        .collect();
    assert!(
        cases.len() > 300,
        "only {} edition 0.2 packages",
        cases.len()
    );
    for expected in EXPECTED_DIFFERENCES {
        assert!(
            cases.iter().any(|(name, _)| name == expected),
            "{expected} is not an edition 0.2 package"
        );
    }

    // Linear in the packages: each one is checked twice, on its own copy.
    let workers = 4;
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("editions");
    let results: Vec<(String, bool)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                let cases = &cases;
                let work = scratch.join(format!("w{worker}"));
                scope.spawn(move || {
                    cases
                        .iter()
                        .skip(worker)
                        .step_by(workers)
                        .map(|(name, package)| {
                            let before = check_as(package, &work, EDITION_0_2);
                            let after = check_as(package, &work, EDITION_0_3);
                            (name.clone(), before == after)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    });

    let mut wrong = Vec::new();
    for (name, same) in &results {
        let expected = EXPECTED_DIFFERENCES.contains(&name.as_str());
        if *same == expected {
            wrong.push(if expected {
                format!("{name}: listed as differing, but checks the same in edition 0.3")
            } else {
                format!("{name}: checks differently in edition 0.3")
            });
        }
    }
    wrong.sort();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
