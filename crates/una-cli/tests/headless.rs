//! Headless use (`plan/milestones-v0.1.md` M10: "the evaluation track can drive `una` headlessly
//! through `--json`"): an agent's loop that reads only JSON documents and exit codes, and applies
//! a mechanical edit it was given (`spec/13 §1, §3, §5`). Tier 1 needs Z3 at the version of
//! `verification.lock` on the path.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

use pretty_assertions::assert_eq;
use serde_json::{Value, json};

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Runs `una --json <args>` in `dir`: the exit code and the one document on stdout.
fn una(dir: &Path, args: &[&str]) -> (i32, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        // The proof cache's key lives under this home, never the user's (#1164).
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .arg("--json")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert_eq!(output.stderr, b"", "{args:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.matches('\n').count(), 1, "{args:?}: one document");
    (
        output.status.code().unwrap(),
        serde_json::from_str(&text).unwrap(),
    )
}

/// The byte offset of a 1-based line and column counted in Unicode scalar values.
fn offset(text: &str, position: &Value) -> usize {
    let line = usize::try_from(position["line"].as_u64().unwrap()).unwrap();
    let col = usize::try_from(position["col"].as_u64().unwrap()).unwrap();
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
            .map_or(rest.len(), |(at, _)| at)
}

/// Applies a repair's `edit` to its file under `root`.
fn apply(root: &Path, edit: &Value) {
    let path = root.join(edit["file"].as_str().unwrap());
    let text = std::fs::read_to_string(&path).unwrap();
    let start = offset(&text, &edit["start"]);
    let end = offset(&text, &edit["end"]);
    let replacement = edit["replacement"].as_str().unwrap();
    let edited = format!("{}{replacement}{}", &text[..start], &text[end..]);
    std::fs::write(&path, edited).unwrap();
}

#[test]
fn an_agent_repairs_and_verifies_a_package_through_json_alone() {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("headless");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        "[package]\nname = \"clocks\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n",
    )
    .unwrap();
    std::fs::copy(
        repository().join("verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(
        root.join("src/clocks.una"),
        "module clocks

pub capability Clock
  op now() -> Int
    ensures result >= 0

pub stamp() -> Int
  ensures result >= 0
  uses @clock: Clock[now]
=
  @clock.now()

pub later() -> Int
  ensures result > 0
=
  let t = stamp()
  t + 1

handler fixed(at: Nat) for Clock
  op now() = at

example \"a second after midnight\"
  given @clock = fixed(at: 1)
  do later()
  expect result == 2
",
    )
    .unwrap();

    // 1. Check: one diagnostic, with a mechanical edit.
    let (code, document) = una(&root, &["check"]);
    assert_eq!(code, 1);
    assert_eq!(document["schema"], "una.diagnostics/1");
    let diagnostics = document["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "{document}");
    assert_eq!(diagnostics[0]["code"], "U3003");
    let edit = &diagnostics[0]["repairs"][0]["edit"];
    assert_eq!(edit["replacement"], "\n  uses @clock: Clock[now]");

    // 2. Apply the edit; the package now checks at tier 1.
    apply(&root, edit);
    let (code, document) = una(&root, &["check"]);
    assert_eq!(
        (code, document),
        (0, json!({"schema": "una.diagnostics/1", "diagnostics": []}))
    );

    // 3. The examples pass, every obligation is proven, and the report sees no gap.
    let (code, tested) = una(&root, &["test"]);
    assert_eq!(code, 0);
    assert_eq!(
        (tested["passed"].clone(), tested["failed"].clone()),
        (json!(1), json!(0))
    );
    let (code, listed) = una(&root, &["obligations"]);
    assert_eq!(code, 0);
    let statuses: Vec<&Value> = listed["obligations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| &o["status"])
        .collect();
    assert!(
        !statuses.is_empty() && statuses.iter().all(|s| *s == "proven"),
        "{listed}"
    );
    let (code, report) = una(&root, &["report"]);
    assert_eq!(code, 0);
    assert_eq!(report["obligations"]["refuted"], 0);
    assert_eq!(report["coverage"]["trivial_ensures"], 0);
    assert_eq!(
        report["assumptions"],
        json!([{"capability": "clocks.Clock", "operations": ["now"], "ensures": ["now: result >= 0"]}])
    );

    // 4. The contract of the repaired function shows the clause the edit added.
    let (code, contract) = una(&root, &["contract", "clocks.later"]);
    assert_eq!(code, 0);
    assert_eq!(contract["schema"], "una.contract/1");
    assert_eq!(
        contract["uses"],
        json!([{"cap": "clock", "capability": "Clock", "ops": ["now"], "inferred": false}])
    );

    // 5. A usage error is the only output outside a document: exit code 2 and stderr.
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        // The proof cache's key lives under this home, never the user's (#1164).
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(["--json", "check", "no-such-package"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"");
}
