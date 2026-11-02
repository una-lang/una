//! Repairs that narrow before they widen (ADR 0093 rule 6, #1397): the first repair of a U3020
//! grants the callee's requirement narrowed to what it declares, and only within the
//! boundary's ceiling; `una fix` applies that grant and nothing beyond it, never writes the
//! ceiling, and never applies an edit of U3021 or U3024 (ADR 0084). U3021's repair narrows to the
//! ceiling or says that no grant keeps both. In edition 0.3, U3001 and U3002 rank a narrowed
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

/// A fresh copy of the case at `case`, for `una fix` to write.
fn copy_of(case: &str) -> PathBuf {
    let copy = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("boundary-repairs")
        .join(case.replace('/', "-"));
    let _ = std::fs::remove_dir_all(&copy);
    for (file, bytes) in files_of(&conformance(case)) {
        let target = copy.join(&file);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, bytes).unwrap();
    }
    copy
}

/// The diagnostics of `una --json check` on the case at `case`.
fn check_json(case: &str) -> Vec<Value> {
    let output = una(&conformance(case), &["--json", "check", "--tier", "0", "."]);
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    document["diagnostics"].as_array().cloned().unwrap()
}

/// The `una.fix/1` document of `una --json fix --tier 0` on `root`.
fn fix(root: &Path) -> Value {
    let output = una(root, &["--json", "fix", "--tier", "0", "."]);
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
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
    let files = &at(&diagnostics, "U3020", 13, 3)[0];
    let net = &at(&diagnostics, "U3020", 18, 3)[0];
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
    assert_eq!(codes, ["U3020", "U3020"]);
    assert_eq!(files_of(&copy), before, "every byte, `una.toml` included");
}

/// Every conformance case whose `una.toml` declares `[boundaries]`, by its path under
/// `tests/conformance`.
fn boundary_cases() -> Vec<String> {
    let root = conformance("");
    let mut cases = Vec::new();
    for area in std::fs::read_dir(&root).unwrap() {
        let area = area.unwrap();
        if !area.file_type().unwrap().is_dir() {
            continue;
        }
        for case in std::fs::read_dir(area.path()).unwrap() {
            let case = case.unwrap().path();
            let manifest = case.join("una.toml");
            if std::fs::read_to_string(&manifest).is_ok_and(|text| text.contains("[boundaries]")) {
                cases.push(
                    case.strip_prefix(&root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    cases.sort();
    cases
}

#[test]
fn una_fix_never_writes_a_ceiling_nor_applies_a_boundary_edit_beyond_the_narrowed_grant() {
    let cases = boundary_cases();
    assert!(cases.len() >= 30, "{cases:?}");
    for case in cases {
        let copy = copy_of(&case);
        let manifest = std::fs::read(copy.join("una.toml")).unwrap();
        let document = fix(&copy);
        assert_eq!(
            std::fs::read(copy.join("una.toml")).unwrap(),
            manifest,
            "{case}: `una fix` wrote the ceiling"
        );
        for applied in document["applied"].as_array().unwrap() {
            let code = applied["code"].as_str().unwrap();
            assert!(
                !matches!(code, "U3021" | "U3024"),
                "{case}: applied a {code} edit: {applied}"
            );
            if code == "U3020" {
                let text = applied["replacement"].as_str().unwrap();
                let binding = text
                    .strip_prefix(" with ")
                    .or_else(|| text.strip_prefix(", "))
                    .unwrap_or_else(|| panic!("{case}: {text}"));
                assert!(
                    binding.ends_with(']') && binding.contains(" = @") && !binding.contains(','),
                    "{case}: a U3020 edit that is not one narrowed grant: {text}"
                );
            }
        }
        // The grant the check offers lies within the ceiling: after `una fix`, no U3021 is
        // reported where a grant was applied.
        for applied in document["applied"].as_array().unwrap() {
            let file = applied["file"].as_str().unwrap();
            let line = applied["start"]["line"].as_u64().unwrap();
            let over = document["diagnostics"].as_array().unwrap().iter().any(|d| {
                d["code"] == "U3021"
                    && d["primary"]["file"] == file
                    && d["primary"]["start"]["line"] == line
            });
            assert!(
                !over,
                "{case}: a grant `una fix` applied is over the ceiling"
            );
        }
    }
}

#[test]
fn u3021_narrows_to_the_ceiling_or_says_that_no_grant_keeps_both() {
    let ceiling = check_json("authority/boundary_ceiling");
    let found = at(&ceiling, "U3021", 12, 3);
    assert_eq!(
        rows(&found[0]),
        [row(
            "",
            "narrow the grant to what the callee declares, `media.store.Files[read]`, which lies within the ceiling: `with @files = @y[read]`, or a handler whose own `uses` lie within `media.store.Files[read]`",
            ""
        )]
    );
    let beyond = check_json("project/boundaries_unknown_capability");
    let found = at(&beyond, "U3021", 13, 3);
    let beyond_rows = rows(&found[0]);
    assert_eq!(beyond_rows.len(), 1, "{beyond_rows:?}");
    assert!(
        beyond_rows[0].1.starts_with("no grant keeps both: "),
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
