//! `una fix` restores a locked condition in the header of a locked function (ADR 0092 and ADR
//! 0084, amendments of 2026-10-15): for each case, the source before, the source `una fix`
//! leaves, a check of it, and `una.toml` unchanged byte for byte; for conditions (a), (b) and
//! (e) when they fail, a first repair that keeps its description without an edit; and two runs
//! that agree byte for byte. (c) and (d) cannot fail on a checked program, so unit tests pin
//! them (`una-verify`, `una-check`). They need Z3 at the version of `verification.lock` on the path
//! (`compiler/testing.md §3`).

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

/// The capability every case uses, in the module `shop`.
const CATALOG: &str = "module shop

pub capability Catalog
  op price(sku: Int) -> Int
  op set_price(sku: Int, cents: Int) -> Unit
";

/// What (e) adds to the description of a repair whose edit it withholds.
const WITHHELD: &str = "; not applied while an obligation of this package is unknown (U5002)";

/// The description of the first repair of a refuted `contract_lock` obligation.
const RESTORE: &str = "restore the locked condition: write `set_price where";

/// A package of edition 0.3 named `name`, whose `[contracts]` table is `contracts` and whose
/// module `src/shop.una` is [`CATALOG`] and then `rest`.
fn package(name: &str, contracts: &str, rest: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("lock-edits")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        format!(
            "[package]\nname = \"c\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n\n[contracts]\n{contracts}\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(root.join("src/shop.una"), format!("{CATALOG}\n{rest}")).unwrap();
    root
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

/// The `uses` line of the function `update` in `text`.
fn uses_of_update(text: &str) -> &str {
    text.lines()
        .skip_while(|line| !line.starts_with("pub update("))
        .find(|line| line.trim_start().starts_with("uses "))
        .unwrap_or_else(|| panic!("no `uses` of `update` in {text}"))
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

/// The diagnostics of `root` with the obligation kind `contract_lock`.
fn locks(root: &Path) -> Vec<Value> {
    check(root)
        .into_iter()
        .filter(|d| d["obligation"]["kind"] == "contract_lock")
        .collect()
}

/// `una fix` restores the header of `root`: one U3023 applied in the first round, `stopped`
/// `done`, the header's `uses` line `expected`, `una.toml` unchanged, and a clean check.
fn restored(root: &Path, expected: &str) {
    let toml = read(root, "una.toml");
    let before = locks(root);
    assert_eq!(before.len(), 1, "{before:#?}");
    assert_eq!(before[0]["code"], "U3023");
    assert!(
        before[0]["repairs"][0]["description"]
            .as_str()
            .unwrap()
            .starts_with(RESTORE),
        "{before:#?}"
    );
    assert!(!before[0]["repairs"][0]["edit"].is_null(), "{before:#?}");
    let document = fix(root);
    assert_eq!(
        applied(&document),
        vec![(1, "U3023".to_owned())],
        "{document:#?}"
    );
    assert_eq!(document["stopped"], "done");
    assert_eq!(document["skipped"], Value::Array(Vec::new()));
    assert_eq!(uses_of_update(&read(root, "src/shop.una")), expected);
    assert_eq!(read(root, "una.toml"), toml);
    assert_eq!(check(root), Vec::<Value>::new());
}

/// The case's one `contract_lock` diagnostic is U3023 with the restoring repair as a
/// description and no edit, and `una fix` writes nothing.
fn refused(name: &str, root: &Path) {
    let before = read(root, "src/shop.una");
    let found = locks(root);
    assert_eq!(found.len(), 1, "{name}: {found:#?}");
    assert_eq!(found[0]["code"], "U3023", "{name}: {found:#?}");
    let repair = &found[0]["repairs"][0];
    assert!(repair["edit"].is_null(), "{name}: an edit: {repair:#?}");
    assert!(
        repair["description"].as_str().unwrap().starts_with(RESTORE),
        "{name}: {repair:#?}"
    );
    let document = fix(root);
    assert_eq!(applied(&document), Vec::new(), "{name}: {document:#?}");
    assert_eq!(read(root, "src/shop.una"), before, "{name}");
}

/// The lock that most cases use.
const LOCK: &str = r#""shop.update" = ["@catalog: Catalog[price, set_price where not dry_run]"]"#;

/// A body that performs `set_price` only where `dry_run` is false.
const GUARDED: &str = "=
  if not dry_run
    @catalog.set_price(sku, cents)
  unit
";

#[test]
fn an_operation_without_a_condition_gets_the_locked_one() {
    let root = package(
        "no-condition",
        LOCK,
        &format!(
            "pub update(sku: Int, cents: Int, dry_run: Bool) -> Unit\n  uses @catalog: Catalog[price, set_price]\n{GUARDED}"
        ),
    );
    let repair = &locks(&root)[0]["repairs"][0];
    assert_eq!(
        repair["edit"]["replacement"], " where not dry_run",
        "{repair:#?}"
    );
    restored(
        &root,
        "uses @catalog: Catalog[price, set_price where not dry_run]",
    );
}

#[test]
fn a_weaker_condition_is_kept_and_conjoined() {
    // The header's condition holds `or`; the parentheses keep the conjunction.
    let root = package(
        "conjoined",
        LOCK,
        "pub update(sku: Int, cents: Int, dry_run: Bool) -> Unit
  uses @catalog: Catalog[price, set_price where cents > 0 or sku > 0]
=
  if (cents > 0 or sku > 0) and not dry_run
    @catalog.set_price(sku, cents)
  unit
",
    );
    restored(
        &root,
        "uses @catalog: Catalog[price, set_price where (cents > 0 or sku > 0) and (not dry_run)]",
    );
}

#[test]
fn an_escaped_text_literal_is_written_as_its_string_reads() {
    let root = package(
        "escaped",
        r#""shop.update" = ["@catalog: Catalog[set_price where zone == \"eu\"]"]"#,
        "pub update(sku: Int, cents: Int, zone: Text) -> Unit
  uses @catalog: Catalog[set_price]
=
  if zone == \"eu\"
    @catalog.set_price(sku, cents)
  unit
",
    );
    restored(
        &root,
        "uses @catalog: Catalog[set_price where zone == \"eu\"]",
    );
}

#[test]
fn a_line_separator_in_a_text_literal_stays_on_the_line() {
    // U+2028 is not a line break to the lexer: written back unchanged, on one line.
    let root = package(
        "line-separator",
        r#""shop.update" = ["@catalog: Catalog[set_price where zone == \"a b\"]"]"#,
        "pub update(sku: Int, cents: Int, zone: Text) -> Unit
  uses @catalog: Catalog[set_price]
=
  if zone == \"a\u{2028}b\"
    @catalog.set_price(sku, cents)
  unit
",
    );
    restored(
        &root,
        "uses @catalog: Catalog[set_price where zone == \"a\u{2028}b\"]",
    );
}

#[test]
fn a_parameter_named_like_a_function_resolves_to_the_parameter() {
    // Hygiene: the lock and the header resolve `ready` in the same scope, to the parameter.
    let root = package(
        "hygiene",
        r#""shop.update" = ["@catalog: Catalog[set_price where ready]"]"#,
        "ready(n: Int) -> Bool = n > 0

pub update(sku: Int, cents: Int, ready: Bool) -> Unit
  uses @catalog: Catalog[set_price]
=
  if ready
    @catalog.set_price(sku, cents)
  unit
",
    );
    restored(&root, "uses @catalog: Catalog[set_price where ready]");
}

#[test]
fn a_is_refused_for_a_binding_without_brackets() {
    // The lock lists every operation, so the bracketless header grants nothing more.
    let root = package(
        "a-bracketless",
        LOCK,
        &format!(
            "pub update(sku: Int, cents: Int, dry_run: Bool) -> Unit\n  uses @catalog: Catalog\n{GUARDED}"
        ),
    );
    refused("(a), no brackets", &root);
}

#[test]
fn b_is_refused_for_a_condition_on_two_lines() {
    let root = package(
        "b-two-lines",
        "\"shop.update\" = [\"@catalog: Catalog[price, set_price where (not\\n dry_run)]\"]",
        &format!(
            "pub update(sku: Int, cents: Int, dry_run: Bool) -> Unit\n  uses @catalog: Catalog[price, set_price]\n{GUARDED}"
        ),
    );
    refused("(b), two lines", &root);
}

#[test]
fn a_comment_after_the_lock_condition_is_u7002_and_nothing_is_applied() {
    let root = package(
        "comment",
        r#""shop.update" = ["@catalog: Catalog[price, set_price where not dry_run -- x]"]"#,
        &format!(
            "pub update(sku: Int, cents: Int, dry_run: Bool) -> Unit\n  uses @catalog: Catalog[price, set_price]\n{GUARDED}"
        ),
    );
    // The entry does not parse, so it locks nothing: the header's binding is one that no
    // entry names, a U3023 of tier 0, with no obligation and no edit.
    let before = read(&root, "src/shop.una");
    let found = check(&root);
    let codes: Vec<&str> = found.iter().map(|d| d["code"].as_str().unwrap()).collect();
    assert_eq!(codes, vec!["U3023", "U7002"], "{found:#?}");
    assert!(
        found.iter().all(|d| d["obligation"].is_null()),
        "{found:#?}"
    );
    assert_eq!(applied(&fix(&root)), Vec::new());
    assert_eq!(read(&root, "src/shop.una"), before);
}

#[test]
fn a_parameter_renamed_since_the_lock_is_u7002_and_nothing_is_applied() {
    let root = package(
        "renamed-parameter",
        LOCK,
        "pub update(sku: Int, cents: Int, dry: Bool) -> Unit
  uses @catalog: Catalog[price, set_price]
=
  if not dry
    @catalog.set_price(sku, cents)
  unit
",
    );
    let before = read(&root, "src/shop.una");
    let found = check(&root);
    let codes: Vec<&str> = found.iter().map(|d| d["code"].as_str().unwrap()).collect();
    assert_eq!(codes, vec!["U7002"], "{found:#?}");
    assert!(
        found[0]["message"].as_str().unwrap().contains("dry_run"),
        "{found:#?}"
    );
    assert_eq!(applied(&fix(&root)), Vec::new());
    assert_eq!(read(&root, "src/shop.una"), before);
}

#[test]
fn an_operation_the_restored_condition_does_not_cover_is_reported_next() {
    // The body performs `set_price` unguarded: once the header is restored, that operation is
    // refuted in the body, whose repair is a guard, a description only.
    let root = package(
        "body",
        LOCK,
        "pub update(sku: Int, cents: Int, dry_run: Bool) -> Unit
  uses @catalog: Catalog[price, set_price]
=
  @catalog.set_price(sku, cents)
",
    );
    let document = fix(&root);
    assert_eq!(
        applied(&document),
        vec![(1, "U3023".to_owned())],
        "{document:#?}"
    );
    assert_eq!(document["stopped"], "done");
    assert_eq!(
        uses_of_update(&read(&root, "src/shop.una")),
        "uses @catalog: Catalog[price, set_price where not dry_run]"
    );
    let after = check(&root);
    assert_eq!(after.len(), 1, "{after:#?}");
    assert_eq!(after[0]["code"], "U5001");
    assert_eq!(after[0]["obligation"]["kind"], "effect_condition");
    assert!(after[0]["repairs"][0]["edit"].is_null(), "{after:#?}");
}

#[test]
fn tier_zero_applies_no_restoration() {
    let root = package(
        "tier-zero",
        LOCK,
        &format!(
            "pub update(sku: Int, cents: Int, dry_run: Bool) -> Unit\n  uses @catalog: Catalog[price, set_price]\n{GUARDED}"
        ),
    );
    let before = read(&root, "src/shop.una");
    let document = una(&root, &["--json", "fix", "--tier", "0", "."]);
    assert_eq!(applied(&document), Vec::new(), "{document:#?}");
    assert_eq!(document["rounds"], 0);
    assert_eq!(read(&root, "src/shop.una"), before);
}

#[test]
fn a_tier_zero_edit_comes_first_and_the_restoration_in_a_later_round() {
    // The unlabeled second argument is U2004, whose edit labels it; only then is tier 0 clean.
    let root = package(
        "tier-zero-first",
        LOCK,
        "note(n: Int, quiet: Bool) -> Unit = unit

pub update(sku: Int, cents: Int, dry_run: Bool) -> Unit
  uses @catalog: Catalog[price, set_price]
=
  note(sku, dry_run)
  if not dry_run
    @catalog.set_price(sku, cents)
  unit
",
    );
    let document = fix(&root);
    assert_eq!(
        applied(&document),
        vec![(1, "U2004".to_owned()), (2, "U3023".to_owned())],
        "{document:#?}"
    );
    assert_eq!(
        uses_of_update(&read(&root, "src/shop.una")),
        "uses @catalog: Catalog[price, set_price where not dry_run]"
    );
    assert_eq!(check(&root), Vec::<Value>::new());
}

#[test]
fn e_withholds_the_restoration_while_an_obligation_is_unknown_and_says_why() {
    for run in ["first", "second"] {
        let root = package(
            &format!("e-unknown-{run}"),
            LOCK,
            &format!(
                "pub update(sku: Int, cents: Int, dry_run: Bool) -> Unit\n  uses @catalog: Catalog[price, set_price]\n{GUARDED}"
            ),
        );
        let lock = read(&root, "verification.lock");
        let lock = lock.replace("step_budget      = 5000000", "step_budget      = 1");
        assert!(lock.contains("step_budget      = 1\n"), "{lock}");
        std::fs::write(root.join("verification.lock"), lock).unwrap();
        let before = read(&root, "src/shop.una");
        let found = locks(&root);
        assert_eq!(found.len(), 1, "{found:#?}");
        let repair = &found[0]["repairs"][0];
        assert!(repair["edit"].is_null(), "{repair:#?}");
        assert!(
            repair["description"].as_str().unwrap().ends_with(WITHHELD),
            "{repair:#?}"
        );
        let document = fix(&root);
        assert_eq!(applied(&document), Vec::new(), "{document:#?}");
        assert_eq!(document["skipped"], Value::Array(Vec::new()));
        assert_eq!(document["stopped"], "done");
        assert_eq!(read(&root, "src/shop.una"), before);
    }
}

#[test]
fn a_restorable_lock_counts_as_fixable() {
    let root = package(
        "fixable",
        LOCK,
        &format!(
            "pub update(sku: Int, cents: Int, dry_run: Bool) -> Unit\n  uses @catalog: Catalog[price, set_price]\n{GUARDED}"
        ),
    );
    let document = una(&root, &["--json", "check", "."]);
    assert_eq!(document["fixable"], 1, "{document:#?}");
}

#[test]
fn a_restoration_and_a_condition_on_a_callee_are_taken_in_one_round() {
    // The locked function asks its callee `note` for `not quiet` (ADR 0092, amendment of
    // 2026-10-14) and has lost its locked condition on `set_price`: both edits are tier-1
    // candidates of the first round, in different ranges, and the second round has none.
    let root = package(
        "with-callee",
        r#""shop.update" = [
  "@catalog: Catalog[price, set_price where not dry_run]",
  "@log: Log[write where not dry_run]",
]"#,
        "pub capability Log
  op write(n: Int) -> Unit

note(n: Int, quiet: Bool) -> Unit
  uses @log: Log[write]
=
  if not quiet
    @log.write(n)
  unit

pub update(sku: Int, cents: Int, dry_run: Bool) -> Unit
  uses @catalog: Catalog[price, set_price]
  uses @log: Log[write where not dry_run]
=
  note(sku, quiet: dry_run)
  if not dry_run
    @catalog.set_price(sku, cents)
  unit
",
    );
    let toml = read(&root, "una.toml");
    let document = fix(&root);
    assert_eq!(
        applied(&document),
        vec![(1, "U5001".to_owned()), (1, "U3023".to_owned())],
        "{document:#?}"
    );
    assert_eq!(document["rounds"], 1);
    assert_eq!(document["stopped"], "done");
    let text = read(&root, "src/shop.una");
    assert!(
        text.contains("  uses @log: Log[write where not quiet]\n"),
        "{text}"
    );
    assert_eq!(
        uses_of_update(&text),
        "uses @catalog: Catalog[price, set_price where not dry_run]"
    );
    assert_eq!(read(&root, "una.toml"), toml);
    assert_eq!(check(&root), Vec::<Value>::new());
}

#[test]
fn a_locked_callee_asked_for_the_same_condition_gets_only_the_restoration() {
    // `run` asks `update` for `not dry_run` on the operation that `update`'s lock conditions.
    // A locked function is public, so the repair at the call stays a description (condition
    // (d) of 2026-10-14) and only the restoration edits the header's operation. Once it is
    // restored, the call's obligation holds too.
    let root = package(
        "same-operation",
        LOCK,
        "pub update(sku: Int, cents: Int, dry_run: Bool) -> Unit
  uses @catalog: Catalog[price, set_price]
=
  if not dry_run
    @catalog.set_price(sku, cents)
  unit

pub run(sku: Int, dry: Bool) -> Unit
  uses @catalog: Catalog[price, set_price where not dry]
=
  update(sku, cents: 1, dry_run: dry)
",
    );
    let before = check(&root);
    let call: Vec<&Value> = before
        .iter()
        .filter(|d| d["obligation"]["kind"] == "effect_condition")
        .collect();
    assert_eq!(call.len(), 1, "{before:#?}");
    assert!(call[0]["repairs"][0]["edit"].is_null(), "{before:#?}");
    let document = fix(&root);
    assert_eq!(
        applied(&document),
        vec![(1, "U3023".to_owned())],
        "{document:#?}"
    );
    assert_eq!(document["stopped"], "done");
    assert_eq!(
        uses_of_update(&read(&root, "src/shop.una")),
        "uses @catalog: Catalog[price, set_price where not dry_run]"
    );
    assert_eq!(check(&root), Vec::<Value>::new());
}

#[test]
fn two_runs_give_the_same_output_and_the_same_sources() {
    let source = "pub update(sku: Int, cents: Int, dry_run: Bool) -> Unit
  uses @catalog: Catalog[price, set_price where cents > 0]
=
  @catalog.set_price(sku, cents)
";
    let first = package("twice-a", LOCK, source);
    let second = package("twice-b", LOCK, source);
    assert_eq!(fix(&first), fix(&second));
    assert_eq!(read(&first, "src/shop.una"), read(&second, "src/shop.una"));
}
