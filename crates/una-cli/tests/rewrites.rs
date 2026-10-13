//! The exact rewrites of foreign syntax (ADR 0096, part A): for each row of rule 3's table, the
//! foreign source, the first repair's edit applied, the expected Una source, and a check of the
//! result that reports no syntax error; and, where a row's condition fails, a repair that
//! keeps its description without an edit.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use pretty_assertions::assert_eq;
use serde_json::Value;

/// A package of edition 0.1 whose only module is `m`, holding `source`.
fn package(name: &str, source: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("rewrites")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        "[package]\nname = \"c\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n",
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

fn diagnostics(root: &Path) -> Vec<Value> {
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .args(["--json", "check", "--tier", "0", "."])
        .current_dir(root)
        .output()
        .unwrap();
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    document["diagnostics"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The character offset of a 1-based line and column of `text`.
fn offset(text: &str, position: &Value) -> usize {
    let line = position["line"].as_u64().unwrap() as usize;
    let col = position["col"].as_u64().unwrap() as usize;
    let start: usize = text
        .split_inclusive('\n')
        .take(line - 1)
        .map(str::len)
        .sum();
    let rest = &text[start..];
    start
        + rest
            .char_indices()
            .nth(col - 1)
            .map_or(rest.len(), |(i, _)| i)
}

/// The first syntax diagnostic of `source` and its first repair.
fn first_syntax_repair(name: &str, source: &str) -> (Value, Value) {
    let root = package(name, source);
    let found = diagnostics(&root);
    let diagnostic = found
        .iter()
        .find(|d| d["code"].as_str().unwrap().starts_with("U0"))
        .unwrap_or_else(|| panic!("no syntax diagnostic in {found:#?}"))
        .clone();
    let repair = diagnostic["repairs"][0].clone();
    (diagnostic, repair)
}

/// Applies the first repair of the first syntax diagnostic of `source`, checks that the result
/// is `expected`, and that `expected` reports no syntax error.
fn rewrites_to(name: &str, source: &str, expected: &str) {
    let (diagnostic, repair) = first_syntax_repair(name, source);
    let edit = &repair["edit"];
    assert!(edit.is_object(), "no edit: {diagnostic:#?}");
    assert_eq!(repair["escape_cost"], 0);
    let start = offset(source, &edit["start"]);
    let end = offset(source, &edit["end"]);
    let mut fixed = source.to_owned();
    fixed.replace_range(start..end, edit["replacement"].as_str().unwrap());
    assert_eq!(fixed, expected);
    let after = diagnostics(&package(&format!("{name}-fixed"), &fixed));
    let syntax: Vec<&Value> = after
        .iter()
        .filter(|d| d["code"].as_str().unwrap().starts_with("U0"))
        .collect();
    assert!(syntax.is_empty(), "the rewrite leaves {syntax:#?}");
}

/// The first syntax diagnostic of `source` has a repair without an edit that says `says`.
fn advises(name: &str, source: &str, says: &str) {
    let (diagnostic, repair) = first_syntax_repair(name, source);
    assert!(
        repair["edit"].is_null(),
        "an edit where the condition fails: {diagnostic:#?}"
    );
    assert!(
        repair["description"].as_str().unwrap().contains(says),
        "{diagnostic:#?}"
    );
}

#[test]
fn then_at_the_end_of_a_line() {
    rewrites_to(
        "then-eol",
        "module m\n\npub f(x: Int) -> Int\n=\n  if x > 0 then\n    x\n  else\n    0\n",
        "module m\n\npub f(x: Int) -> Int\n=\n  if x > 0\n    x\n  else\n    0\n",
    );
}

#[test]
fn an_else_chain_on_the_next_lines() {
    rewrites_to(
        "else-chain",
        "module m\n\npub f(x: Int) -> Int\n=\n  if x > 0 then 1 else\n  let y = x\n  y\n",
        "module m\n\npub f(x: Int) -> Int\n=\n  if x > 0\n    1\n  else\n    let y = x\n    y\n",
    );
}

#[test]
fn then_and_else_at_the_start_of_lines() {
    rewrites_to(
        "then-start",
        "module m\n\npub f(x: Int) -> Int\n=\n  if x > 0\n    then 1\n    else 0\n",
        "module m\n\npub f(x: Int) -> Int\n=\n  if x > 0\n    1\n  else\n    0\n",
    );
}

#[test]
fn a_guard_without_else() {
    rewrites_to(
        "guard",
        "module m\n\npub kind E = Broke\n\npub f(x: Int) -> Int\n  fails E\n=\n  if x < 0 then fail Broke\n  x + 1\n",
        "module m\n\npub kind E = Broke\n\npub f(x: Int) -> Int\n  fails E\n=\n  if x < 0\n    fail Broke\n  else\n    x + 1\n",
    );
}

#[test]
fn equals_at_the_end_of_a_header() {
    rewrites_to(
        "header-eol",
        "module m\n\npub f(x: Int) -> Int =\n  x + 1\n",
        "module m\n\npub f(x: Int) -> Int\n=\n  x + 1\n",
    );
}

#[test]
fn equals_indented_under_the_clauses() {
    rewrites_to(
        "header-indented",
        "module m\n\npub f(x: Int) -> Int\n  requires x > 0\n  = x + 1\n",
        "module m\n\npub f(x: Int) -> Int\n  requires x > 0\n=\n  x + 1\n",
    );
}

#[test]
fn a_body_after_equals_on_its_line() {
    rewrites_to(
        "header-body",
        "module m\n\npub f(x: Int) -> Int\n  requires x > 0\n= x + 1\n",
        "module m\n\npub f(x: Int) -> Int\n  requires x > 0\n=\n  x + 1\n",
    );
}

#[test]
fn a_name_starting_with_an_underscore() {
    rewrites_to(
        "underscore",
        "module m\n\npub f(_seats: Int) -> Int = _seats + 1\n",
        "module m\n\npub f(seats: Int) -> Int = seats + 1\n",
    );
}

#[test]
fn a_rename_never_takes_a_visible_name() {
    // Rule 3a: `x` is an outer local and a function of the module, so `_x` becomes `x2`, and
    // every reference to the local and to the function is unchanged.
    rewrites_to(
        "fresh-names",
        "module m\n\nx(n: Int) -> Int = n\n\npub f(a: Int) -> Int\n=\n  let x = a\n  let _x = x(x)\n  _x + x\n",
        "module m\n\nx(n: Int) -> Int = n\n\npub f(a: Int) -> Int\n=\n  let x = a\n  let x2 = x(x)\n  x2 + x\n",
    );
}

#[test]
fn an_underscore_binding_is_left_out() {
    rewrites_to(
        "lone-underscore",
        "module m\n\npub f(o: Option[Int]) -> Bool\n=\n  when o\n    is Some(_) -> true\n    is None -> false\n",
        "module m\n\npub f(o: Option[Int]) -> Bool\n=\n  when o\n    is Some -> true\n    is None -> false\n",
    );
}

#[test]
fn a_one_line_when() {
    rewrites_to(
        "one-line-when",
        "module m\n\npub f(x: Option[Int]) -> Int\n=\n  when x is Some(value) -> value is None -> 0\n",
        "module m\n\npub f(x: Option[Int]) -> Int\n=\n  when x\n    is Some(value) -> value\n    is None -> 0\n",
    );
}

#[test]
fn a_let_starting_an_arm() {
    rewrites_to(
        "arm-let",
        "module m\n\npub f(x: Option[Int]) -> Int\n=\n  when x\n    is Some(value) -> let y = value + 1\n      y * 2\n    is None -> 0\n",
        "module m\n\npub f(x: Option[Int]) -> Int\n=\n  when x\n    is Some(value) ->\n      let y = value + 1\n      y * 2\n    is None -> 0\n",
    );
}

#[test]
fn fn_before_a_declaration() {
    rewrites_to(
        "fn-word",
        "module m\n\nfn f(x: Int) -> Int = x\n",
        "module m\n\nf(x: Int) -> Int = x\n",
    );
}

#[test]
fn priv_before_a_declaration() {
    rewrites_to(
        "priv-word",
        "module m\n\npriv f(x: Int) -> Int = x\n",
        "module m\n\nf(x: Int) -> Int = x\n",
    );
}

#[test]
fn a_semicolon_at_the_end_of_a_line() {
    rewrites_to(
        "semicolon",
        "module m\n\npub f(a: Int) -> Int\n=\n  let b = a;\n  b\n",
        "module m\n\npub f(a: Int) -> Int\n=\n  let b = a\n  b\n",
    );
}

#[test]
fn a_multi_line_lambda_names_the_form_without_an_edit() {
    advises(
        "lambda",
        "module m\n\npub f(xs: List[Int]) -> List[Int]\n=\n  xs.map(each: fn(x) ->\n    let y = x + 1\n    y)\n",
        "named function",
    );
}

#[test]
fn a_guard_with_nothing_after_it_has_no_edit() {
    // The condition of rule 3 fails: no rest of the block becomes the `else` branch.
    let (diagnostic, repair) = first_syntax_repair(
        "guard-last",
        "module m\n\npub kind E = Broke\n\npub f(x: Int) -> Int\n  fails E\n=\n  let y = x\n  if y < 0 then fail Broke\n",
    );
    assert!(repair["edit"].is_null(), "{diagnostic:#?}");
}
