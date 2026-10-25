//! #1160 (S-31): every write under a package goes through `una_diag::files`, which refuses
//! planted links. This guard reads the non-test sources of the crates that write under a
//! package, and of the helper itself, and fails when a call that creates or writes a file or a
//! directory appears outside the allowlist below, by file and function.
//!
//! The scan is textual and deterministic: comment lines are skipped, and a file is read up to
//! its `#[cfg(test)]` module. The function of a line is the last `fn` declared above it.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The calls that create or write a file or a directory.
const PATTERNS: &[&str] = &[
    "std::fs::write(",
    "fs::write(",
    "File::create(",
    ".create(true)",
    "OpenOptions::new(",
    "DirBuilder::new(",
    "create_dir(",
    "create_dir_all(",
    "hard_link(",
];

/// The places allowed to make those calls, as `file:function`, with why.
const ALLOWED: &[(&str, &str)] = &[
    // The helper itself: the exclusive temporary file of `write_atomically`.
    (
        "crates/una-diag/src/files.rs:fresh_temporary",
        "the helper's exclusive temporary file",
    ),
    // The helper itself: the directories under the root, one checked component at a time.
    (
        "crates/una-diag/src/files.rs:create_directories",
        "the helper's checked directories",
    ),
    // The helper itself: the private directory of `reserve`, created exclusively.
    (
        "crates/una-diag/src/files.rs:fresh_directory",
        "the helper's exclusive private directory",
    ),
    // `una build --out <file>`: a directory the user named on the command line, not one the
    // package chose; the outputs inside it still go through the helper.
    (
        "crates/una-cli/src/build.rs:out_directory",
        "the directory of --out, named by the user",
    ),
    // The proof cache key under `~/.una` (or `UNA_PROOF_KEY`), outside any package: a 0700
    // directory, a 0600 file created exclusively, and a hard link that fails on an existing
    // name.
    (
        "crates/una-cli/src/proof_key.rs:read_or_make",
        "the user's key under ~/.una, outside any package",
    ),
    // A program's own `File.write` under its granted FileSystem capability (ADR 0038 D9 rule
    // 3): created with `create_new`, which never follows a link, and checked by the runtime's
    // path rules, not a write the toolchain makes under the package.
    (
        "crates/una-cli/src/runtime.rs:write_file",
        "a program's write under its FileSystem grant",
    ),
];

/// The directories scanned, relative to the repository root.
const SCANNED: &[&str] = &[
    "crates/una-cli/src",
    "crates/una-verify/src",
    "crates/una-codegen/src",
    "crates/una-diag/src",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The `.rs` files under `dir`, sorted.
fn sources(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            found.extend(sources(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
    found
}

/// The function a line declares, if it declares one.
fn declared(line: &str) -> Option<&str> {
    let mut rest = line.trim_start();
    for prefix in [
        "pub(crate) ",
        "pub(super) ",
        "pub ",
        "const ",
        "async ",
        "unsafe ",
    ] {
        rest = rest.strip_prefix(prefix).unwrap_or(rest);
    }
    let rest = rest.strip_prefix("fn ")?;
    let end = rest.find(['(', '<']).unwrap_or(rest.len());
    Some(rest[..end].trim())
}

/// The `file:function` of each call of [`PATTERNS`] in the non-test code of `file`, with the
/// line.
fn calls(file: &Path, shown: &str) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(file).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let mut function = String::from("<top level>");
    let mut found = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed == "#[cfg(test)]"
            && lines[index + 1..]
                .iter()
                .map(|l| l.trim())
                .find(|l| !l.is_empty())
                .is_some_and(|next| next.starts_with("mod "))
        {
            break;
        }
        if trimmed.starts_with("//") {
            continue;
        }
        if let Some(name) = declared(line) {
            function = name.to_owned();
        }
        if PATTERNS.iter().any(|pattern| line.contains(pattern)) {
            found.push((
                format!("{shown}:{function}"),
                format!("{shown}:{}: {trimmed}", index + 1),
            ));
        }
    }
    found
}

#[test]
fn every_write_under_a_package_goes_through_the_shared_helper() {
    let root = root();
    let allowed: BTreeSet<&str> = ALLOWED.iter().map(|(place, _)| *place).collect();
    let mut used = BTreeSet::new();
    let mut unexpected = Vec::new();
    for dir in SCANNED {
        for file in sources(&root.join(dir)) {
            let shown = file
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            for (place, line) in calls(&file, &shown) {
                if allowed.contains(place.as_str()) {
                    used.insert(place);
                } else {
                    unexpected.push(line);
                }
            }
        }
    }
    assert!(
        unexpected.is_empty(),
        "write through `una_diag::files` (#1160), or justify the place in ALLOWED:\n{}",
        unexpected.join("\n")
    );
    let stale: Vec<&&str> = allowed
        .iter()
        .filter(|place| !used.contains(**place))
        .collect();
    assert!(
        stale.is_empty(),
        "allowed places with no such call: {stale:?}"
    );
}

#[test]
fn the_guard_finds_a_write_and_its_function() {
    let dir = std::env::temp_dir().join(format!("una-safe-writes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("sample.rs");
    std::fs::write(
        &file,
        "pub(crate) fn save(path: &Path) {\n    // std::fs::write(path, b\"comment\")\n    std::fs::write(path, b\"x\");\n}\n\n#[cfg(test)]\nmod tests {\n    fn t() { std::fs::write(p, b\"\"); }\n}\n",
    )
    .unwrap();
    let found = calls(&file, "sample.rs");
    assert_eq!(
        found,
        vec![(
            "sample.rs:save".to_owned(),
            "sample.rs:3: std::fs::write(path, b\"x\");".to_owned()
        )]
    );
    let _ = std::fs::remove_dir_all(&dir);
}
