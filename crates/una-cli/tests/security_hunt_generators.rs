//! The security hunt's generators (`scripts/security-hunt`) write programs this `una` must read:
//! a generated program refused with a lexical or syntax code it does not expect (U0xxx) is a
//! generator error, which the hunt gate reports only at release validation (#1911: S2's function
//! was named `test`, a keyword from edition 0.3). This guard runs in the ordinary suite: the
//! first seeds of every operator of the gate's fixed run, in every edition the gate runs them in
//! (`hunt gen --editions gate`, the operator's editions and the edition 0.3 sample), checked at
//! tier 0 as the must-reject driver checks them. Only the codes decide, never the verdict: the
//! hunt's judges own that. Programs the gate judges by another command (`lock-check`) are
//! skipped, since `una check` is not their judge.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

/// The gate's campaign seed (`hunt.gate.SEED`) and how many of each operator's seeds run here.
const SEED: &str = "767";
const COUNT: &str = "3";
const WORKERS: usize = 4;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("una-hunt-generators-{}", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A program's unexpected syntax codes, as the gate's `generator_errors` counts them: the error
/// codes `U0...` of `una --json check --tier 0` that `expect` does not name, matched by code.
fn unexpected_syntax(dir: &Path, expect: &BTreeSet<String>) -> Result<Vec<String>, String> {
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .args(["--json", "check", "--tier", "0", "."])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("cannot run una: {e}"))?;
    let doc: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| {
        format!(
            "exit {:?}, not a document ({e}): {}{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })?;
    let codes: BTreeSet<String> = doc["diagnostics"]
        .as_array()
        .map(|ds| {
            ds.iter()
                .filter(|d| d["severity"] == "error")
                .filter_map(|d| d["code"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    Ok(codes
        .into_iter()
        .filter(|c| c.starts_with("U0") && !expect.contains(c))
        .collect())
}

#[test]
fn the_gates_generated_programs_have_no_unexpected_syntax_code_in_any_of_their_editions() {
    let out = scratch();
    let generated = Command::new("python3")
        .args(["-B", "-m", "hunt", "gen", "--count", COUNT, "--seed", SEED])
        .args(["--editions", "gate", "--out"])
        .arg(&out)
        .current_dir(root().join("scripts/security-hunt"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "hunt gen: {}{}",
        String::from_utf8_lossy(&generated.stdout),
        String::from_utf8_lossy(&generated.stderr)
    );
    let manifest = std::fs::read_to_string(out.join("manifest.jsonl")).unwrap();
    let rows: Vec<serde_json::Value> = manifest
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .filter(|row: &serde_json::Value| row["command"].as_str().unwrap_or("check") == "check")
        .collect();
    assert!(rows.len() > 400, "{} programs generated", rows.len());
    let editions: BTreeSet<&str> = rows.iter().filter_map(|r| r["edition"].as_str()).collect();
    assert!(
        editions.contains("0.3"),
        "the sample has no edition 0.3 program: {editions:?}"
    );
    let problems = Mutex::new(Vec::new());
    let next = Mutex::new(0usize);
    std::thread::scope(|scope| {
        for _ in 0..WORKERS {
            scope.spawn(|| {
                loop {
                    let i = {
                        let mut n = next.lock().unwrap();
                        let i = *n;
                        *n += 1;
                        i
                    };
                    let Some(row) = rows.get(i) else { break };
                    let name = row["name"].as_str().unwrap();
                    let expect: BTreeSet<String> = row["expect"]
                        .as_array()
                        .map(|xs| {
                            xs.iter()
                                .filter_map(|x| x.as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default();
                    match unexpected_syntax(&out.join(name), &expect) {
                        Ok(codes) if codes.is_empty() => {}
                        Ok(codes) => problems
                            .lock()
                            .unwrap()
                            .push(format!("{name}: a generator error, {}", codes.join(", "))),
                        Err(why) => problems.lock().unwrap().push(format!("{name}: {why}")),
                    }
                }
            });
        }
    });
    let mut problems = problems.into_inner().unwrap();
    problems.sort();
    assert!(
        problems.is_empty(),
        "{} of {} generated programs are refused with an unexpected syntax code:\n{}",
        problems.len(),
        rows.len(),
        problems.join("\n")
    );
    std::fs::remove_dir_all(&out).unwrap();
}
