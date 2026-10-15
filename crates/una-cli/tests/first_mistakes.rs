//! Repairs for common first mistakes (#743), found by writing the programs of Una by Example
//! (#733, #576): a reserved keyword where a module path segment or an operation is named, a
//! requirement identifier off the convention, and a method in a contract. Each diagnostic says
//! what the rule is and what to write instead; an edit is offered only where it stays inside
//! the declaration (ADR 0096 rule 2(a)), and the edited program checks.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

use pretty_assertions::assert_eq;
use serde_json::Value;

/// A package of edition 0.2 named `name`, whose only module is `src/<file>.una`.
fn package(name: &str, file: &str, source: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("first-mistakes")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        "[package]\nname = \"c\"\nversion = \"0.1.0\"\nedition = \"0.2\"\n",
    )
    .unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(root.join("src").join(format!("{file}.una")), source).unwrap();
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

/// The only diagnostic of `code` of the package.
fn only(root: &Path, code: &str) -> Value {
    let found = diagnostics(root);
    let of: Vec<&Value> = found.iter().filter(|d| d["code"] == code).collect();
    assert_eq!(of.len(), 1, "{found:#?}");
    of[0].clone()
}

#[test]
fn a_reserved_keyword_as_a_module_path_segment_names_the_rule_and_a_name() {
    let root = package("module-when", "when", "module when\n\nf() -> Int = 1\n");
    let found = only(&root, "U0101");
    assert_eq!(
        found["message"],
        "`when` is a reserved keyword, so it cannot be a module path segment"
    );
    let repair = &found["repairs"][0];
    assert_eq!(
        repair["description"],
        "choose another name, such as `when_module`"
    );
    // Renaming a module moves its file and changes its imports: advice, not an edit.
    assert!(repair["edit"].is_null(), "{repair:#?}");
}

#[test]
fn an_entry_module_whose_path_is_refused_is_reported_once() {
    let root = package(
        "entry-when",
        "when",
        "module when\n\npub main() -> Unit = unit\n",
    );
    std::fs::write(
        root.join("una.toml"),
        "[package]\nname = \"c\"\nversion = \"0.1.0\"\nedition = \"0.2\"\n\n[run]\nentry = \"when\"\n",
    )
    .unwrap();
    let codes: Vec<String> = diagnostics(&root)
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(codes, vec!["U0101"]);
}

#[test]
fn a_reserved_keyword_as_an_operation_name_names_the_rule_and_a_name() {
    let root = package(
        "op-take",
        "m",
        "module m\n\ncapability Stock\n  op take(item: Text) -> Bool\n",
    );
    let found = only(&root, "U0101");
    assert_eq!(
        found["message"],
        "`take` is a reserved keyword, so it cannot be an operation name"
    );
    let repair = &found["repairs"][0];
    assert_eq!(
        repair["description"],
        "choose another name, such as `take_op`"
    );
    assert!(repair["edit"].is_null(), "{repair:#?}");
}

const REQUIREMENT: &str = "module m\n\nrequirement SOLD_OUT\n  text \"x\"\n  source human\n  owner team\n  status active\n";

#[test]
fn an_unreferenced_requirement_identifier_is_renamed_from_its_own_name() {
    let root = package("requirement-free", "m", REQUIREMENT);
    let found = only(&root, "U1005");
    assert_eq!(
        found["message"],
        "requirement identifier `SOLD_OUT` must be upper snake case ending with `_` and a number, as in `SOLD_OUT_1`"
    );
    let repair = &found["repairs"][0];
    assert_eq!(repair["edit"]["replacement"], "SOLD_OUT_1", "{repair:#?}");
    // The edited program has no U1005.
    let fixed = REQUIREMENT.replace("requirement SOLD_OUT\n", "requirement SOLD_OUT_1\n");
    let after = package("requirement-free-fixed", "m", &fixed);
    assert_eq!(diagnostics(&after), Vec::<Value>::new());
}

#[test]
fn a_cited_requirement_identifier_gets_advice_that_names_where_it_is_cited() {
    let source = format!(
        "{REQUIREMENT}\nkind Availability = SoldOut | Open\n\navailability(remaining: Int) -> Availability\n  derived from SOLD_OUT\n=\n  if remaining < 1 then SoldOut else Open\n"
    );
    let root = package("requirement-cited", "m", &source);
    let found = only(&root, "U1005");
    let repair = &found["repairs"][0];
    assert!(repair["edit"].is_null(), "{repair:#?}");
    assert_eq!(
        repair["description"],
        "rename `SOLD_OUT` to `SOLD_OUT_1` here and in the `derived from` clauses that name it, at src/m.una:12"
    );
}

#[test]
fn the_suggested_requirement_identifier_is_one_the_package_does_not_hold() {
    let source = format!(
        "{REQUIREMENT}\nrequirement SOLD_OUT_1\n  text \"y\"\n  source human\n  owner team\n  status active\n"
    );
    let root = package("requirement-taken", "m", &source);
    let found = only(&root, "U1005");
    assert_eq!(found["repairs"][0]["edit"]["replacement"], "SOLD_OUT_2");
}

#[test]
fn to_int_in_a_contract_says_what_the_refinement_language_offers() {
    let root = package(
        "to-int-requires",
        "m",
        "module m\n\nadd(a: U8, b: U8) -> U8\n  requires a.to_int() + b.to_int() <= 255\n=\n  a + b\n",
    );
    let found = diagnostics(&root);
    let first = found.iter().find(|d| d["code"] == "U2014").unwrap();
    let description = first["repairs"][0]["description"].as_str().unwrap();
    assert!(
        description.starts_with("state the bound on the value in its own type, such as `a <= 100`"),
        "{first:#?}"
    );
    assert!(first["repairs"][0]["edit"].is_null());
}
