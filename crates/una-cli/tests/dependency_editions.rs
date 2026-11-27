//! ADR 0122 rule 49 (#2260): each module is checked under the edition of the package that
//! contains it, never under the importing package's, in every command that checks. The cases
//! are the conformance packages of `tests/conformance/project`, a root of edition `0.5` with a
//! path dependency of edition `0.4`: `una check`, `una test`, `una run`, `una report` and
//! `una build` report the same diagnostics for each, those of the dependency's own edition.
//!
//! `una build` of a program that checks needs the native runtime, so the differential test of
//! the native backend (`tests/native.rs`) builds the runnable case instead. `una lsp` checks an
//! opened file as its own package, in the edition of the nearest manifest above it
//! (`spec/30`, #1175), so it never mounts a dependency under another package's edition.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// The conformance case `name` of `tests/conformance/project`.
fn case(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/conformance/project")
        .join(name)
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

/// A fresh copy of the case `name`, since `una run` and `una build` write below the package.
fn fresh(name: &str, command: &str) -> PathBuf {
    let work = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("dependency-editions")
        .join(format!("{name}-{command}"));
    if work.exists() {
        std::fs::remove_dir_all(&work).unwrap();
    }
    copy_dir(&case(name), &work);
    work
}

/// The exit status of `una --json <args>` in the package at `dir`, and the code and
/// `file:line:col` of each diagnostic of its JSON document, in order.
fn diagnostics(dir: &Path, args: &[&str]) -> (Option<i32>, Vec<String>) {
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .arg("--json")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    let document: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "una {args:?}: no JSON document; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    // A document with no diagnostics, such as the report of a program that checks, may omit
    // the list.
    let found = document["diagnostics"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|d| {
            format!(
                "{} at {}:{}:{}",
                d["code"].as_str().unwrap(),
                d["primary"]["file"].as_str().unwrap(),
                d["primary"]["start"]["line"],
                d["primary"]["start"]["col"]
            )
        })
        .collect();
    (output.status.code(), found)
}

#[test]
fn a_dependency_of_edition_0_4_keeps_its_rules_in_every_command() {
    // The dependency declares `to_decimal` on an `Int` and `update` on a `Map`, valid in
    // edition 0.4 and U1002 in edition 0.5, and binds with `let` and `let edit`.
    let name = "ita_path_dep_0_4_in_0_5_own_rules";
    for args in [
        &["check"][..],
        &["test"],
        &["run", "--grant", "console"],
        &["report"],
    ] {
        let work = fresh(name, args[0]);
        let (status, found) = diagnostics(&work, args);
        assert_eq!(found, Vec::<String>::new(), "una {args:?}");
        assert_eq!(status, Some(0), "una {args:?}");
    }
}

#[test]
fn a_dependency_of_edition_0_4_has_no_names_of_edition_0_5_in_any_command() {
    for (name, expected) in [
        (
            "ita_path_dep_0_4_array_in_0_5_error",
            &["U2002 at deps/util/src/text.una:3:13"][..],
        ),
        (
            "ita_path_dep_0_4_f32_in_0_5_error",
            &[
                "U1001 at deps/util/src/text.una:3:13",
                "U1001 at deps/util/src/text.una:3:21",
            ],
        ),
        // Method syntax reaches no function that edition 0.5 adds to the prelude either.
        (
            "ita_path_dep_0_4_parse_decimal_method_error",
            &["U1001 at deps/util/src/text.una:5:27"],
        ),
        (
            "ita_path_dep_0_4_parse_f32_method_error",
            &["U1001 at deps/util/src/text.una:5:27"],
        ),
        (
            "ita_path_dep_0_4_array_method_error",
            &["U1001 at deps/util/src/text.una:5:14"],
        ),
    ] {
        for args in [&["check"][..], &["test"], &["run"], &["report"], &["build"]] {
            let work = fresh(name, args[0]);
            let (status, found) = diagnostics(&work, args);
            assert_eq!(found, expected, "{name}: una {args:?}");
            assert_eq!(status, Some(1), "{name}: una {args:?}");
        }
    }
}
