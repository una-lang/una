//! `una fix` moves a package to edition 0.5 (ADR 0137 rule 11, #1629): it rewrites each `let`
//! to `const` and each `let edit` to `var`, the first repairs of their U7007, in blocks, in arms
//! and in `together` branches, keeping comments and layout; a second run finds nothing to
//! rewrite; and it lists, with file and line, each `let` it leaves: one in the code of a doc
//! comment, and one in a file that does not parse, which waits until the file's own repairs
//! make it parse. The equivalence over the repository's packages is in `editions.rs`.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn una_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

/// A package of `edition`, written in `ita.una` with its lock, under the test's temporary
/// directory, whose modules are `modules`, each a path under `src/` and its text.
fn package(name: &str, edition: &str, modules: &[(&str, &str)]) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("binding_words_fix")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("ita.una"),
        format!("ita {name}\n  version 0.1.0\n  edition {edition}\n"),
    )
    .unwrap();
    for (path, text) in modules {
        std::fs::write(root.join("src").join(path), text).unwrap();
    }
    let lock = una_in(&root, &["ita", "lock", "."]);
    assert!(lock.status.success(), "{lock:?}");
    root
}

/// The `una.fix/1` document of `una --json fix` in `root`, and its exit status.
fn fix(root: &Path) -> (Option<i32>, Value) {
    let output = una_in(root, &["--json", "fix", "."]);
    (
        output.status.code(),
        serde_json::from_slice(&output.stdout).unwrap(),
    )
}

fn read(root: &Path, path: &str) -> String {
    std::fs::read_to_string(root.join("src").join(path)).unwrap()
}

/// Each skipped entry of `document`: its code, `line:col`, description and reason.
fn skipped(document: &Value) -> Vec<(String, String, String, String)> {
    document["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|skip| {
            (
                skip["code"].as_str().unwrap().to_owned(),
                format!("{}:{}", skip["start"]["line"], skip["start"]["col"]),
                skip["description"].as_str().unwrap().to_owned(),
                skip["reason"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn row(code: &str, at: &str, description: &str, reason: &str) -> (String, String, String, String) {
    (
        code.to_owned(),
        at.to_owned(),
        description.to_owned(),
        reason.to_owned(),
    )
}

/// A module of edition 0.4 words: bindings in a block, with annotations and comments, in a
/// `when` arm, in `together` branches, and a test.
const EDITION_0_4: &str = "module m

-- The sum of a number and its double.
pub triple(x: Int) -> Int
=
  -- a comment that says let stays as it is
  let y = x + x -- trailing: let edit too
  let edit total: Int = x

  total = total + y
  total

pub pick(n: Int) -> Int
=
  when n
    is 0 ->
      let one = 1
      one
    else -> n

pub both() -> Int
=
  together
    let a = triple(1)
    let edit b = triple(2)
  a + b

test \"triple\"
  do triple(2)
  expect result == 6
";

/// The same module in the words of edition 0.5: nothing else changes.
const EDITION_0_5: &str = "module m

-- The sum of a number and its double.
pub triple(x: Int) -> Int
=
  -- a comment that says let stays as it is
  const y = x + x -- trailing: let edit too
  var total: Int = x

  total = total + y
  total

pub pick(n: Int) -> Int
=
  when n
    is 0 ->
      const one = 1
      one
    else -> n

pub both() -> Int
=
  together
    const a = triple(1)
    var b = triple(2)
  a + b

test \"triple\"
  do triple(2)
  expect result == 6
";

#[test]
fn una_fix_rewrites_every_let_keeping_comments_and_layout_and_a_second_run_changes_nothing() {
    // The module checks and its test passes in edition 0.4.
    let old = package("kept", "0.4", &[("m.una", EDITION_0_4)]);
    assert_eq!(una_in(&old, &["check", "."]).status.code(), Some(0));
    let test = una_in(&old, &["test", "."]);
    assert!(test.status.success(), "{test:?}");

    let root = package("moved", "0.5", &[("m.una", EDITION_0_4)]);
    let (status, document) = fix(&root);
    assert_eq!(status, Some(0), "{document}");
    assert_eq!(read(&root, "m.una"), EDITION_0_5);
    let applied: Vec<(String, String)> = document["applied"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edit| {
            (
                format!("{}:{}", edit["start"]["line"], edit["start"]["col"]),
                edit["replacement"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let want: Vec<(String, String)> = [
        ("7:3", "const"),
        ("8:3", "var"),
        ("17:7", "const"),
        ("24:5", "const"),
        ("25:5", "var"),
    ]
    .iter()
    .map(|(at, word)| ((*at).to_owned(), (*word).to_owned()))
    .collect();
    assert_eq!(applied, want);
    assert_eq!(document["skipped"], Value::Array(Vec::new()));
    assert_eq!(document["formatted"], Value::Array(Vec::new()));

    // Idempotent: on the file it rewrote, `una fix` finds nothing to rewrite.
    let (status, again) = fix(&root);
    assert_eq!(status, Some(0));
    assert_eq!(again["rounds"], 0);
    assert_eq!(again["applied"], Value::Array(Vec::new()));
    assert_eq!(again["formatted"], Value::Array(Vec::new()));
    assert_eq!(read(&root, "m.una"), EDITION_0_5);

    // The moved package has the result of the original.
    assert_eq!(una_in(&root, &["check", "."]).status.code(), Some(0));
    let moved = una_in(&root, &["test", "."]);
    assert!(moved.status.success(), "{moved:?}");
    assert_eq!(moved.stdout, test.stdout);
}

#[test]
fn una_fix_lists_a_let_in_the_code_of_a_doc_comment_and_leaves_it() {
    let source = "module m

--- The double of `x`: as `let y = x + x`, or `let edit y = x`; let it be.
pub double(x: Int) -> Int
=
  let y = x + x
  y
";
    let root = package("doc_comment", "0.5", &[("m.una", source)]);
    let (status, document) = fix(&root);
    assert_eq!(status, Some(0), "{document}");
    assert_eq!(
        read(&root, "m.una"),
        source.replace("  let y = x + x", "  const y = x + x")
    );
    // The prose `let it be` is no code; each `let` between backticks is listed.
    assert_eq!(
        skipped(&document),
        [
            row("U7007", "3:28", "write `const`", "doc comment"),
            row("U7007", "3:48", "write `var`", "doc comment"),
        ]
    );
    let human = una_in(&root, &["fix", "."]);
    let text = String::from_utf8(human.stdout).unwrap();
    assert!(
        text.contains("skipped src/m.una:3:28 U7007 write `const`: doc comment\n"),
        "{text}"
    );
    // A doc comment is no code that `una check` reads.
    assert_eq!(una_in(&root, &["check", "."]).status.code(), Some(0));
    // In edition 0.4, `let` is the binding word: nothing is listed.
    let old = package("doc_comment_0_4", "0.4", &[("m.una", source)]);
    let (_, document) = fix(&old);
    assert_eq!(document["skipped"], Value::Array(Vec::new()));
}

#[test]
fn una_fix_leaves_and_lists_each_let_of_a_file_that_does_not_parse() {
    let broken = "module b

pub half(x: Int) -> Int
=
  let y = x / 2
  y + )
";
    let clean = "module c

pub twice(x: Int) -> Int
=
  let edit y = x
  y = y + x
  y
";
    let root = package("unparsed", "0.5", &[("b.una", broken), ("c.una", clean)]);
    let (status, document) = fix(&root);
    assert_eq!(status, Some(1));
    assert_eq!(read(&root, "b.una"), broken);
    assert_eq!(read(&root, "c.una"), clean.replace("let edit y", "var y"));
    assert_eq!(
        skipped(&document),
        [row("U7007", "5:3", "write `const`", "does not parse")]
    );
    // `una check` still reports it.
    let check = una_in(&root, &["--json", "check", "."]);
    let check: Value = serde_json::from_slice(&check.stdout).unwrap();
    assert!(
        check["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "U7007" && d["primary"]["file"] == "src/b.una"),
        "{check}"
    );
}

#[test]
fn a_let_waits_for_the_repairs_that_make_its_file_parse() {
    // `=` at the end of the header is U0101 with an exact repair; once applied, the next round
    // rewrites the `let`.
    let source = "module m

pub double(x: Int) -> Int =
  let y = x + x
  y
";
    let root = package("waits", "0.5", &[("m.una", source)]);
    let (status, document) = fix(&root);
    assert_eq!(status, Some(0), "{document}");
    assert_eq!(document["rounds"], 2);
    assert_eq!(
        read(&root, "m.una"),
        "module m\n\npub double(x: Int) -> Int\n=\n  const y = x + x\n  y\n"
    );
}

#[test]
fn a_name_const_is_left_with_its_advice_and_never_made_const_const() {
    // ADR 0137 rule 11: a name `const` is U0101 in edition 0.5 with the advice to rename it and
    // no edit. The file does not parse, so its `let` is left too, not written `const const`.
    let source = "module m

scale(const: Int, x: Int) -> Int = const * x

pub use_it(y: Int) -> Int
=
  let const = 2
  scale(const: const, x: y)
";
    let root = package("const_name", "0.5", &[("m.una", source)]);
    let (status, document) = fix(&root);
    assert_eq!(status, Some(1));
    assert_eq!(read(&root, "m.una"), source);
    assert_eq!(
        skipped(&document),
        [row("U7007", "7:3", "write `const`", "does not parse")]
    );
    let advice: Vec<&Value> = document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "U0101")
        .collect();
    assert!(!advice.is_empty());
    for diagnostic in advice {
        assert_eq!(
            diagnostic["repairs"][0]["edit"],
            Value::Null,
            "{diagnostic}"
        );
        assert!(
            diagnostic["repairs"][0]["description"]
                .as_str()
                .unwrap()
                .contains("const_value"),
            "{diagnostic}"
        );
    }
}
