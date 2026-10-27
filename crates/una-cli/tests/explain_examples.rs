//! Every explain text with examples is checked: its failing example yields its own code and
//! nothing else, and its fixed example yields nothing, at tier 1, so with the solver
//! (`spec/13 §4`, approved M1 plan §5, review finding R-037).
//!
//! An example is one `una` block, or a package whose files are fenced blocks
//! each introduced by its path in backticks on its own line; the test completes a package with
//! a default `una.toml` and the repository's `verification.lock` unless it gives its own, or
//! gives an `ita.una` (edition 0.4).

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

/// The flags of `una run` that the example under `heading` names, in a line with
/// ``Run it with `una run <flags>` ``: a runtime code that only a flag causes, such as U6007
/// after `--deadline` (ADR 0116).
fn run_flags(text: &str, heading: &str) -> Vec<String> {
    let Some(section) = text
        .split(heading)
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
    else {
        return Vec::new();
    };
    section
        .lines()
        .find_map(|line| line.strip_prefix("Run it with `una run "))
        .and_then(|rest| rest.split('`').next())
        .map(|flags| flags.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The headings of the fixed examples of `text`, in order: `## Fixed example`, and
/// `## Fixed example: <case>` for each further case.
fn fixed_headings(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| line.starts_with("## Fixed example"))
        .map(|line| format!("{}\n", line.trim_end()))
        .collect()
}

/// The codes `check` reports for `example`, written under a directory named after the code.
/// A fixed example is checked at tier 1, a failing one at the tier of its code.
fn codes(code: Code, name: &str, example: &Example, fixed: bool, flags: &[String]) -> Vec<Code> {
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
            // A package of edition 0.4 gives its own `ita.una` and `ita.lock` (ADR 0122).
            let ita = given("ita.una");
            if !ita && !given("una.toml") {
                std::fs::write(dir.join("una.toml"), MANIFEST).unwrap();
            }
            if !ita && !given("verification.lock") {
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
        let mut args = vec!["una", "--json", "run"];
        args.extend(flags.iter().map(String::as_str));
        args.push(target.to_str().unwrap());
        una_cli::run(
            args,
            &mut stdout,
            &mut stderr,
            false,
            &|_| Ok(None),
            una_cli::Heap::unmeasured(),
        );
        let document: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
        return document["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|diagnostic| diagnostic["code"].as_str().unwrap().parse().unwrap())
            .collect();
    }
    // Obligations need tier 1; every other failing example is checked at tier 0, where a
    // refuted obligation of the example cannot add a second code.
    let tier = if fixed || code.domain() == Domain::Verification {
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
        let text = una_explain::text(code);
        let (Some(failing), Some(_)) = (
            example(text, "## Failing example"),
            example(text, "## Fixed example"),
        ) else {
            continue;
        };
        let reported = codes(
            code,
            "failing",
            &failing,
            false,
            &run_flags(text, "## Failing example"),
        );
        assert!(
            !reported.is_empty() && reported.iter().all(|found| *found == code),
            "{code}: the failing example reports {reported:?}"
        );
        // Every fixed example, as `## Fixed example` and `## Fixed example: <case>` (#404).
        for (n, heading) in fixed_headings(text).iter().enumerate() {
            let fixed = example(text, heading).unwrap();
            assert_eq!(
                codes(
                    code,
                    &format!("fixed{n}"),
                    &fixed,
                    true,
                    &run_flags(text, heading)
                ),
                [],
                "{code}: {heading}"
            );
        }
        checked.push(code.as_str());
    }
    // Every code has its examples, but U8001, an internal error, which no program should
    // cause, U6005, which the host's data causes and no program text does (ADR 0086), and
    // U6006, which the platform's floating-point environment causes (ADR 0114), U6009, which
    // the host's random source causes (ADR 0119 rule 9), and U7013, which only `una ita`
    // reports when it resolves, and `una check` never does (ADR 0122 rule 25). U7010 and U7011
    // are reachable only with a registry need, which this release refuses (U7008), so a locked
    // registry package is U7009 too and no example can report the code alone: remove with #998,
    // which lets registry needs load and writes their examples. U2039 and U3032 belong to the
    // `simd required` clause of ADR 0120 rule 58, which this release does not read yet: remove
    // with ADR 0120 S8 (#991), which lands the clause and writes their examples.
    for code in Code::ALL {
        if code == Code::U8001
            || code == Code::U2039
            || code == Code::U3032
            || code == Code::U6005
            || code == Code::U6006
            || code == Code::U6009
            || code == Code::U7010
            || code == Code::U7011
            || code == Code::U7013
        {
            assert!(
                !una_explain::text(code).contains("Stub explanation."),
                "{code} has its text"
            );
            continue;
        }
        assert!(checked.contains(&code.as_str()), "{code} has no examples");
    }
}
