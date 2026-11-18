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
        .env_remove("UNA_PROOF_KEY")
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
    let fixed = fix_copy(root);
    let codes = apply_all_by_hand(root);
    agrees_with_fix(root, &fixed, &codes);
    codes
}

fn apply_all_by_hand(root: &Path) -> Vec<String> {
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

/// A fresh copy of the package at `root`, next to it, with `suffix` added to its name.
fn copy_package(root: &Path, suffix: &str) -> PathBuf {
    let copy = root.with_file_name(format!(
        "{}-{suffix}",
        root.file_name().unwrap().to_string_lossy()
    ));
    let _ = std::fs::remove_dir_all(&copy);
    for file in files_of(root) {
        let target = copy.join(&file);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::copy(root.join(&file), target).unwrap();
    }
    copy
}

/// Every file of the package at `root`, relative to it, in path order.
fn files_of(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(root.join(&dir)).unwrap() {
            let entry = entry.unwrap();
            let relative = dir.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                pending.push(relative);
            } else {
                out.push(relative);
            }
        }
    }
    out.sort();
    out
}

/// The modules of the package at `root`, by path.
fn modules_of(root: &Path) -> std::collections::BTreeMap<PathBuf, String> {
    files_of(root)
        .into_iter()
        .filter(|file| file.extension().is_some_and(|e| e == "una"))
        .map(|file| {
            let text = std::fs::read_to_string(root.join(&file)).unwrap();
            (file, text)
        })
        .collect()
}

/// A copy of the package at `root` that `una fix` repaired, before the test applies the
/// edits by hand to `root` itself, with the `una.fix/1` document.
fn fix_copy(root: &Path) -> (PathBuf, Value) {
    let copy = copy_package(root, "fix");
    let output = una(&copy, &["--json", "fix", "--tier", "0", "."]);
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["schema"], "una.fix/1", "{document}");
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    (copy, document)
}

/// ADR 0084: `una fix` applies what the test applies by hand, in safe rounds. What remains
/// after it carries no edit it could apply: every remaining first repair with an edit is
/// listed as skipped. When the edits by hand leave nothing, `una fix` leaves nothing either,
/// and its files are the hand-repaired ones in canonical form.
fn agrees_with_fix(root: &Path, (fixed, document): &(PathBuf, Value), codes: &[String]) {
    assert_eq!(document["stopped"], "done", "{document}");
    let skipped = document["skipped"].as_array().unwrap();
    for diagnostic in document["diagnostics"].as_array().unwrap() {
        let edit = &diagnostic["repairs"][0]["edit"];
        if edit.is_null() {
            continue;
        }
        assert!(
            skipped.iter().any(|s| s["code"] == diagnostic["code"]
                && s["file"] == edit["file"]
                && s["start"] == edit["start"]),
            "a mechanical edit left unapplied and not skipped: {diagnostic}"
        );
    }
    if codes.is_empty() && skipped.is_empty() {
        assert_eq!(document["diagnostics"], serde_json::json!([]), "{document}");
        let canonical = copy_package(root, "canonical");
        let output = una(&canonical, &["fmt", "."]);
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(modules_of(fixed), modules_of(&canonical));
    }
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
fn then_at_the_end_of_a_line_before_an_else_if_chain_is_repaired_exactly() {
    // #935: the exact edit of U0101 for `if c then` at the end of a line keeps the `else if`
    // chain that follows, with its blocks, so the repaired program checks and runs.
    let root = package(
        "then-chain",
        "0.2",
        "module m\n\n--- The box a parcel needs.\nbox_for(grams: Int) -> Text\n=\n  if grams <= 1_000 then\n    \"small box\"\n  else if grams <= 5_000\n    let kilos = (grams + 999) / 1_000\n    \"medium box for {kilos} kg\"\n  else\n    \"large box\"\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"{box_for(800)}; {box_for(3_200)}; {box_for(9_000)}\")\n",
    );
    assert_eq!(
        apply_all(&root),
        Vec::<String>::new(),
        "{}",
        repaired(&root)
    );
    assert!(repaired(&root).contains(
        "  if grams <= 1_000\n    \"small box\"\n  else if grams <= 5_000\n    let kilos"
    ));
    assert_eq!(run(&root), "small box; medium box for 4 kg; large box\n");
}

#[test]
fn a_misspelled_operation_is_one_code_with_one_edit_in_a_body_and_an_example() {
    // ADR 0082: an operation the capability does not declare is U1001 with the closest
    // operation as an edit, in a body and after `expect performed` alike, in both editions.
    for edition in ["0.1", "0.2"] {
        let root = package(
            "undeclared-op",
            edition,
            "module m\n\npub capability Log\n  op write(line: Text) -> Unit\n\npub handler quiet() for Log\n  op write(line) = unit\n\npub note() -> Unit\n  uses @log: Log\n=\n  @log.wirte(\"x\")\n\nexample \"a note is written\"\n  given @log = quiet()\n  do note()\n  expect performed @log.wirte\n",
        );
        let codes: Vec<String> = diagnostics(&root)
            .iter()
            .map(|d| d["code"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(codes, ["U1001", "U1001"]);
        assert_eq!(
            apply_all(&root),
            Vec::<String>::new(),
            "{}",
            repaired(&root)
        );
        assert!(repaired(&root).contains("expect performed @log.write\n"));
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

/// A package of edition 0.1 with the modules `files`, by name, whose entry is `b`.
fn package_of(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("repairs")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        "[package]\nname = \"c\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n\n[run]\nentry = \"b\"\ngrants = [\"console\"]\n",
    )
    .unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    for (module, source) in files {
        std::fs::write(root.join(format!("src/{module}.una")), source).unwrap();
    }
    root
}

/// Applies the first repair's edit of every diagnostic of one check, in every file, and returns
/// the codes of the check that follows. Every diagnostic must carry an edit: an agent applies
/// them with no step of its own.
fn apply_all_files(root: &Path) -> Vec<String> {
    let fixed = fix_copy(root);
    let codes = apply_all_files_by_hand(root);
    agrees_with_fix(root, &fixed, &codes);
    codes
}

fn apply_all_files_by_hand(root: &Path) -> Vec<String> {
    let mut edits: std::collections::BTreeMap<String, Vec<(Value, Value, String)>> =
        std::collections::BTreeMap::new();
    for diagnostic in diagnostics(root) {
        let edit = diagnostic["repairs"][0]["edit"].clone();
        assert!(!edit.is_null(), "a diagnostic with no edit: {diagnostic}");
        edits
            .entry(edit["file"].as_str().unwrap().to_owned())
            .or_default()
            .push((
                edit["start"].clone(),
                edit["end"].clone(),
                edit["replacement"].as_str().unwrap().to_owned(),
            ));
    }
    for (file, edits) in edits {
        let path = root.join(file);
        let text = std::fs::read_to_string(&path).unwrap();
        let mut edits: Vec<(usize, usize, String)> = edits
            .into_iter()
            .map(|(start, end, replacement)| {
                (offset(&text, &start), offset(&text, &end), replacement)
            })
            .collect();
        edits.sort();
        let mut chars: Vec<char> = text.chars().collect();
        for (start, end, replacement) in edits.iter().rev() {
            chars.splice(start..end, replacement.chars());
        }
        std::fs::write(&path, chars.into_iter().collect::<String>()).unwrap();
    }
    diagnostics(root)
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_rename_never_makes_a_call_reach_another_function_125() {
    // #125 (R-1, R-2): `a` already has a `the_table` of its own. Renamed one file at a time,
    // `a`'s `table` became `the_table_2` and `b`'s call `the_table`, and renaming `b`'s import
    // line alike made the call reach `a.the_table`: the program printed 101. The new name is
    // now chosen across the package, and the import line gets the edit too.
    let a = "module a\n\n--- Another function, which already has the name.\npub the_table(x: Int) -> Int = x + 100\n\npub table(x: Int) -> Int = x\n";
    let b = "module b\n\nimport a (table)\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let n = table(1)\n  @console.print(\"{n}\")\n";
    let root = package_of("rename-across-modules", &[("a", a), ("b", b)]);
    assert_eq!(apply_all_files(&root), Vec::<String>::new());
    assert_eq!(run(&root), "1\n");
    let b = std::fs::read_to_string(root.join("src/b.una")).unwrap();
    assert!(b.contains("import a (the_table_2)\n"), "{b}");
}

#[test]
fn a_rename_takes_no_name_of_another_module_125() {
    // The new name is none of the package's names, a private one of another module included,
    // and a kind renamed in its module is renamed alike where another module names it.
    let a = "module a\n\npub kind service = Open | Closed\n\npub table(x: Int) -> Int = x\n";
    let b = "module b\n\nimport a (Open, service, table)\nimport c (helper)\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let s: service = Open\n  let n = table(1) + helper()\n  @console.print(\"{n} {s is Open}\")\n";
    let c = "module c\n\nthe_table() -> Int = 10\n\npub helper() -> Int = the_table()\n";
    let root = package_of("rename-private-clash", &[("a", a), ("b", b), ("c", c)]);
    assert_eq!(apply_all_files(&root), Vec::<String>::new());
    assert_eq!(run(&root), "11 true\n");
    let a = std::fs::read_to_string(root.join("src/a.una")).unwrap();
    assert!(
        a.contains("pub kind Service = ") && a.contains("pub the_table_2("),
        "{a}"
    );
}

#[test]
fn json_written_in_text_is_one_edit_per_brace_403() {
    // #403 (D-1 of the dogfood pass): a `{` that no expression can follow is a literal brace
    // written without its escape. It was a cascade of U0101, U0004, U0001 and U0006 that
    // pointed elsewhere; now each brace is one U0007 with the edit to `\{` or `\}`, an
    // interpolation still interpolates, and applying the edits prints the JSON meant.
    let root = package(
        "json-in-text",
        "0.2",
        "module m\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let n = 2\n  @console.print(\"{\\\"a\\\": [{\\\"b\\\": {n}}], \\\"c\\\": {}}\")\n",
    );
    let found: Vec<(String, String)> = diagnostics(&root)
        .iter()
        .map(|d| {
            (
                d["code"].as_str().unwrap().to_owned(),
                d["repairs"][0]["edit"]["replacement"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            )
        })
        .collect();
    let brace = |r: &str| ("U0007".to_owned(), r.to_owned());
    assert_eq!(
        found,
        [
            brace("\\{"),
            brace("\\{"),
            brace("\\}"),
            brace("\\{"),
            brace("\\}"),
            brace("\\}")
        ]
    );
    assert_eq!(
        apply_all(&root),
        Vec::<String>::new(),
        "{}",
        repaired(&root)
    );
    assert_eq!(run(&root), "{\"a\": [{\"b\": 2}], \"c\": {}}\n");
}

#[test]
fn an_edit_marker_before_a_call_moves_to_its_first_argument_403() {
    // #403 (D-8): `edit f(x, ...)` marks the call, which no marker can; the edit moves it to
    // the first argument, as a method receiver's marker stands for the first parameter.
    let root = package(
        "edit-marker",
        "0.2",
        "module m\n\npub kind Tally\n  count: Int\n\nadd(edit t: Tally, by: Int) -> Unit\n=\n  t.count = t.count + by\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit t = Tally(count: 0)\n  edit add(t, by: 2)\n  @console.print(\"{t.count}\")\n",
    );
    assert_eq!(
        apply_all(&root),
        Vec::<String>::new(),
        "{}",
        repaired(&root)
    );
    assert!(repaired(&root).contains("  add(edit t, by: 2)\n"));
    assert_eq!(run(&root), "2\n");
}

#[test]
fn a_labeled_value_in_a_prelude_kind_is_told_of_a_policy_and_of_key_name() {
    // #404 (dogfood D-5): the declared label of a field of a prelude kind cannot be raised, so
    // U4001 there names what can apply: a policy, and for a header's value, `key_name`.
    let root = package(
        "prelude-field-label",
        "0.2",
        "module m\n\npub accept(token: secret Text) -> Header = Header(name: \"X-Api-Token\", value: token)\n\npub shown(text: secret Text) -> Json = Str(value: text)\n",
    );
    let found: Vec<(String, Vec<String>)> = diagnostics(&root)
        .iter()
        .map(|d| {
            let advice = d["repairs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["description"].as_str().unwrap().to_owned())
                .collect();
            (d["code"].as_str().unwrap().to_owned(), advice)
        })
        .collect();
    assert_eq!(
        found,
        [
            (
                "U4001".to_owned(),
                vec!["a field of `Header`, a kind of the standard library, keeps its declared label: lower this value through a `policy`, or give a token by the name of its key, as `key_name` of the request, rather than in a header".to_owned()]
            ),
            (
                "U4001".to_owned(),
                vec!["a field of `Json`, a kind of the standard library, keeps its declared label: lower this value through a `policy`".to_owned()]
            ),
        ]
    );
}

/// The code and the repairs of each diagnostic of `una --json check`, as (code, [(description,
/// replacement)]).
/// A diagnostic's code and its repairs, as (description, replacement).
type Repairs = (String, Vec<(String, Option<String>)>);

fn repairs_of(root: &Path) -> Vec<Repairs> {
    diagnostics(root)
        .iter()
        .map(|d| {
            let repairs = d["repairs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    (
                        r["description"].as_str().unwrap().to_owned(),
                        r["edit"]["replacement"].as_str().map(str::to_owned),
                    )
                })
                .collect();
            (d["code"].as_str().unwrap().to_owned(), repairs)
        })
        .collect()
}

#[test]
fn an_unknown_name_close_to_one_name_in_scope_is_offered_it_as_advice_419() {
    // #419: U1001 offers the closest name in scope, for a value and for a type, as
    // advice without an edit, even when it is the only close one: a typo's fix is a guess, and
    // an edit applied unread could bind another name that checks (R-1, #125).
    let root = package(
        "unknown-name-one",
        "0.2",
        "module m\n\nshow(n: Txt) -> Text = \"{n}\"\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let total = 3\n  @console.print(show(\"{totl}\"))\n",
    );
    assert_eq!(
        repairs_of(&root),
        [
            (
                "U1001".to_owned(),
                vec![("names in scope close to `Txt`: `Text`".to_owned(), None)]
            ),
            (
                "U1001".to_owned(),
                vec![("names in scope close to `totl`: `total`".to_owned(), None)]
            ),
        ]
    );
}

#[test]
fn an_unknown_name_close_to_several_lists_three_in_a_fixed_order_419() {
    // #419: several candidates at the same distance are listed by distance, then bytewise,
    // three at most, and none is written for the agent.
    let root = package(
        "unknown-name-several",
        "0.2",
        "module m\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let cat = 1\n  let car = 2\n  let cap = 3\n  let can = 4\n  @console.print(\"{cat + car + cap + can + caz}\")\n",
    );
    assert_eq!(
        repairs_of(&root),
        [(
            "U1001".to_owned(),
            vec![(
                "names in scope close to `caz`: `can`, `cap`, `car`".to_owned(),
                None
            )]
        )]
    );
}

#[test]
fn an_unknown_name_far_from_every_name_in_scope_has_no_repair_419() {
    // #419: nothing is offered when no name in scope is close.
    let root = package(
        "unknown-name-none",
        "0.2",
        "module m\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"{zzzzzzzz}\")\n",
    );
    assert_eq!(repairs_of(&root), [("U1001".to_owned(), Vec::new())]);
}

#[test]
fn una_fix_never_widens_authority_or_visibility_0084() {
    // ADR 0084 (2026-10-08: "Never apply, no flag"): the edit that adds `flush` to
    // the `uses` clause and the one that makes `helper` public are reported as skipped, and
    // the files keep their `uses` clause and their private declaration.
    for edition in ["0.1", "0.2"] {
        let root = package_of(
            &format!("fix-authority-{edition}"),
            &[
                ("a", "module a\n\nhelper(x: Int) -> Int = x\n"),
                (
                    "b",
                    "module b\n\nimport a (helper)\n\npub capability Log\n  op write(line: Text) -> Unit\n  op flush() -> Unit\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"{helper(1)}\")\n\npub finish() -> Unit\n  uses @log: Log[write]\n=\n  @log.flush()\n",
                ),
            ],
        );
        let manifest = root.join("una.toml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(
            &manifest,
            text.replace("edition = \"0.1\"", &format!("edition = \"{edition}\"")),
        )
        .unwrap();
        let before = modules_of(&root);
        let output = una(&root, &["--json", "fix", "--tier", "0", "."]);
        assert_eq!(output.status.code(), Some(1));
        let document: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(document["applied"], serde_json::json!([]), "{document}");
        let skipped: Vec<(&str, &str)> = document["skipped"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["code"].as_str().unwrap(), s["reason"].as_str().unwrap()))
            .collect();
        assert_eq!(
            skipped,
            [("U1004", "widens authority"), ("U3002", "widens authority")]
        );
        assert_eq!(modules_of(&root), before);
    }
}

#[test]
fn una_fix_dry_run_writes_nothing_and_reports_the_edits_0084() {
    let source = "module m\n\npub kind P = Over(index: Int) | Fine\n\npub f(p: P) -> Int\n  requires p != Over\n=\n  1\n";
    let root = package("fix-dry-run", "0.2", source);
    let output = una(&root, &["--json", "fix", "--dry-run", "--tier", "0", "."]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["dry_run"], true);
    assert_eq!(document["rounds"], 1);
    assert_eq!(document["applied"][0]["replacement"], "not (p is Over)");
    assert_eq!(repaired(&root), source);
    // The same run without `--dry-run` writes what the dry run reported, and prints one line
    // per edit.
    let output = una(&root, &["fix", "--tier", "0", "."]);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.starts_with("fixed src/m.una:6:12 U2014 "),
        "{stdout}"
    );
    assert!(repaired(&root).contains("requires not (p is Over)\n"));
    assert!(una(&root, &["check", "--tier", "0", "."]).status.success());
}

/// The codes of `una --json check`, sorted.
fn codes_of(root: &Path) -> Vec<String> {
    let mut codes: Vec<String> = diagnostics(root)
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_owned())
        .collect();
    codes.sort();
    codes
}

/// ADR 0100's sources for `una fix`: `E`, a function `risky` that fails with it, and `rest`.
fn dead_package(name: &str, rest: &str) -> PathBuf {
    package(
        name,
        "0.3",
        &format!(
            "module m\n\nkind E = | Bad\n\nrisky(n: Int) -> Int\n  fails E\n=\n  if n < 0 then fail Bad else n\n\n{rest}"
        ),
    )
}

#[test]
fn una_fix_removes_a_dead_clause_and_the_failure_it_gave_the_callers_0100() {
    // ADR 0100, the root case: the dead clause gives `helper` a kind, so `one` reports U3007.
    // U3025 is reported with it, and once `una fix` deletes the clause, no U3007 remains.
    let root = dead_package(
        "dead-root",
        "helper(n: Int) -> Int\n=\n  let x = n + 1\n    if error fail Bad\n  x\n\npub one(n: Int) -> Int = helper(n)\n",
    );
    assert_eq!(codes_of(&root), ["U3007", "U3025"]);
    let output = una(&root, &["--json", "fix", "--tier", "0", "."]);
    assert_eq!(output.status.code(), Some(0), "{}", repaired(&root));
    assert_eq!(codes_of(&root), Vec::<String>::new());
    assert!(!repaired(&root).contains("if error fail Bad"));
}

#[test]
fn a_clause_on_a_callee_with_an_unhandled_failure_survives_una_fix_0100() {
    // ADR 0100, shape (a): `c` leaves the failure of `risky` unhandled (U3007), so it infers
    // no failure, and `user`'s clause on `c(n)` looks dead. `c` is poisoned: `una fix` leaves
    // the clause, and once `c` passes the failure, `user` checks with its clause.
    let root = dead_package(
        "dead-shape-a",
        "c(n: Int) -> Int\n=\n  let x = risky(n)\n  x\n\npub user(n: Int) -> Int\n  fails E\n=\n  let y = c(n)\n    if error use 0\n  y\n",
    );
    assert_eq!(codes_of(&root), ["U3007"]);
    let _ = una(&root, &["--json", "fix", "--tier", "0", "."]);
    let source = repaired(&root);
    assert!(source.contains("    if error use 0\n"), "{source}");
    let fixed = source.replace(
        "  let x = risky(n)\n",
        "  let x = risky(n)\n    if error pass\n",
    );
    std::fs::write(root.join("src/m.una"), fixed).unwrap();
    assert_eq!(codes_of(&root), Vec::<String>::new());
}

#[test]
fn a_clause_on_a_callee_that_fails_without_a_kind_survives_una_fix_0100() {
    // ADR 0100, shape (b): `b` fails without `fails` (U3009), so a clause on its call looks
    // dead. `b` is poisoned: `una fix` leaves the clause, and once `b` declares its kind,
    // `user` checks with its clause.
    let root = dead_package(
        "dead-shape-b",
        "pub b(n: Int) -> Int = if n < 0 then fail Bad else n\n\npub user(n: Int) -> Int\n  fails E\n=\n  let y = b(n)\n    if error use 0\n  y\n",
    );
    assert_eq!(codes_of(&root), ["U3009"]);
    let _ = una(&root, &["--json", "fix", "--tier", "0", "."]);
    let source = repaired(&root);
    assert!(source.contains("    if error use 0\n"), "{source}");
    let fixed = source.replace(
        "pub b(n: Int) -> Int = if",
        "pub b(n: Int) -> Int\n  fails E\n= if",
    );
    std::fs::write(root.join("src/m.una"), fixed).unwrap();
    assert_eq!(codes_of(&root), Vec::<String>::new());
}
