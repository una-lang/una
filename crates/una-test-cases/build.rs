//! The build script of `una-test-cases` (#1652): it lists the cases of the tests of `una-cli`
//! that run one test per case and writes, into `OUT_DIR/cases.rs`, one `macro_rules!` macro per
//! list, which hands every entry to a macro of the test file. The lists are those of
//! `crates/una-cli/tests/common/case_lists.rs` and of the conformance runner's discovery
//! (`crates/una-cli/tests/conformance/discovery.rs`), included here, so the build and the
//! tests read the corpus with the same code; each test file checks the generated list against
//! the one on disk. An empty list fails the build. Cargo reruns this script when anything under
//! the listed directories changes. This script runs no program and writes nothing but its
//! output file.

// A build script speaks to Cargo on standard output and may stop the build on a broken corpus;
// the library crates' rules (compiler/conventions.md §1) do not apply to it.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::unwrap_used,
    dead_code
)]

#[path = "../una-cli/tests/common/case_lists.rs"]
mod case_lists;
#[path = "../una-cli/tests/conformance/discovery.rs"]
mod discovery;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// A test name for `path`, unique among `taken`: lowercase ASCII letters and digits, `/` as
/// `__` and every other character as `_`, with `_2`, `_3`, ... after a name already taken.
fn test_name(path: &str, taken: &mut BTreeSet<String>) -> String {
    let mut name = String::new();
    for c in path.chars() {
        match c {
            '/' => name.push_str("__"),
            c if c.is_ascii_alphanumeric() => name.push(c.to_ascii_lowercase()),
            _ => name.push('_'),
        }
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert(0, '_');
    }
    let mut unique = name.clone();
    let mut n = 2;
    while !taken.insert(unique.clone()) {
        unique = format!("{name}_{n}");
        n += 1;
    }
    unique
}

/// A Rust string literal of `text`.
fn literal(text: &str) -> String {
    format!("{text:?}")
}

/// Writes `macro_rules! NAME`, documented by `doc`, that calls `$m! { name => "path" [grants], ... }`.
fn write_macro(out: &mut String, name: &str, doc: &str, entries: &[(String, Vec<String>)]) {
    assert!(
        !entries.is_empty(),
        "{name}: no case found on disk; the corpus is missing or empty"
    );
    let mut taken = BTreeSet::new();
    writeln!(out, "/// {doc}").unwrap();
    writeln!(
        out,
        "/// Calls `$m! {{ name => \"path\" [\"grant\", ...], ... }}`, one entry per case, in path order ({} entries).",
        entries.len()
    )
    .unwrap();
    writeln!(out, "#[macro_export]").unwrap();
    writeln!(out, "macro_rules! {name} {{").unwrap();
    writeln!(out, "    ($m:ident) => {{").unwrap();
    writeln!(out, "        $m! {{").unwrap();
    for (path, grants) in entries {
        let grants: Vec<String> = grants.iter().map(|g| literal(g)).collect();
        writeln!(
            out,
            "            {} => {} [{}],",
            test_name(path, &mut taken),
            literal(path),
            grants.join(", ")
        )
        .unwrap();
    }
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }};").unwrap();
    writeln!(out, "}}").unwrap();
}

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("../..").canonicalize().unwrap();
    for dir in ["examples", "tests/conformance", "docs/reviews"] {
        println!("cargo:rerun-if-changed={}", root.join(dir).display());
    }
    println!("cargo:rerun-if-changed=../una-cli/tests/common/case_lists.rs");
    println!("cargo:rerun-if-changed=../una-cli/tests/conformance/discovery.rs");
    println!("cargo:rerun-if-changed=build.rs");

    let mut out = String::new();
    let (cases, _) = discovery::discover(&root.join("tests/conformance"));
    let cases: Vec<(String, Vec<String>)> = cases
        .into_iter()
        .map(|case| (case.path, Vec::new()))
        .collect();
    write_macro(
        &mut out,
        "conformance_cases",
        "The conformance cases of `tests/conformance`, by path relative to it, as the runner discovers them (`compiler/testing.md §2`).",
        &cases,
    );
    let programs: Vec<(String, Vec<String>)> = case_lists::native_programs(&root)
        .into_iter()
        .map(|program| (program.path, program.grants))
        .collect();
    write_macro(
        &mut out,
        "native_programs",
        "The runnable programs of the native differential, by path relative to the repository root, with their grants.",
        &programs,
    );
    let targets: Vec<(String, Vec<String>)> = case_lists::determinism_targets(&root)
        .into_iter()
        .map(|target| (target, Vec::new()))
        .collect();
    write_macro(
        &mut out,
        "determinism_targets",
        "The targets of the determinism test of `una check --json` (`compiler/testing.md §4`), by path relative to the repository root.",
        &targets,
    );
    let corpus: Vec<(String, Vec<String>)> = case_lists::core_corpus(&root)
        .into_iter()
        .map(|case| (case, Vec::new()))
        .collect();
    write_macro(
        &mut out,
        "core_corpus",
        "The packages and single files of the Core IR export test, by path relative to the repository root.",
        &corpus,
    );
    let target = Path::new(&std::env::var_os("OUT_DIR").unwrap()).join("cases.rs");
    std::fs::write(target, out).unwrap();
}
