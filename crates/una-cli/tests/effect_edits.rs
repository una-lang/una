//! `una fix` writes the condition that an `effect_condition` obligation asks of a callee
//! (ADR 0092 and ADR 0084, amendments of 2026-10-14): for each case, the source before, the
//! source `una fix` leaves, and a check of it; for each condition (a) to (h) that fails, a first
//! repair that keeps its description without an edit; and two runs that agree byte for byte.
//! They need Z3 at the version of `verification.lock` on the path (`compiler/testing.md §3`).

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

/// The capability every case uses.
const LOG: &str = "module m\n\npub capability Log\n  op write(n: Int) -> Unit\n";

/// What (h) adds to the description of a repair whose edit it withholds.
const WITHHELD: &str = "; not applied while an obligation of this package is unknown (U5002)";

/// A package of edition 0.3 named `name`, with the repository's lock and the files `files`.
fn package(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("effect-edits")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        "[package]\nname = \"c\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n",
    )
    .unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    for (path, text) in files {
        std::fs::write(root.join(path), text).unwrap();
    }
    root
}

/// A package whose only module is `src/m.una`, holding [`LOG`] and then `rest`.
fn module(name: &str, rest: &str) -> PathBuf {
    package(name, &[("src/m.una", &format!("{LOG}\n{rest}"))])
}

/// Runs `una` with `args` in `root`, and parses its JSON output.
fn una(root: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// `una fix` at tier 1, which writes.
fn fix(root: &Path) -> Value {
    una(root, &["--json", "fix", "."])
}

/// The diagnostics of `una check` at tier 1.
fn check(root: &Path) -> Vec<Value> {
    una(root, &["--json", "check", "."])["diagnostics"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn read(root: &Path, path: &str) -> String {
    std::fs::read_to_string(root.join(path)).unwrap()
}

/// The `uses` line of the declaration `name` in `text`.
fn uses_of<'t>(text: &'t str, name: &str) -> &'t str {
    let mut lines = text.lines().skip_while(|line| {
        !line.starts_with(&format!("{name}("))
            && !line.starts_with(&format!("pub {name}("))
            && !line.contains(&format!("handler {name}("))
    });
    lines
        .next()
        .unwrap_or_else(|| panic!("no `{name}` in {text}"));
    lines
        .find(|line| line.trim_start().starts_with("uses "))
        .unwrap_or_else(|| panic!("no `uses` of `{name}` in {text}"))
        .trim()
}

/// The applied edits of a `una.fix/1` document, as (round, code).
fn applied(document: &Value) -> Vec<(u64, String)> {
    document["applied"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["round"].as_u64().unwrap(),
                a["code"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// The first repair of the `effect_condition` diagnostic of `root`, which must be the only
/// one.
fn effect_repair(root: &Path) -> Value {
    let found = check(root);
    let effects: Vec<&Value> = found
        .iter()
        .filter(|d| d["obligation"]["kind"] == "effect_condition")
        .collect();
    assert_eq!(effects.len(), 1, "{found:#?}");
    effects[0]["repairs"][0].clone()
}

/// The first repair of the case's one `effect_condition` diagnostic keeps its description
/// and carries no edit, and `una fix` writes nothing.
fn refused(name: &str, root: &Path) {
    let before = read(root, "src/m.una");
    let repair = effect_repair(root);
    assert!(repair["edit"].is_null(), "{name}: an edit: {repair:#?}");
    assert!(
        repair["description"]
            .as_str()
            .unwrap()
            .starts_with("write the condition in the `uses` of"),
        "{name}: {repair:#?}"
    );
    let document = fix(root);
    assert_eq!(applied(&document), Vec::new(), "{name}: {document:#?}");
    assert_eq!(read(root, "src/m.una"), before, "{name}");
}

/// The callee that the cases call: it guards the operation itself, so its condition holds
/// once it is written.
const NOTE: &str = "note(n: Int, quiet: Bool) -> Unit
  uses @log: Log[write]
=
  if not quiet
    @log.write(n)
  unit
";

#[test]
fn the_condition_is_written_on_the_callee_at_a_call() {
    let root = module(
        "call",
        &format!(
            "{NOTE}
pub run(n: Int, dry: Bool) -> Unit
  uses @log: Log[write where not dry]
=
  note(n, quiet: dry)
"
        ),
    );
    let before = check(&root);
    assert_eq!(before.len(), 1, "{before:#?}");
    assert_eq!(before[0]["code"], "U5001");
    let repair = &before[0]["repairs"][0];
    assert_eq!(
        repair["edit"]["replacement"], " where not quiet",
        "{repair:#?}"
    );
    let document = fix(&root);
    assert_eq!(applied(&document), vec![(1, "U5001".to_owned())]);
    assert_eq!(document["stopped"], "done");
    assert_eq!(document["skipped"], Value::Array(Vec::new()));
    let text = read(&root, "src/m.una");
    assert_eq!(
        uses_of(&text, "note"),
        "uses @log: Log[write where not quiet]"
    );
    assert_eq!(uses_of(&text, "run"), "uses @log: Log[write where not dry]");
    assert_eq!(check(&root), Vec::<Value>::new());
}

#[test]
fn the_condition_is_written_on_the_handler_at_a_with() {
    let root = module(
        "with",
        "pub capability Store
  op put(n: Int) -> Unit

handler forward(quiet: Bool) for Store
  uses @log: Log[write]
  op put(n) = @log.write(n)

save(n: Int) -> Unit
  uses @store: Store[put]
=
  @store.put(n)

pub run(n: Int, dry: Bool) -> Unit
  uses @log: Log[write where not dry]
=
  save(n) with @store = forward(quiet: dry)
",
    );
    let document = fix(&root);
    assert_eq!(
        applied(&document)[0],
        (1, "U5001".to_owned()),
        "{document:#?}"
    );
    let text = read(&root, "src/m.una");
    assert_eq!(
        uses_of(&text, "forward"),
        "uses @log: Log[write where not quiet]"
    );
    // The handler's own operation now has to prove it: the diagnostic moves into the handler.
    let after = check(&root);
    assert_eq!(after.len(), 1, "{after:#?}");
    assert_eq!(after[0]["code"], "U5001");
    assert_eq!(
        after[0]["message"],
        "`@log.write` may run where `not quiet` is not proven"
    );
}

#[test]
fn a_renamed_binding_writes_on_the_callee_binding_that_receives_it() {
    let root = module(
        "renamed",
        &format!(
            "{NOTE}
pub run(n: Int, dry: Bool) -> Unit
  uses @audit: Log[write where not dry]
=
  note(n, quiet: dry) with @log = @audit
"
        ),
    );
    let document = fix(&root);
    assert_eq!(
        applied(&document),
        vec![(1, "U5001".to_owned())],
        "{document:#?}"
    );
    let text = read(&root, "src/m.una");
    assert_eq!(
        uses_of(&text, "note"),
        "uses @log: Log[write where not quiet]"
    );
    assert_eq!(check(&root), Vec::<Value>::new());
}

#[test]
fn the_replacement_is_simultaneous() {
    // `C = a and not b`, with `b` passed as `x` and `a` as `y`: `C″ = y and not x`.
    let root = module(
        "swap",
        "both(x: Bool, y: Bool, n: Int) -> Unit
  uses @log: Log[write]
=
  if y and not x
    @log.write(n)
  unit

pub run(a: Bool, b: Bool, n: Int) -> Unit
  uses @log: Log[write where a and not b]
=
  both(b, y: a, n: n)
",
    );
    fix(&root);
    let text = read(&root, "src/m.una");
    assert_eq!(
        uses_of(&text, "both"),
        "uses @log: Log[write where y and not x]"
    );
    assert_eq!(check(&root), Vec::<Value>::new());
}

#[test]
fn a_condition_already_written_is_kept_and_conjoined() {
    // Both operands hold `or`; the parentheses keep the conjunction.
    let root = module(
        "conjoined",
        "note(n: Int, quiet: Bool, loud: Bool) -> Unit
  uses @log: Log[write where loud or n > 0]
=
  if (loud or n > 0) and (not quiet or n > 10)
    @log.write(n)
  unit

pub run(n: Int, dry: Bool) -> Unit
  uses @log: Log[write where not dry or n > 10]
=
  note(n, quiet: dry, loud: true)
",
    );
    let document = fix(&root);
    assert_eq!(
        applied(&document),
        vec![(1, "U5001".to_owned())],
        "{document:#?}"
    );
    let text = read(&root, "src/m.una");
    assert_eq!(
        uses_of(&text, "note"),
        "uses @log: Log[write where (loud or n > 0) and (not quiet or n > 10)]"
    );
    assert_eq!(check(&root), Vec::<Value>::new());
}

/// A chain: `run` calls `outer`, which calls `inner`, which guards.
const CHAIN: &str = "inner(n: Int, q: Bool) -> Unit
  uses @log: Log[write]
=
  if not q
    @log.write(n)
  unit

outer(n: Int, quiet: Bool) -> Unit
  uses @log: Log[write]
=
  inner(n, q: quiet)

pub run(n: Int, dry: Bool) -> Unit
  uses @log: Log[write where not dry]
=
  outer(n, quiet: dry)
";

#[test]
fn a_diagnostic_moved_into_the_callee_is_repaired_one_call_further_down() {
    let root = module("chain", CHAIN);
    let document = fix(&root);
    assert_eq!(
        applied(&document),
        vec![(1, "U5001".to_owned()), (2, "U5001".to_owned())],
        "{document:#?}"
    );
    let text = read(&root, "src/m.una");
    assert_eq!(
        uses_of(&text, "outer"),
        "uses @log: Log[write where not quiet]"
    );
    assert_eq!(uses_of(&text, "inner"), "uses @log: Log[write where not q]");
    assert_eq!(check(&root), Vec::<Value>::new());
}

#[test]
fn two_sites_asking_one_callee_conjoin_over_two_rounds() {
    let root = module(
        "two-sites",
        "note(n: Int, quiet: Bool) -> Unit
  uses @log: Log[write]
=
  if not quiet and n > 5
    @log.write(n)
  unit

pub first(n: Int, dry: Bool) -> Unit
  uses @log: Log[write where not dry]
=
  note(n, quiet: dry)

pub second(n: Int) -> Unit
  uses @log: Log[write where n > 5]
=
  note(n, quiet: false)
",
    );
    // `fixable` counts both candidates of the first round (ADR 0096 rule 6).
    let checked = una(&root, &["--json", "check", "."]);
    assert_eq!(checked["fixable"], 2, "{checked:#?}");
    let document = fix(&root);
    assert_eq!(
        applied(&document),
        vec![(1, "U5001".to_owned()), (2, "U5001".to_owned())],
        "{document:#?}"
    );
    let text = read(&root, "src/m.una");
    assert_eq!(
        uses_of(&text, "note"),
        "uses @log: Log[write where (not quiet) and (n > 5)]"
    );
    assert_eq!(check(&root), Vec::<Value>::new());
}

#[test]
fn tier_zero_applies_no_tier_one_edit() {
    let source = format!(
        "{LOG}\n{NOTE}
pub run(n: Int, dry: Bool) -> Unit
  uses @log: Log[write where not dry]
=
  note(n, quiet: dry)
"
    );
    let root = package("tier-zero", &[("src/m.una", &source)]);
    let document = una(&root, &["--json", "fix", "--tier", "0", "."]);
    assert_eq!(applied(&document), Vec::new(), "{document:#?}");
    assert_eq!(document["rounds"], 0);
    assert_eq!(read(&root, "src/m.una"), source);
}

#[test]
fn a_tier_zero_edit_comes_first_and_the_condition_in_a_later_round() {
    // The unlabeled second argument is U2004, whose edit labels it; only then is tier 0 clean.
    let root = module(
        "tier-zero-first",
        &format!(
            "{NOTE}
pub run(n: Int, dry: Bool) -> Unit
  uses @log: Log[write where not dry]
=
  note(n, dry)
"
        ),
    );
    let document = fix(&root);
    assert_eq!(
        applied(&document),
        vec![(1, "U2004".to_owned()), (2, "U5001".to_owned())],
        "{document:#?}"
    );
    let text = read(&root, "src/m.una");
    assert_eq!(
        uses_of(&text, "note"),
        "uses @log: Log[write where not quiet]"
    );
    assert_eq!(check(&root), Vec::<Value>::new());
}

#[test]
fn a_is_refused_for_an_argument_that_is_not_the_parameter() {
    let root = module(
        "a-expression",
        &format!(
            "{NOTE}
pub run(n: Int, dry: Bool) -> Unit
  uses @log: Log[write where not dry]
=
  note(n, quiet: dry and dry)
"
        ),
    );
    refused("(a), an expression", &root);
}

#[test]
fn a_is_refused_for_an_edit_parameter() {
    let root = module(
        "a-edit",
        &format!(
            "{NOTE}
pub run(n: Int, edit dry: Bool) -> Unit
  uses @log: Log[write where not dry]
=
  dry = false
  note(n, quiet: dry)
"
        ),
    );
    refused("(a), an `edit` parameter", &root);
}

#[test]
fn a_is_refused_for_a_parameter_not_passed() {
    let root = module(
        "a-not-passed",
        &format!(
            "{NOTE}
pub run(n: Int, dry: Bool) -> Unit
  uses @log: Log[write where not dry]
=
  note(n, quiet: false)
"
        ),
    );
    refused("(a), not passed", &root);
}

#[test]
fn a_lambda_parameter_cannot_take_the_parameter_s_name() {
    // What keeps (a) from mistaking a lambda's parameter for the caller's: a local cannot
    // rebind a name already bound (U1002), so an argument named like a parameter is it.
    let root = module(
        "a-lambda",
        &format!(
            "{NOTE}
pub run(flags: List[Bool], dry: Bool) -> List[Unit]
  uses @log: Log[write where not dry]
=
  flags.map(each: fn(dry) -> note(1, quiet: dry))
"
        ),
    );
    let found = check(&root);
    let codes: Vec<&str> = found.iter().map(|d| d["code"].as_str().unwrap()).collect();
    assert_eq!(codes, vec!["U1002"], "{found:#?}");
}

#[test]
fn b_is_refused_for_a_callee_of_another_module() {
    let root = package(
        "b-module",
        &[
            (
                "src/caps.una",
                "module caps\n\npub capability Log\n  op write(n: Int) -> Unit\n",
            ),
            (
                "src/notes.una",
                "module notes\n\nimport caps (Log)\n\npub note(n: Int, quiet: Bool) -> Unit
  uses @log: Log[write]
=
  if not quiet
    @log.write(n)
  unit
",
            ),
            (
                "src/m.una",
                "module m\n\nimport caps (Log)\nimport notes (note)\n
pub run(n: Int, dry: Bool) -> Unit
  uses @log: Log[write where not dry]
=
  note(n, quiet: dry)
",
            ),
        ],
    );
    refused("(b), another module", &root);
}

#[test]
fn b_is_refused_for_inferred_uses() {
    let root = module(
        "b-inferred",
        "note(n: Int, quiet: Bool) -> Unit
=
  if not quiet
    @log.write(n)
  unit

pub run(n: Int, dry: Bool) -> Unit
  uses @log: Log[write where not dry]
=
  note(n, quiet: dry)
",
    );
    refused("(b), inferred `uses`", &root);
}

#[test]
fn b_is_refused_for_a_binding_without_brackets() {
    let root = module(
        "b-brackets",
        "note(n: Int, quiet: Bool) -> Unit
  uses @log: Log
=
  if not quiet
    @log.write(n)
  unit

pub run(n: Int, dry: Bool) -> Unit
  uses @log: Log[write where not dry]
=
  note(n, quiet: dry)
",
    );
    refused("(b), no brackets", &root);
}

#[test]
fn c_is_refused_when_a_callee_parameter_has_a_name_of_the_condition() {
    // `C = small(n) and not dry`: `note`'s parameter `small` would capture the predicate.
    let root = module(
        "c-capture",
        "predicate reflect small(k: Int) -> Bool = k < 100

note(n: Int, small: Bool) -> Unit
  uses @log: Log[write]
=
  if not small
    @log.write(n)
  unit

pub run(n: Int, dry: Bool) -> Unit
  uses @log: Log[write where small(n) and not dry]
=
  note(n, small: dry)
",
    );
    refused("(c), a captured name", &root);
}

#[test]
fn d_is_refused_for_a_public_callee() {
    let root = module(
        "d-public",
        &format!(
            "pub {NOTE}
pub run(n: Int, dry: Bool) -> Unit
  uses @log: Log[write where not dry]
=
  note(n, quiet: dry)
"
        ),
    );
    refused("(d), `pub`", &root);
}

#[test]
fn d_is_refused_for_a_callee_used_as_a_value() {
    let root = module(
        "d-value",
        &format!(
            "{NOTE}
apply(f: fn(Int, Bool) -> Unit uses @log: Log[write], n: Int) -> Unit
  uses @log: Log[write]
=
  f(n, true)

pub elsewhere(n: Int) -> Unit
  uses @log: Log[write]
=
  apply(note, n: n)

pub run(n: Int, dry: Bool) -> Unit
  uses @log: Log[write where not dry]
=
  note(n, quiet: dry)
"
        ),
    );
    refused("(d), a function value", &root);
}

/// The chain of [`CHAIN`], with `step_budget = 1` so that every obligation is unknown.
fn unknown(name: &str) -> PathBuf {
    let root = module(name, CHAIN);
    let lock = read(&root, "verification.lock");
    let lock = lock.replace("step_budget      = 5000000", "step_budget      = 1");
    assert!(lock.contains("step_budget      = 1\n"), "{lock}");
    std::fs::write(root.join("verification.lock"), lock).unwrap();
    root
}

#[test]
fn h_withholds_every_edit_while_an_obligation_is_unknown_and_says_why() {
    for run in ["first", "second"] {
        let root = unknown(&format!("h-unknown-{run}"));
        let before = read(&root, "src/m.una");
        let repair = effect_repair(&root);
        assert!(repair["edit"].is_null(), "{repair:#?}");
        assert!(
            repair["description"].as_str().unwrap().ends_with(WITHHELD),
            "{repair:#?}"
        );
        let document = fix(&root);
        assert_eq!(applied(&document), Vec::new(), "{document:#?}");
        assert_eq!(document["skipped"], Value::Array(Vec::new()));
        assert_eq!(document["stopped"], "done");
        assert_eq!(read(&root, "src/m.una"), before);
    }
}

#[test]
fn two_runs_give_the_same_output_and_the_same_sources() {
    let first = module("twice-a", CHAIN);
    let second = module("twice-b", CHAIN);
    assert_eq!(fix(&first), fix(&second));
    assert_eq!(read(&first, "src/m.una"), read(&second, "src/m.una"));
}
