//! Escape repairs rank last and say what they cost (ADR 0095, #574): on named conformance cases,
//! the repairs of `una --json check` come in the exact order of the decision's table, and each
//! escape carries its kind and cost. The invariant over every case is in the conformance
//! runner.

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

/// The conformance case at `case`, relative to `tests/conformance`.
fn case(case: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/conformance")
        .join(case)
}

/// The diagnostics of `una --json check` on the package directory or single file `path`.
fn check_json(path: &Path) -> Vec<Value> {
    let (dir, target) = if path.is_dir() {
        (path.to_path_buf(), ".".to_owned())
    } else {
        (
            path.parent().unwrap().to_path_buf(),
            path.file_name().unwrap().to_string_lossy().into_owned(),
        )
    };
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .args(["--json", "check", &target])
        .current_dir(dir)
        .output()
        .unwrap();
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    document["diagnostics"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The first diagnostic with `code` at `line`:`col`.
fn at(diagnostics: &[Value], code: &str, line: u64, col: u64) -> Value {
    diagnostics
        .iter()
        .find(|d| {
            d["code"] == code
                && d["primary"]["start"]["line"] == line
                && d["primary"]["start"]["col"] == col
        })
        .cloned()
        .unwrap_or_else(|| panic!("no {code} at {line}:{col} in {diagnostics:#?}"))
}

/// Each repair as (rank, escape kind or "", description), in output order.
fn repairs(diagnostic: &Value) -> Vec<(u64, String, String)> {
    diagnostic["repairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repair| {
            (
                repair["rank"].as_u64().unwrap(),
                repair["escape"]["kind"].as_str().unwrap_or("").to_owned(),
                repair["description"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn row(rank: u64, escape: &str, description: &str) -> (u64, String, String) {
    (rank, escape.to_owned(), description.to_owned())
}

#[test]
fn u7003_proves_first_and_raises_the_budget_last() {
    let diagnostics = check_json(&case("authority/escape_budget_exceeded"));
    let diagnostic = at(&diagnostics, "U7003", 16, 3);
    assert_eq!(
        repairs(&diagnostic),
        [
            row(
                1,
                "",
                "state the fact in a refined type or a `requires` clause, so that the solver proves it, and remove the `checked`"
            ),
            row(
                2,
                "",
                "handle the values where the fact does not hold with a guard (`if` or a `when` arm), and remove the `checked`"
            ),
            row(
                3,
                "escape_budget",
                "raise `checked_per_module` in `[escape_budget]` of `una.toml`"
            ),
        ]
    );
    assert_eq!(diagnostic["repairs"][2]["escape_cost"], 1);
    assert!(
        diagnostic["repairs"][2]["escape"]["cost"]
            .as_str()
            .unwrap()
            .contains("build report")
    );
}

#[test]
fn u2016_tests_without_the_fact_first_and_checks_last() {
    // In code, for a predicate whose body calls no other opaque predicate, a function with its
    // body keeps the escape budget, and `checked` comes last.
    let value = check_json(&case("types/opaque_predicate_as_value.una"));
    assert_eq!(
        repairs(&at(&value, "U2016", 14, 11)),
        [
            row(
                1,
                "",
                "if the code needs only the test and not the fact, pass a function, not a predicate, with the body of `valid_slug`"
            ),
            row(
                2,
                "checked",
                "call it inside `checked(value, valid_slug(it)) else fail <Variant>` instead"
            ),
        ]
    );
    // ADR 0095, rule 6 conditions: no function route where it would be U2016 again, or where
    // a call of a function is not allowed.
    let call = check_json(&case("types/opaque_predicate_outside_checked"));
    let only_checked = |line, col, name: &str| {
        assert_eq!(
            repairs(&at(&call, "U2016", line, col)),
            [row(
                1,
                "checked",
                &format!("check the value with `checked(value, {name}(it)) else fail <Variant>`")
            )],
            "U2016 at {line}:{col}"
        );
    };
    // `valid_slug` calls the opaque `short`.
    only_checked(37, 34, "valid_slug");
    // A `requires` clause, a `reflect` body and an example.
    only_checked(45, 12, "short");
    only_checked(49, 57, "short");
    only_checked(52, 6, "short");
}

#[test]
fn u4004_declares_trust_last() {
    // No module of the case is trusted, so declaring this one trusted is the only repair, and
    // it is marked.
    let diagnostics = check_json(&case("security/policy_outside_trusted.una"));
    let diagnostic = at(&diagnostics, "U4004", 6, 12);
    assert_eq!(
        diagnostic["message"],
        "policy `reveal` is declared in module `policy_outside_trusted`, which is not trusted"
    );
    assert_eq!(
        repairs(&diagnostic),
        [row(
            1,
            "trusted",
            "declare `trusted module policy_outside_trusted` in its header and list `policy_outside_trusted` under `[trusted] modules` in `una.toml`"
        )]
    );
}

#[test]
fn u4004_moves_only_into_a_module_trusted_in_both_places() {
    // `app.vault` is trusted in its header and in `una.toml`; `app.keys` only in its header,
    // which is U7005, so moving the policy there would only move the error (spec/07 §6).
    let diagnostics = check_json(&case("security/policy_move_trusted_both"));
    assert_eq!(
        repairs(&at(&diagnostics, "U4004", 7, 12)),
        [
            row(
                1,
                "",
                "move the policy and its `via` function into a module that is already trusted: `app.vault`"
            ),
            row(
                2,
                "trusted",
                "declare `trusted module app.main` in its header and list `app.main` under `[trusted] modules` in `una.toml`"
            ),
        ]
    );
}

#[test]
fn u7005_narrows_trust_first() {
    let diagnostics = check_json(&case("security/trusted_toml_only"));
    assert_eq!(
        repairs(&at(&diagnostics, "U7005", 10, 12)),
        [
            // `app.keys` declares a policy, so narrowing alone does not close the problem.
            row(
                1,
                "",
                "remove `app.keys` from `[trusted] modules`; its policies are then U4004: move them into a trusted module"
            ),
            row(
                2,
                "trusted",
                "write `trusted module app.keys` in the header of `app.keys`"
            ),
        ]
    );
    // A file checked on its own has no una.toml: removing `trusted` is the only route.
    let single = check_json(&case("project/trusted_header_not_listed.una"));
    assert_eq!(
        repairs(&at(&single, "U7005", 4, 1)),
        [row(
            1,
            "",
            "remove `trusted` from the header of `trusted_header_not_listed`"
        )]
    );
}

#[test]
fn u5002_at_the_step_budget_offers_no_escape() {
    // The verification timeout of `spec/12 §6`: a postcondition, whose repairs all keep the
    // proof obligation.
    let diagnostics = check_json(&case("verification/step_budget_unknown"));
    let diagnostic = at(&diagnostics, "U5002", 9, 11);
    let rows = repairs(&diagnostic);
    assert!(!rows.is_empty());
    for (index, (rank, escape, _)) in rows.iter().enumerate() {
        assert_eq!(*rank, u64::try_from(index + 1).unwrap());
        assert_eq!(escape, "");
        assert!(diagnostic["repairs"][index].get("escape").is_none());
    }
}
