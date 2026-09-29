//! Every explain text with examples is checked: its failing example yields its own code and
//! nothing else, and its fixed example yields nothing (`spec/13 §4`, approved M1 plan §5).
//!
//! DECISION D-043: an example is one `una` block, or a package whose files are fenced blocks
//! each introduced by its path in backticks on its own line; the test completes a package with
//! a default `una.toml` and the repository's `verification.lock` unless it gives its own.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use pretty_assertions::assert_eq;
use una_cli::Tier;
use una_diag::{Code, Domain};

const MANIFEST: &str = "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n";

/// An example: one file, or the files of a package by path.
enum Example {
    File(String),
    Package(Vec<(String, String)>),
}

/// The example under `heading` in `text`, which ends at the next `## ` heading.
fn example(text: &str, heading: &str) -> Option<Example> {
    let section = text.split(heading).nth(1)?.split("\n## ").next()?;
    let mut files = Vec::new();
    let mut path: Option<String> = None;
    let mut lines = section.lines();
    while let Some(line) = lines.next() {
        let line = line.trim_end();
        if line.len() > 2
            && line.starts_with('`')
            && line.ends_with('`')
            && !line.starts_with("```")
        {
            path = Some(line.trim_matches('`').to_owned());
        } else if line.starts_with("```") {
            let mut body = String::new();
            for inner in lines.by_ref() {
                if inner.starts_with("```") {
                    break;
                }
                body.push_str(inner);
                body.push('\n');
            }
            files.push((path.take(), body));
        }
    }
    match files.as_slice() {
        [] => None,
        [(None, body)] => Some(Example::File(body.clone())),
        _ => Some(Example::Package(
            files
                .into_iter()
                .map(|(path, body)| {
                    (
                        path.expect("every file of a package example has a path"),
                        body,
                    )
                })
                .collect(),
        )),
    }
}

/// The codes `check` reports for `example`, written under a directory named after the code.
fn codes(code: Code, name: &str, example: &Example) -> Vec<Code> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("explain-examples")
        .join(code.as_str())
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let target = match example {
        Example::File(source) => {
            let file = dir.join(format!("{name}.una"));
            std::fs::write(&file, source).unwrap();
            file
        }
        Example::Package(files) => {
            let given = |path: &str| files.iter().any(|(file, _)| file == path);
            if !given("una.toml") {
                std::fs::write(dir.join("una.toml"), MANIFEST).unwrap();
            }
            if !given("verification.lock") {
                let lock = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verification.lock");
                std::fs::copy(lock, dir.join("verification.lock")).unwrap();
            }
            for (path, body) in files {
                let file = dir.join(path);
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(file, body).unwrap();
            }
            dir.clone()
        }
    };
    // Runtime codes come from `una run`, whose JSON document lists them with the diagnostics
    // of the checks.
    if code.domain() == Domain::Runtime {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let args = ["una", "--json", "run", target.to_str().unwrap()];
        una_cli::run(args, &mut stdout, &mut stderr, false, &|_| None);
        let document: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
        return document["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|diagnostic| diagnostic["code"].as_str().unwrap().parse().unwrap())
            .collect();
    }
    // Obligations need tier 1; every other code is checked at tier 0.
    let tier = if code.domain() == Domain::Verification {
        Tier::One
    } else {
        Tier::Zero
    };
    let checked = una_cli::check(&target, tier).unwrap();
    checked
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

#[test]
fn failing_examples_yield_their_code_and_fixed_examples_yield_nothing() {
    let mut checked = Vec::new();
    for code in Code::ALL {
        let text = code.explain_text();
        let (Some(failing), Some(fixed)) = (
            example(text, "## Failing example"),
            example(text, "## Fixed example"),
        ) else {
            continue;
        };
        let reported = codes(code, "failing", &failing);
        assert!(
            !reported.is_empty() && reported.iter().all(|found| *found == code),
            "{code}: the failing example reports {reported:?}"
        );
        assert_eq!(codes(code, "fixed", &fixed), [], "{code}: fixed example");
        checked.push(code.as_str());
    }
    // DECISION D-126: every code has its examples, but U8001, an internal error, which no
    // program should cause.
    for code in Code::ALL {
        if code == Code::U8001 {
            assert!(
                !code.explain_text().contains("Stub explanation."),
                "U8001 has its text"
            );
            continue;
        }
        assert!(checked.contains(&code.as_str()), "{code} has no examples");
    }
}
