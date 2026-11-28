//! Repairs that narrow before they widen (ADR 0093 rule 6, #1397): the first repair of a U3020
//! grants the callee's requirement narrowed to what it declares, and only within the
//! boundary's ceiling; `una fix` applies that grant and nothing beyond it, never writes the
//! ceiling, and never applies an edit of U3021 or U3024 (ADR 0084). U3021's repair narrows to the
//! ceiling or says that no grant of a binding keeps both, and names a handler within the ceiling. In edition 0.3, U3001 and U3002 rank a narrowed
//! rebinding first and a widening of a public header last, marked as an authority change.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pretty_assertions::assert_eq;
use serde_json::Value;

/// The conformance directory, or the case at `case` under it.
fn conformance(case: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/conformance")
        .join(case)
}

fn una(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

/// The JSON document on the stdout of `output`; a run that wrote none fails with its status and
/// stderr.
fn json_of(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "no JSON document ({error}): {:?}, stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// Every file under `root`, relative to it, with its bytes.
fn files_of(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(root.join(&dir)).unwrap() {
            let entry = entry.unwrap();
            let relative = dir.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                pending.push(relative);
            } else {
                out.insert(
                    relative.clone(),
                    std::fs::read(root.join(&relative)).unwrap(),
                );
            }
        }
    }
    out
}

/// A fresh copy of a case for `una fix` to write, removed when dropped. It derefs to its root.
struct CaseCopy(PathBuf);

impl std::ops::Deref for CaseCopy {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for CaseCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A fresh copy of the case at `case`, for `una fix` to write: a directory of its own for each
/// call, named by the process and a count, since the tests run at once in many processes and
/// several copy the same case (#2334: a copy shared by name was removed under another test's
/// `una check`, which then printed nothing).
fn copy_of(case: &str) -> CaseCopy {
    static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let copy = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("boundary-repairs")
        .join(format!(
            "{}-{}-{n}",
            case.replace('/', "-"),
            std::process::id()
        ));
    let _ = std::fs::remove_dir_all(&copy);
    for (file, bytes) in files_of(&conformance(case)) {
        let target = copy.join(&file);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, bytes).unwrap();
    }
    CaseCopy(copy)
}

/// The diagnostics of `una --json check` on the case at `case`.
fn check_json(case: &str) -> Vec<Value> {
    let output = una(&conformance(case), &["--json", "check", "--tier", "0", "."]);
    let document = json_of(&output);
    document["diagnostics"].as_array().cloned().unwrap()
}

/// The `una.fix/1` document of `una --json fix --tier 0` on `root`.
fn fix(root: &Path) -> Value {
    let output = una(root, &["--json", "fix", "--tier", "0", "."]);
    let document = json_of(&output);
    assert_eq!(document["schema"], "una.fix/1", "{document}");
    document
}

/// The diagnostics with `code` at `line`:`col`.
fn at(diagnostics: &[Value], code: &str, line: u64, col: u64) -> Vec<Value> {
    diagnostics
        .iter()
        .filter(|d| {
            d["code"] == code
                && d["primary"]["start"]["line"] == line
                && d["primary"]["start"]["col"] == col
        })
        .cloned()
        .collect()
}

/// Each repair as (widening kind or "", description, edit replacement or ""), in output order.
fn rows(diagnostic: &Value) -> Vec<Row> {
    diagnostic["repairs"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(index, repair)| {
            assert_eq!(repair["rank"], index + 1);
            assert_eq!(repair["escape_cost"], 0, "{repair}");
            (
                repair["widens"]["kind"].as_str().unwrap_or("").to_owned(),
                repair["description"].as_str().unwrap().to_owned(),
                repair["edit"]["replacement"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned(),
            )
        })
        .collect()
}

/// A repair as [`rows`] reads it.
type Row = (String, String, String);

/// The repairs of the diagnostic at a line and column.
type Positioned = ((u64, u64), Vec<Row>);

fn row(widens: &str, description: &str, edit: &str) -> Row {
    (widens.to_owned(), description.to_owned(), edit.to_owned())
}

/// The narrowed grant of a U3020 within `key`'s ceiling.
fn grant(binding: &str, key: &str) -> String {
    format!(
        "grant `with {binding}`, narrowed to the operations the callee declares, which lie within the ceiling of `{key}`"
    )
}

#[test]
fn u3020_within_the_ceiling_grants_the_narrowed_requirement_first() {
    let diagnostics = check_json("authority/boundary_repair_within_ceiling");
    assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");
    for (line, col, binding) in [
        (14, 3, "@files = @files[read]"),
        (19, 3, "@files = @user_files[read]"),
        (24, 15, "@files = @files[read]"),
    ] {
        let found = at(&diagnostics, "U3020", line, col);
        assert_eq!(found.len(), 1, "{diagnostics:#?}");
        assert_eq!(
            rows(&found[0]),
            [row(
                "",
                &grant(binding, "media.resize"),
                &format!(" with {binding}")
            )]
        );
    }
}

#[test]
fn u3020_beyond_the_ceiling_offers_only_a_handler_and_names_what_it_withholds() {
    let diagnostics = check_json("authority/boundary_repair_beyond_ceiling");
    let files = &at(&diagnostics, "U3020", 14, 3)[0];
    let net = &at(&diagnostics, "U3020", 19, 3)[0];
    for (diagnostic, withheld) in [(files, "`write`"), (net, "`post`")] {
        let rows = rows(diagnostic);
        assert_eq!(rows.len(), 1, "{rows:?}");
        let (widens, description, edit) = &rows[0];
        assert_eq!(widens, "");
        assert_eq!(edit, "", "no grant beyond the ceiling carries an edit");
        assert!(
            description.starts_with("install a handler for `"),
            "{description}"
        );
        assert!(
            description.contains(&format!("the ceiling in `una.toml` withholds {withheld}")),
            "{description}"
        );
        assert!(!description.contains("grant `with"), "{description}");
    }
    assert!(
        rows(files)[0]
            .1
            .contains("which allows `media.store.Files[read]`")
    );
    assert!(
        rows(net)[0]
            .1
            .contains("which allows no operation of `Net`")
    );
    // The same requirement in the body of a lambda: no `with` can be written there, so the
    // advice points to a named function that installs the handler, never to a grant.
    assert_eq!(
        rows_at(&diagnostics, "U3020", 29, 25),
        [row(
            "",
            "this call is in the body of a lambda, which is one line and cannot carry `with`, so no handler for `@files` can be installed for it here: make the call from a named function, where the statement that holds it installs a handler for `media.store.Files` whose own `uses` lie within the ceiling of `media.resize`, which allows `media.store.Files[read]`: `with @files = <handler>(...)`, where `handler <handler>(...) for media.store.Files` is written outside the boundary; the ceiling in `una.toml` withholds `write`, so no binding that permits it can be granted",
            ""
        )]
    );
}

#[test]
fn una_fix_applies_the_grant_within_the_ceiling_and_the_result_builds() {
    for (case, rounds, applied) in [
        (
            "authority/boundary_repair_within_ceiling",
            1,
            vec![
                " with @files = @files[read]",
                " with @files = @user_files[read]",
                " with @files = @files[read]",
            ],
        ),
        // Two grants at one call touch at one point: the second waits for the next round,
        // which appends it to the `with` the first wrote (ADR 0084 rule 3).
        (
            "authority/boundary_repair_two_needs",
            2,
            vec![" with @files = @files[read]", ", @net = @net[post]"],
        ),
    ] {
        let copy = copy_of(case);
        let manifest = std::fs::read(copy.join("una.toml")).unwrap();
        let document = fix(&copy);
        assert_eq!(document["stopped"], "done", "{document}");
        assert_eq!(document["rounds"], rounds, "{document}");
        let replacements: Vec<&str> = document["applied"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| {
                assert_eq!(a["code"], "U3020");
                a["replacement"].as_str().unwrap()
            })
            .collect();
        assert_eq!(replacements, applied, "{document}");
        assert_eq!(document["skipped"], serde_json::json!([]));
        assert_eq!(document["diagnostics"], serde_json::json!([]), "{document}");
        assert_eq!(std::fs::read(copy.join("una.toml")).unwrap(), manifest);
        assert!(una(&copy, &["check", "."]).status.success());
    }
    // What `una fix` leaves is the reference that checks clean.
    let fixed = copy_of("authority/boundary_repair_two_needs");
    fix(&fixed);
    let text = std::fs::read_to_string(fixed.join("src/media/app.una")).unwrap();
    assert!(
        text.contains("  share(path) with @files = @files[read], @net = @net[post]\n"),
        "{text}"
    );
}

#[test]
fn una_fix_applies_nothing_beyond_the_ceiling() {
    let copy = copy_of("authority/boundary_repair_beyond_ceiling");
    let before = files_of(&copy);
    let document = fix(&copy);
    assert_eq!(document["applied"], serde_json::json!([]), "{document}");
    // Advice is not a candidate, so nothing is skipped either.
    assert_eq!(document["skipped"], serde_json::json!([]), "{document}");
    let codes: Vec<&str> = document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["U3020", "U3020", "U3020"]);
    assert_eq!(files_of(&copy), before, "every byte, `una.toml` included");
}

/// Every conformance case whose manifest declares ceilings: `una.toml` with `[boundaries]`, or
/// `ita.una` with `boundaries` or `needs` (a dependency is a boundary too, ADR 0122), by its path
/// under `tests/conformance`, with the manifest's file name.
fn boundary_cases_with_manifest() -> Vec<(String, &'static str)> {
    let root = conformance("");
    let mut cases = Vec::new();
    for area in std::fs::read_dir(&root).unwrap() {
        let area = area.unwrap();
        if !area.file_type().unwrap().is_dir() {
            continue;
        }
        for case in std::fs::read_dir(area.path()).unwrap() {
            let case = case.unwrap().path();
            let toml = std::fs::read_to_string(case.join("una.toml"))
                .is_ok_and(|text| text.contains("[boundaries]"));
            let ita = std::fs::read_to_string(case.join("ita.una")).is_ok_and(|text| {
                text.lines()
                    .any(|line| matches!(line.trim_end(), "boundaries" | "needs"))
            });
            let manifest = match (toml, ita) {
                (true, _) => "una.toml",
                (false, true) => "ita.una",
                (false, false) => continue,
            };
            cases.push((
                case.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                manifest,
            ));
        }
    }
    cases.sort();
    cases
}

/// [`boundary_cases_with_manifest`], by path alone.
fn boundary_cases() -> Vec<String> {
    boundary_cases_with_manifest()
        .into_iter()
        .map(|(case, _)| case)
        .collect()
}

/// Whether `text` is exactly one narrowed grant added to a statement, ` with @x = @y[ops]` or
/// `, @x = @y[ops]`, with one or more operations: what the checker writes for a U3020 within
/// the ceiling (ADR 0093 rule 6).
fn narrowed_grant(text: &str) -> bool {
    let name = |part: &str| {
        part.strip_prefix('@').is_some_and(|rest| {
            !rest.is_empty() && rest.chars().all(|c| c.is_alphanumeric() || c == '_')
        })
    };
    text.strip_prefix(" with ")
        .or_else(|| text.strip_prefix(", "))
        .and_then(|binding| binding.split_once(" = "))
        .and_then(|(target, source)| {
            let (held, ops) = source.strip_suffix(']')?.split_once('[')?;
            Some(
                name(target)
                    && name(held)
                    && ops.split(", ").all(|op| {
                        !op.is_empty() && op.chars().all(|c| c.is_alphanumeric() || c == '_')
                    }),
            )
        })
        .unwrap_or(false)
}

#[test]
fn the_sweep_reads_a_grant_of_several_operations_and_refuses_anything_else() {
    assert!(narrowed_grant(" with @files = @files[read]"));
    assert!(narrowed_grant(" with @files = @store[read, write]"));
    assert!(narrowed_grant(", @net = @user_net[post, get]"));
    for text in [
        " with @files = @files",
        " with @files = read_only()",
        " with @files = @files[read], @net = @net[post]",
        " with @files = @files[]",
        " with @files = @files[read,write]",
        "uses @files: Files[read, write]",
    ] {
        assert!(!narrowed_grant(text), "{text}");
    }
}

/// The number of U3021 diagnostics among `diagnostics`.
fn u3021s(diagnostics: &[Value]) -> usize {
    diagnostics.iter().filter(|d| d["code"] == "U3021").count()
}

#[test]
fn una_fix_never_writes_a_ceiling_nor_applies_a_boundary_edit_beyond_the_narrowed_grant() {
    let cases = boundary_cases_with_manifest();
    assert!(cases.len() >= 30, "{cases:?}");
    assert!(
        cases.iter().any(|(_, manifest)| *manifest == "ita.una"),
        "{cases:?}"
    );
    let mut grants = 0;
    for (case, manifest_file) in cases {
        let copy = copy_of(&case);
        let manifest = std::fs::read(copy.join(manifest_file)).unwrap();
        let before = una(&copy, &["--json", "check", "--tier", "0", "."]);
        let before = json_of(&before);
        let over_before = u3021s(before["diagnostics"].as_array().unwrap());
        let document = fix(&copy);
        assert_eq!(
            std::fs::read(copy.join(manifest_file)).unwrap(),
            manifest,
            "{case}: `una fix` wrote the ceiling in `{manifest_file}`"
        );
        for applied in document["applied"].as_array().unwrap() {
            let code = applied["code"].as_str().unwrap();
            assert!(
                !matches!(code, "U3021" | "U3024"),
                "{case}: applied a {code} edit: {applied}"
            );
            if code == "U3020" {
                let text = applied["replacement"].as_str().unwrap();
                assert!(
                    narrowed_grant(text),
                    "{case}: a U3020 edit that is not one narrowed grant: {text}"
                );
                grants += 1;
            }
        }
        // The grant the check offers lies within the ceiling: `una fix` leaves no more U3021
        // than it found, so a round whose grant goes over the ceiling fails here.
        let over_after = u3021s(document["diagnostics"].as_array().unwrap());
        assert!(
            over_after <= over_before,
            "{case}: a grant `una fix` applied is over the ceiling: {document}"
        );
    }
    // The sweep exercises applied grants: those of `boundary_repair_within_ceiling` (3),
    // `boundary_repair_two_needs` (2) and `boundary_repair_choice` (1) at least.
    assert!(grants >= 6, "{grants} grants applied");
}

/// The repairs of the diagnostic with `code` at `line`:`col` of `diagnostics`.
fn rows_at(diagnostics: &[Value], code: &str, line: u64, col: u64) -> Vec<Row> {
    let found = at(diagnostics, code, line, col);
    assert_eq!(found.len(), 1, "{code} at {line}:{col}: {diagnostics:#?}");
    rows(&found[0])
}

#[test]
fn u3020_with_several_qualifying_bindings_is_advice_that_lists_them() {
    let diagnostics = check_json("authority/boundary_repair_choice");
    // Two bindings permit `read` and neither is `@files`: which one the third-party library
    // reads is the consumer's decision, so no edit names one.
    assert_eq!(
        rows_at(&diagnostics, "U3020", 25, 3),
        [row(
            "",
            "grant `@files` with `with @files = @y[read]`, narrowed to the operations the callee declares, which lie within the ceiling of `media.resize`, where `@y` is one of `@secrets` and `@user`, which each permit `read`; which binding the boundary receives is a decision about trust, so no edit is offered",
            ""
        )]
    );
    // `@files` itself permits `read`: the grant names it, with its edit.
    assert_eq!(
        rows_at(&diagnostics, "U3020", 31, 3),
        [row(
            "",
            &grant("@files = @files[read]", "media.resize"),
            " with @files = @files[read]"
        )]
    );
    // A function value: advice, never an edit.
    assert_eq!(
        rows_at(&diagnostics, "U3020", 36, 11),
        [row(
            "",
            &format!(
                "{}, written on the statement that holds this call or reference, where it binds `@files` for the whole statement",
                grant("@files = @files[read]", "media.resize")
            ),
            ""
        )]
    );
    // An installation: the grant resolves below the installing `with`, so it names the
    // header's `@user_net`, never `@n`, which the same `with` binds.
    assert_eq!(
        rows_at(&diagnostics, "U3020", 47, 44),
        [row(
            "",
            &format!(
                "{}, as another binding of the `with` that installs the handler",
                grant("@net = @user_net[post]", "media.cache")
            ),
            ""
        )]
    );
    // The statement has a `with` already: the grant added to it resolves outside that
    // `with`, where the header's `@secrets` and `@user` both still qualify, whichever of them
    // the `with` rebinds. Advice, never an edit.
    let several = rows_at(&diagnostics, "U3020", 25, 3);
    assert_eq!(rows_at(&diagnostics, "U3020", 53, 3), several);
    assert_eq!(rows_at(&diagnostics, "U3020", 59, 3), several);
    // `with @secrets = @secrets[write]` narrows `@secrets` inside the statement, but the
    // grant resolves outside it, where `@secrets` is the only binding and permits `read`.
    assert_eq!(
        rows_at(&diagnostics, "U3020", 64, 3),
        [row(
            "",
            &grant("@files = @secrets[read]", "media.resize"),
            ", @files = @secrets[read]"
        )]
    );
    // A call in the body of a lambda given to `apply`, on a statement with a `with`: the body
    // is one line, so no grant can be written there, and the frames are the lambda's own. The
    // advice names no binding: neither the lambda's own `@files`, which the caller does not
    // hold, nor `@secrets` or `@user`, the statement's own choice (its U3001 lists them).
    let lambda = rows_at(&diagnostics, "U3020", 75, 27);
    assert_eq!(
        lambda,
        [row(
            "",
            "this call is in the body of a lambda, which is one line and cannot carry `with`, so no grant of `@files` can be written for it here: make the call from a named function, where the statement that holds it grants `@files` with `with`, within the ceiling of `media.resize`, which allows `media.store.Files[read]`",
            ""
        )]
    );
    for name in ["@files =", "@files[", "@secrets", "@user"] {
        assert!(!lambda[0].1.contains(name), "{name}: {lambda:?}");
    }
    let statement = at(&diagnostics, "U3001", 75, 3);
    assert_eq!(statement.len(), 1, "{diagnostics:#?}");
    assert!(
        statement[0]["repairs"][0]["description"]
            .as_str()
            .unwrap()
            .contains("one of `@secrets` and `@user`"),
        "{statement:#?}"
    );
    // The named function the advice describes: the lambda calls `via`, whose statement grants
    // `@files`, and nothing is reported in `via` or `lam_via`.
    assert!(
        diagnostics
            .iter()
            .all(|d| !(77..=85).contains(&d["primary"]["start"]["line"].as_u64().unwrap())),
        "{diagnostics:#?}"
    );
    // A function value referenced in the body of a lambda: the same advice, worded for the
    // reference, naming no binding either.
    let value = rows_at(&diagnostics, "U3020", 90, 39);
    assert_eq!(
        value,
        [row(
            "",
            "this reference is in the body of a lambda, which is one line and cannot carry `with`, so no grant of `@files` can be written for it here: make the reference from a named function, where the statement that holds it grants `@files` with `with`, within the ceiling of `media.resize`, which allows `media.store.Files[read]`",
            ""
        )]
    );
    for name in ["@files =", "@files[", "@secrets"] {
        assert!(!value[0].1.contains(name), "{name}: {value:?}");
    }
}

#[test]
fn una_fix_applies_only_the_grant_that_names_one_binding() {
    let copy = copy_of("authority/boundary_repair_choice");
    let manifest = std::fs::read(copy.join("una.toml")).unwrap();
    let document = fix(&copy);
    let applied: Vec<(&str, &str)> = document["applied"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["code"].as_str().unwrap(),
                a["replacement"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        applied,
        [
            ("U3020", " with @files = @files[read]"),
            ("U3020", ", @files = @secrets[read]")
        ],
        "{document}"
    );
    assert_eq!(std::fs::read(copy.join("una.toml")).unwrap(), manifest);
    let text = std::fs::read_to_string(copy.join("src/media/app.una")).unwrap();
    // The only `@secrets[read]` is the grant of `narrowed`, the only binding there.
    assert_eq!(text.matches("@secrets[read]").count(), 1, "{text}");
    assert!(
        text.contains("shrink(path) with @secrets = @secrets[write], @files = @secrets[read]"),
        "{text}"
    );
    assert!(!text.contains("@user[read]"), "{text}");
    assert!(!text.contains("@n[post]"), "{text}");
    assert!(
        text.contains("  shrink(path) with @user = @secrets\n"),
        "{text}"
    );
    assert!(
        text.contains("  shrink(path) with @secrets = @user[write]\n"),
        "{text}"
    );
}

#[test]
fn u3021_names_the_callers_binding_and_the_handler_that_u3020_offers() {
    // #1931: the narrowed grant names the caller's binding, never a placeholder `@y` alone,
    // and when the ceiling withholds an operation the callee declares, U3021 offers the
    // handler that U3020 offers for the same requirement; following both checks
    // (`authority/boundary_ceiling_repair_applied`).
    let diagnostics = check_json("authority/boundary_ceiling_repair_binding");
    let found = at(&diagnostics, "U3021", 13, 3);
    let texts: Vec<Vec<Row>> = found.iter().map(rows).collect();
    assert_eq!(
        texts,
        [
            vec![row(
                "",
                "narrow the grant to what the callee declares, `Prices[price]`, which lies within the ceiling: `with @prices = @store[price]`, or a handler whose own `uses` lie within `Prices[price]`",
                ""
            )],
            vec![row(
                "",
                "no grant of a binding keeps both: the callee declares `Clock[now]`, and the ceiling of `shop.quote` allows no operation of `Clock`; install a handler for `Clock` whose own `uses` lie within the ceiling: `with @clock = <handler>(...)`, where `handler <handler>(...) for Clock` is written outside the boundary, or the consumer decides between this module and its ceiling",
                ""
            )],
        ]
    );
    for text in texts.iter().flatten() {
        assert!(!text.1.contains("@y"), "{text:?}");
    }
    assert_eq!(
        check_json("authority/boundary_ceiling_repair_applied"),
        Vec::<Value>::new()
    );
}

#[test]
fn u3021_gives_one_narrowed_grant_per_requirement() {
    // #2213: two requirements of one capability each get the operations they declare, and
    // following the repair checks, where one grant of both operations stays U3021.
    let diagnostics = check_json("authority/boundary_ceiling_repair_per_requirement");
    let found = at(&diagnostics, "U3021", 12, 3);
    assert_eq!(found.len(), 1, "{diagnostics:?}");
    assert_eq!(
        rows(&found[0]),
        [row(
            "",
            "narrow each grant to what the callee declares for it, `@a: Prices[price]` and `@b: Prices[set]`, which lie within the ceiling, with one narrowed grant per requirement: `with @a = @full[price], @b = @full[set]`, or a handler for each whose own `uses` lie within `Prices[price, set]`",
            ""
        )]
    );
    assert_eq!(
        check_json("authority/boundary_ceiling_repair_per_requirement_applied"),
        Vec::<Value>::new()
    );
}

#[test]
fn u3021_names_the_requirement_whose_operations_the_ceiling_withholds() {
    // #2213: the handler route names `@b`, whose `set` the ceiling withholds, and `@a` keeps
    // its narrowed grant in the same `with`; following both checks.
    let diagnostics = check_json("authority/boundary_ceiling_repair_withheld_requirement");
    let found = at(&diagnostics, "U3021", 12, 3);
    assert_eq!(found.len(), 1, "{diagnostics:?}");
    assert_eq!(
        rows(&found[0]),
        [row(
            "",
            "no grant of a binding keeps both: the callee declares `@b: Prices[set]`, and the ceiling of `shop.quote` allows `Prices[price]`; install a handler for `Prices` whose own `uses` lie within the ceiling for `@b`, and narrow the grant of `@a` to what the callee declares: `with @a = @full[price], @b = <handler>(...)`, where `handler <handler>(...) for Prices` is written outside the boundary, or the consumer decides between this module and its ceiling",
            ""
        )]
    );
    assert_eq!(
        check_json("authority/boundary_ceiling_repair_withheld_applied"),
        Vec::<Value>::new()
    );
}

#[test]
fn u3021_names_the_handler_whose_own_uses_bring_in_the_excess() {
    // #2213: the grant of `@prices` is already within the ceiling; the repair points at the
    // handler granted as `@clock`, not at the grant already written.
    let diagnostics = check_json("authority/boundary_ceiling_repair_handler_uses");
    let found = at(&diagnostics, "U3021", 18, 3);
    assert_eq!(found.len(), 1, "{diagnostics:?}");
    assert_eq!(
        rows(&found[0]),
        [row(
            "",
            "the grant of `@prices` already lies within the ceiling; what exceeds it is the own `uses` of the handler granted as `@clock`, which count at this call: grant `@clock` a handler whose own `uses` lie within the ceiling of `shop.quote`, which allows `Prices[price]`",
            ""
        )]
    );
    // A handler whose own `uses` name a capability that the callee does not require.
    let handler = check_json("authority/boundary_ceiling_handler");
    let found = at(&handler, "U3021", 25, 3);
    assert_eq!(
        rows(&found[0]),
        [row(
            "",
            "narrow the own `uses` of the handler granted as `@files`, which count at this call: grant `@files` a handler whose own `uses` lie within the ceiling of `media.resize`, which allows no operation of `Net`",
            ""
        )]
    );
}

#[test]
fn u3021_tells_a_handlers_own_uses_from_what_its_installation_gives_it() {
    // Review 1 of #2241, M1: `relay` declares no `uses`, so the excess at 30:3 and 35:3 is what
    // its installation's arguments give it (ADR 0093 rule 14), and the repair points at those
    // arguments; `relay_held` at 40:3 brings in `Net` both ways, and the repair names both.
    let diagnostics = check_json("security/boundary_installed_arguments");
    let texts: Vec<Vec<Row>> = [(30, 3), (35, 3), (40, 3)]
        .iter()
        .map(|&(line, col)| {
            let found = at(&diagnostics, "U3021", line, col);
            assert_eq!(found.len(), 1, "{diagnostics:?}");
            rows(&found[0])
        })
        .collect();
    let args = "what the installation of the handler granted as `@log` gives it, the `uses` of the functions and lambdas among its arguments, which count at this call: install `@log` with arguments whose `uses` lie within the ceiling of `media.render`, which allows no operation of `Net`, such as a named function that declares none; a lambda given there counts the `uses` that its parameter's type declares";
    let own = "the own `uses` of the handler granted as `@log`, which count at this call: grant `@log` a handler whose own `uses` lie within the ceiling of `media.render`, which allows no operation of `Net`";
    assert_eq!(
        texts,
        [
            vec![row("", &format!("narrow {args}"), "")],
            vec![row("", &format!("narrow {args}"), "")],
            vec![row("", &format!("narrow {own}; and {args}"), "")],
        ]
    );
}

#[test]
fn u3021_narrows_to_the_ceiling_or_says_that_no_grant_keeps_both() {
    let ceiling = check_json("authority/boundary_ceiling");
    let found = at(&ceiling, "U3021", 12, 3);
    assert_eq!(
        rows(&found[0]),
        [row(
            "",
            "narrow the grant to what the callee declares, `media.store.Files[read]`, which lies within the ceiling: `with @files = @files[read]`, or a handler whose own `uses` lie within `media.store.Files[read]`",
            ""
        )]
    );
    let beyond = check_json("project/boundaries_unknown_capability");
    let found = at(&beyond, "U3021", 13, 3);
    let beyond_rows = rows(&found[0]);
    assert_eq!(beyond_rows.len(), 1, "{beyond_rows:?}");
    assert!(
        beyond_rows[0]
            .1
            .starts_with("no grant of a binding keeps both: "),
        "{beyond_rows:?}"
    );
    assert_eq!(beyond_rows[0].2, "");
    // A value given to the boundary: narrow what it gives.
    let given = check_json("authority/boundary_functional_argument");
    let found = at(&given, "U3021", 12, 3);
    assert_eq!(
        rows(&found[0]),
        [row(
            "",
            "narrow what this gives `media.resize` to its ceiling, which allows no operation of `Net`: a handler or a function value whose own `uses` lie within it",
            ""
        )]
    );
    // No repair of any boundary case edits `una.toml`, where the ceilings are.
    for case in boundary_cases() {
        for diagnostic in check_json(&case) {
            for repair in diagnostic["repairs"].as_array().unwrap() {
                let file = repair["edit"]["file"].as_str().unwrap_or("");
                assert!(
                    !file.ends_with("una.toml"),
                    "{case}: a repair edits `una.toml`: {repair}"
                );
            }
        }
    }
}

/// The repairs of the U3001 and U3002 of the edition case `edition`, by position.
fn narrowing(edition: &str) -> Vec<Positioned> {
    let diagnostics = check_json(&format!("authority/narrowing_repairs_edition_{edition}"));
    let mut out: Vec<Positioned> = diagnostics
        .iter()
        .map(|d| {
            (
                (
                    d["primary"]["start"]["line"].as_u64().unwrap(),
                    d["primary"]["start"]["col"].as_u64().unwrap(),
                ),
                rows(d),
            )
        })
        .collect();
    out.sort();
    out
}

const PUBLIC: &str = "this is an authority change of the public header of";

#[test]
fn edition_0_3_ranks_a_narrowed_rebinding_first_and_a_public_widening_last() {
    let rebind =
        "add `with @files = @store[read]` to this statement, since `@store` permits `read`";
    assert_eq!(
        narrowing("0_3"),
        [
            (
                (22, 3),
                vec![
                    row("", rebind, " with @files = @store[read]"),
                    row(
                        "authority",
                        &format!(
                            "add `read` to the `uses` clause of `@files`; {PUBLIC} `more`, which every caller in every module must then hold"
                        ),
                        "uses @files: Files[read, write]"
                    ),
                ]
            ),
            (
                (27, 3),
                vec![
                    row("", rebind, " with @files = @store[read]"),
                    row(
                        "authority",
                        &format!(
                            "declare `uses @files: Files[read]` in the header; {PUBLIC} `missing`, which every caller in every module must then hold"
                        ),
                        "\n  uses @files: Files[read]"
                    ),
                ]
            ),
            // A private header: the widening is marked, and not as a public authority change.
            (
                (32, 3),
                vec![row(
                    "authority",
                    "add `read` to the `uses` clause of `@files`",
                    "uses @files: Files[read, write]"
                )]
            ),
            (
                (37, 10),
                vec![row(
                    "authority",
                    &format!(
                        "add `read` to the `uses` clause of `@files`; {PUBLIC} `direct`, which every caller in every module must then hold"
                    ),
                    "uses @files: Files[read, write]"
                )]
            ),
        ]
    );
}

#[test]
fn edition_0_3_rebinds_with_an_edit_only_when_one_binding_qualifies() {
    let diagnostics = check_json("authority/narrowing_repairs_choice");
    let several = row(
        "",
        "add `with @files = @y[read]` to this statement, where `@y` is one of `@backup` and `@prod`, which each permit `read`; which binding `@files` receives is a decision about trust, so no edit is offered",
        "",
    );
    let missing = rows_at(&diagnostics, "U3001", 24, 3);
    assert_eq!(missing[0], several);
    assert_eq!(missing.len(), 2, "{missing:?}");
    assert!(missing[1].1.contains(PUBLIC), "{missing:?}");
    let more = rows_at(&diagnostics, "U3002", 31, 3);
    assert_eq!(more[0], several);
    assert_eq!(more.len(), 2, "{more:?}");
    assert_eq!(more[1].0, "authority");
    // One binding qualifies: the narrowed rebinding with its edit.
    let single = rows_at(&diagnostics, "U3001", 36, 3);
    assert_eq!(
        single[0],
        row(
            "",
            "add `with @files = @prod[read]` to this statement, since `@prod` permits `read`",
            " with @files = @prod[read]"
        )
    );
    // The statement has a `with` already: the rebinding added to it resolves outside that
    // `with`, where the header's `@backup` and `@prod` both still qualify, so it stays advice
    // although the `with` rebinds `@prod` to `@backup`.
    let missing_with = rows_at(&diagnostics, "U3001", 42, 3);
    assert_eq!(missing_with[0], several);
    assert_eq!(missing_with.len(), 2, "{missing_with:?}");
    let more_with = rows_at(&diagnostics, "U3002", 49, 3);
    assert_eq!(more_with[0], several);
    assert_eq!(more_with.len(), 2, "{more_with:?}");
    // `with @prod = @prod[write]` narrows `@prod` inside the statement only; outside it,
    // `@prod` is the one binding and permits `read`.
    let single_with = rows_at(&diagnostics, "U3001", 54, 3);
    assert_eq!(
        single_with[0],
        row(
            "",
            "add `with @files = @prod[read]` to this statement, since `@prod` permits `read`",
            ", @files = @prod[read]"
        )
    );
    let copy = copy_of("authority/narrowing_repairs_choice");
    let document = fix(&copy);
    let applied: Vec<&str> = document["applied"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["replacement"].as_str().unwrap())
        .collect();
    assert_eq!(
        applied,
        [" with @files = @prod[read]", ", @files = @prod[read]"],
        "{document}"
    );
}

#[test]
fn editions_0_1_and_0_2_keep_their_u3001_and_u3002_repairs() {
    let found = narrowing("0_2");
    assert_eq!(
        found[0],
        (
            (22, 3),
            vec![
                row(
                    "",
                    "add `with @files = @store` to this statement, since `@store` permits `read`",
                    " with @files = @store"
                ),
                row(
                    "authority",
                    "add `read` to the `uses` clause of `@files`",
                    "uses @files: Files[read, write]"
                ),
            ]
        )
    );
    assert_eq!(
        found[1],
        (
            (27, 3),
            vec![row(
                "authority",
                "declare `uses @files: Files[read]` in the header",
                "\n  uses @files: Files[read]"
            )]
        )
    );
    for (_, rows) in &found {
        assert!(rows.iter().all(|(_, d, _)| !d.contains(PUBLIC)), "{rows:?}");
    }
}

#[test]
fn una_fix_applies_the_narrowed_rebindings_of_edition_0_3_and_never_a_widening() {
    let copy = copy_of("authority/narrowing_repairs_edition_0_3");
    let document = fix(&copy);
    let applied: Vec<(&str, &str)> = document["applied"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["code"].as_str().unwrap(),
                a["replacement"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        applied,
        [
            ("U3002", " with @files = @store[read]"),
            ("U3001", " with @files = @store[read]"),
        ],
        "{document}"
    );
    let skipped: Vec<(&str, &str)> = document["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["code"].as_str().unwrap(), s["reason"].as_str().unwrap()))
        .collect();
    assert_eq!(
        skipped,
        [("U3002", "widens authority"), ("U3002", "widens authority")]
    );
    let text = std::fs::read_to_string(copy.join("src/m.una")).unwrap();
    assert_eq!(
        text.matches("uses @files: Files[write]").count(),
        3,
        "{text}"
    );
}
