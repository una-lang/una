//! Mechanical repairs applied together (#257, the v0.2.0 pre-tag review, area 3b): the edits
//! of every diagnostic of one `una --json check` never overlap, applying them all at once
//! leaves a program that checks, and the repaired program means what the source meant.
//!
//! An agent applies the repairs of a check together, so two repairs of one check that edit the
//! same text are a defect even when each one alone is right.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pretty_assertions::assert_eq;
use serde_json::Value;

fn una(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

/// A package of `edition` whose entry is the module `m` in `source`, with `console` granted.
fn package(name: &str, edition: &str, source: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("repairs")
        .join(format!("{name}-{edition}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        format!(
            "[package]\nname = \"c\"\nversion = \"0.1.0\"\nedition = \"{edition}\"\n\n[run]\nentry = \"m\"\ngrants = [\"console\"]\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(root.join("src/m.una"), source).unwrap();
    root
}

/// The diagnostics of `una --json check`.
fn diagnostics(root: &Path) -> Vec<Value> {
    let output = una(root, &["--json", "check", "."]);
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    document["diagnostics"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The character offset of a 1-based line and column of `text`.
fn offset(text: &str, position: &Value) -> usize {
    let line = usize::try_from(position["line"].as_u64().unwrap()).unwrap();
    let col = usize::try_from(position["col"].as_u64().unwrap()).unwrap();
    text.split('\n')
        .take(line - 1)
        .map(|l| l.chars().count() + 1)
        .sum::<usize>()
        + col
        - 1
}

/// Applies the first repair's edit of every diagnostic of one check together, after checking
/// that no two of them touch the same text, and returns the codes of the check that follows.
fn apply_all(root: &Path) -> Vec<String> {
    let path = root.join("src/m.una");
    let text = std::fs::read_to_string(&path).unwrap();
    let mut edits: Vec<(usize, usize, String)> = diagnostics(root)
        .iter()
        .filter_map(|d| d["repairs"].as_array()?.first()?.get("edit").cloned())
        .filter(|edit| !edit.is_null())
        .map(|edit| {
            (
                offset(&text, &edit["start"]),
                offset(&text, &edit["end"]),
                edit["replacement"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert!(!edits.is_empty(), "no edit to apply");
    edits.sort();
    for pair in edits.windows(2) {
        assert!(
            pair[0].1 <= pair[1].0 && pair[0].0 != pair[1].0,
            "two repairs of one check edit the same text: {pair:?}"
        );
    }
    let mut chars: Vec<char> = text.chars().collect();
    for (start, end, replacement) in edits.iter().rev() {
        chars.splice(start..end, replacement.chars());
    }
    std::fs::write(&path, chars.into_iter().collect::<String>()).unwrap();
    diagnostics(root)
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_owned())
        .collect()
}

/// What `una run` prints.
fn run(root: &Path) -> String {
    let output = una(root, &["run", "."]);
    String::from_utf8(output.stdout).unwrap()
}

fn repaired(root: &Path) -> String {
    std::fs::read_to_string(root.join("src/m.una")).unwrap()
}

#[test]
fn a_variant_compared_in_a_clause_is_repaired_once_f1() {
    // U2014 (outside the refinement language) and U2009 (the variant needs its fields) are
    // both reported at `p != Over`; only one of them carries the edit to `not (p is Over)`.
    for edition in ["0.1", "0.2"] {
        let root = package(
            "f1",
            edition,
            "module m\n\npub kind P = Over(index: Int) | Fine\n\npub f(p: P) -> Int\n  requires p != Over\n=\n  1\n",
        );
        assert_eq!(
            apply_all(&root),
            Vec::<String>::new(),
            "{}",
            repaired(&root)
        );
        assert!(repaired(&root).contains("requires not (p is Over)\n"));
    }
}

#[test]
fn two_result_bindings_of_one_scope_get_distinct_names_f2() {
    // Each `let result` gets its own fresh name, and each use the name of the binding it
    // reads: `let result = result + 3` reads the first one.
    for edition in ["0.1", "0.2"] {
        let root = package(
            "f2",
            edition,
            "module m\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let result = 1\n  let result = result + 3\n  @console.print(\"{result}\")\n",
        );
        assert_eq!(
            apply_all(&root),
            Vec::<String>::new(),
            "{}",
            repaired(&root)
        );
        assert_eq!(run(&root), "4\n");
    }
}

#[test]
fn a_result_binding_of_a_task_is_renamed_after_its_block_f3() {
    // The bindings of a `together` block's tasks are in scope after the block (`spec/19`).
    let root = package(
        "f3",
        "0.2",
        "module m\n\nthree() -> Int = 3\n\nfour() -> Int = 4\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  together\n    let result = three()\n    let b = four()\n  @console.print(\"{result} {b}\")\n",
    );
    assert_eq!(
        apply_all(&root),
        Vec::<String>::new(),
        "{}",
        repaired(&root)
    );
    assert_eq!(run(&root), "3 4\n");
}

#[test]
fn two_handling_clauses_on_one_line_each_get_their_own_f4() {
    for edition in ["0.1", "0.2"] {
        let root = package(
            "f4",
            edition,
            "module m\n\npub kind Oops = | Bad\n\nfetch(x: Int) -> Option[Int]\n  fails Oops\n=\n  if x > 0 then Some(x) else fail Bad\n\npub use_it(x: Int) -> Int\n=\n  let v = fetch(x) if error use None if none use 0\n  v\n",
        );
        assert_eq!(
            apply_all(&root),
            Vec::<String>::new(),
            "{}",
            repaired(&root)
        );
        assert!(
            repaired(&root)
                .contains("  let v = fetch(x)\n    if error use None\n    if none use 0\n"),
            "{}",
            repaired(&root)
        );
    }
}

#[test]
fn one_type_parameter_gets_one_constraint_edit_the_strongest_f5() {
    // Every operator on `T` that its constraint does not allow is U2026; the edit that writes
    // the constraint is on one of them, with the strongest constraint the body needs.
    for (body, constraint) in [
        ("pub both[T](a: T, b: T) -> Bool = a == b or a < b", "Ord"),
        (
            "pub f[T](a: T, b: T) -> Int = if a == b then a.to_int() else 0",
            "Integer",
        ),
        (
            "pub g[T](a: T, b: T) -> Text = if a < b then \"{a}\" else \"{b}\"",
            "Ord",
        ),
        ("pub h[A, T](a: A, x: T, y: T) -> Bool = x == y", "Eq"),
    ] {
        let root = package("f5", "0.2", &format!("module m\n\n{body}\n"));
        assert_eq!(
            apply_all(&root),
            Vec::<String>::new(),
            "{}",
            repaired(&root)
        );
        assert!(
            repaired(&root).contains(&format!("T: {constraint}]")),
            "{}",
            repaired(&root)
        );
    }
}

#[test]
fn every_same_line_clause_of_a_body_is_repaired_in_one_round() {
    // #268: the parser goes on past a handling clause on its expression's line, so a second
    // such line in the same body is reported, with its edit, in the same check.
    for edition in ["0.1", "0.2"] {
        let root = package(
            "two_lines",
            edition,
            "module m\n\npub total(xs: List[Int]) -> Int\n=\n  let a = xs.first() if none use 0\n  let b = xs.first() if none use 1\n  a + b\n",
        );
        assert_eq!(
            diagnostics(&root)
                .iter()
                .map(|d| d["code"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>(),
            ["U0101", "U0101"]
        );
        assert_eq!(
            apply_all(&root),
            Vec::<String>::new(),
            "{}",
            repaired(&root)
        );
        assert!(
            repaired(&root).contains(
                "  let a = xs.first()\n    if none use 0\n  let b = xs.first()\n    if none use 1\n"
            ),
            "{}",
            repaired(&root)
        );
    }
}
