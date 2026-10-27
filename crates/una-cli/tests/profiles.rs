//! ADR 0109 rule 2: `systems` is stricter than or equal to `app`. Every conformance package of
//! editions `0.3` and `0.4` under `tests/conformance/verification` and `tests/conformance/runtime`
//! is checked as written and again with its profile forced to `systems`, with the budget
//! `max_memory 64` added where its `run` section has an entry (rule 4): the second check keeps
//! every diagnostic of the first and adds only `overflow` diagnostics (U5001, U5002, rule 3).
//! A runtime package that both profiles accept runs under both, with the same budget, and gives
//! the same document: output, failures and trace (rule 2.2).
//!
//! The test runs in the ordinary test run, as `editions.rs` does: it checks each of about 120
//! packages twice, on four workers, and runs each runtime package twice. Linear in the packages.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// The files of a package's manifest and lock: a line inserted there moves their positions, so
/// their diagnostics are compared by code only.
const MANIFEST_FILES: [&str; 4] = ["una.toml", "verification.lock", "ita.una", "ita.lock"];

/// The budget added to a package with an entry, in MiB.
const BUDGET: u64 = 64;

fn conformance() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        // A proof cache or a build of the repository's copy is never carried over.
        if path.file_name().is_some_and(|name| name == ".una") {
            continue;
        }
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

/// How a package's manifest is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    /// `una.toml` of edition `0.3`.
    Toml,
    /// `ita.una` of edition `0.4`.
    Ita,
}

/// The packages of editions `0.3` and `0.4` of the areas, in a stable order, each with its
/// manifest format.
fn packages() -> Vec<(String, PathBuf, Format)> {
    let root = conformance();
    let mut out = Vec::new();
    for area in ["verification", "runtime"] {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(root.join(area))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_dir())
            .filter(|path| !path.file_name().unwrap().to_string_lossy().starts_with('.'))
            .collect();
        entries.sort();
        for entry in entries {
            let name = format!("{area}/{}", entry.file_name().unwrap().to_string_lossy());
            let toml = std::fs::read_to_string(entry.join("una.toml")).unwrap_or_default();
            let ita = std::fs::read_to_string(entry.join("ita.una")).unwrap_or_default();
            if toml.lines().any(|line| line.trim() == "edition = \"0.3\"") {
                out.push((name, entry, Format::Toml));
            } else if ita.lines().any(|line| line.trim() == "edition 0.4") {
                out.push((name, entry, Format::Ita));
            }
        }
    }
    out
}

/// The manifest `text` with `max_memory` added after the entry of its `run` section when it has
/// an entry and no budget, and with the profile `systems` when `systems` holds, `app`, the
/// default, otherwise: a declared profile line is dropped. `None` when the
/// manifest names a profile other than `app` or `systems`: a case of the manifest's own errors,
/// which forcing a profile would hide.
fn edited(text: &str, format: Format, systems: bool) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let (edition, profile_prefix, run_header, entry_prefix, budget_prefix, budget, profile) =
        match format {
            Format::Toml => (
                "edition = \"0.3\"",
                "profile",
                "[run]",
                "entry",
                "max_memory",
                format!("max_memory = {BUDGET}"),
                "profile = \"systems\"",
            ),
            Format::Ita => (
                "edition 0.4",
                "profile ",
                "run",
                "entry ",
                "max_memory",
                format!("  max_memory {BUDGET}"),
                "  profile systems",
            ),
        };
    // The declared profile, if any: only `app` and `systems` are rewritten.
    for line in &lines {
        let trimmed = line.trim();
        if trimmed.starts_with(profile_prefix) {
            let value = trimmed
                .trim_start_matches(profile_prefix)
                .trim()
                .trim_start_matches('=')
                .trim()
                .trim_matches('"');
            if value != "app" && value != "systems" {
                return None;
            }
        }
    }
    // The `run` section: from its header to the next unindented line (`ita.una`) or table
    // (`una.toml`).
    let run_start = lines.iter().position(|line| line.trim_end() == run_header);
    let run_end = run_start.map(|start| {
        lines
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, line)| match format {
                Format::Toml => line.trim_start().starts_with('['),
                Format::Ita => !line.is_empty() && !line.starts_with(' '),
            })
            .map_or(lines.len(), |(index, _)| index)
    });
    let entry = run_start.zip(run_end).and_then(|(start, end)| {
        let section = &lines[start + 1..end];
        let has_budget = section
            .iter()
            .any(|line| line.trim().starts_with(budget_prefix));
        section
            .iter()
            .position(|line| line.trim().starts_with(entry_prefix))
            .filter(|_| !has_budget)
            .map(|index| start + 1 + index)
    });
    let mut out = String::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        // A declared `app` or `systems` gives way to the forced profile: `systems` below, or
        // the default, `app`.
        if trimmed.starts_with(profile_prefix) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
        if systems && trimmed == edition {
            out.push_str(profile);
            out.push('\n');
        }
        if Some(index) == entry {
            out.push_str(&budget);
            out.push('\n');
        }
    }
    Some(out)
}

/// A fresh copy of `package` at `work` with its manifest edited as [`edited`] says. `None` when
/// the manifest is not edited.
fn prepared(package: &Path, work: &Path, format: Format, systems: bool) -> Option<()> {
    let file = match format {
        Format::Toml => "una.toml",
        Format::Ita => "ita.una",
    };
    let text = std::fs::read_to_string(package.join(file)).unwrap();
    let text = edited(&text, format, systems)?;
    if work.exists() {
        std::fs::remove_dir_all(work).unwrap();
    }
    copy_dir(package, work);
    std::fs::write(work.join(file), text).unwrap();
    Some(())
}

/// `una --json <args>` in `work`: its exit status and its document.
fn una(work: &Path, args: &[String]) -> (Option<i32>, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .arg("--json")
        .args(args)
        .current_dir(work)
        .output()
        .unwrap();
    let document: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{}: {args:?} printed no document ({error}): status {:?}, stdout {:?}, stderr {:?}",
            work.display(),
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.code(), document)
}

/// The diagnostics of a document: outside the manifest files, by code and `file:line:col` with
/// whether each is an `overflow` one; in them, by code only.
#[derive(Debug, Default, PartialEq, Eq)]
struct Diagnostics {
    sites: BTreeMap<(String, String), bool>,
    manifest: Vec<String>,
}

fn diagnostics(document: &Value) -> Diagnostics {
    let mut out = Diagnostics::default();
    for diagnostic in document["diagnostics"].as_array().into_iter().flatten() {
        let code = diagnostic["code"].as_str().unwrap().to_owned();
        let primary = &diagnostic["primary"];
        let file = primary["file"].as_str().unwrap_or("");
        if MANIFEST_FILES.contains(&file) {
            out.manifest.push(code);
            continue;
        }
        let at = format!(
            "{file}:{}:{}",
            primary["start"]["line"], primary["start"]["col"]
        );
        let overflow = diagnostic["obligation"]["kind"] == "overflow";
        out.sites.insert((code, at), overflow);
    }
    out.manifest.sort();
    out
}

/// The directive lines of the case: the leading comment lines of its modules.
fn directives(package: &Path) -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "una") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    if package.join("src").is_dir() {
        walk(&package.join("src"), &mut files);
    }
    files.sort();
    let mut out = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        out.extend(
            text.lines()
                .take_while(|line| line.starts_with("--"))
                .map(str::to_owned),
        );
    }
    out
}

/// Whether the conformance runner runs the case (`docs/compiler/testing.md` §2): it expects
/// output, grants a capability or expects an error of the runtime domain. Any other entry is
/// not run here either, as a server that never stops would not end.
fn runner_runs(package: &Path) -> bool {
    directives(package).iter().any(|line| {
        line.starts_with("-- expect: run-output")
            || line.starts_with("-- grant: ")
            || line.starts_with("-- expect: error U6")
    })
}

/// The `--grant` arguments of the case's `-- grant:` directives, paths resolved against `work`.
fn grants(package: &Path, work: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for line in directives(package) {
        let Some(spec) = line.strip_prefix("-- grant: ") else {
            continue;
        };
        let spec = spec.trim();
        let anchored = if let Some(rest) = spec.strip_prefix("keys:") {
            format!("keys:{}", work.join(rest).display())
        } else if let Some((path, modes)) = spec
            .strip_prefix("files:")
            .and_then(|rest| rest.rsplit_once(':'))
        {
            format!("files:{}:{modes}", work.join(path).display())
        } else {
            spec.to_owned()
        };
        out.push("--grant".to_owned());
        out.push(anchored);
    }
    out
}

/// Whether the manifest of `package` has a `run` section with an entry.
fn runs(package: &Path, format: Format) -> bool {
    match format {
        Format::Toml => std::fs::read_to_string(package.join("una.toml"))
            .unwrap()
            .lines()
            .skip_while(|line| line.trim() != "[run]")
            .skip(1)
            .take_while(|line| !line.trim_start().starts_with('['))
            .any(|line| line.trim().starts_with("entry")),
        Format::Ita => std::fs::read_to_string(package.join("ita.una"))
            .unwrap()
            .lines()
            .skip_while(|line| line.trim_end() != "run")
            .skip(1)
            .take_while(|line| line.is_empty() || line.starts_with(' '))
            .any(|line| line.trim().starts_with("entry ")),
    }
}

/// Compares `package` under both profiles: `Ok(true)` when it was compared, `Ok(false)` when its
/// manifest names another profile, and `Err` with what differs.
fn compare(package: &Path, format: Format, work: &Path) -> Result<bool, String> {
    let check = vec!["check".to_owned(), ".".to_owned()];
    // The package as written, with the budget only: the same manifest values but the profile.
    if prepared(package, work, format, false).is_none() {
        return Ok(false);
    }
    let (_, app) = una(work, &check);
    let app = diagnostics(&app);
    prepared(package, work, format, true).unwrap();
    let (_, systems) = una(work, &check);
    let systems = diagnostics(&systems);
    let mut wrong = Vec::new();
    if app.manifest != systems.manifest {
        wrong.push(format!(
            "manifest diagnostics {:?} became {:?}",
            app.manifest, systems.manifest
        ));
    }
    for site in app.sites.keys() {
        if !systems.sites.contains_key(site) {
            wrong.push(format!("`systems` lost {} at {}", site.0, site.1));
        }
    }
    let mut added = false;
    for (site, overflow) in &systems.sites {
        if app.sites.contains_key(site) {
            continue;
        }
        added = true;
        let allowed = *overflow && (site.0 == "U5001" || site.0 == "U5002");
        if !allowed {
            wrong.push(format!(
                "`systems` added {} at {}, which is not an `overflow` diagnostic",
                site.0, site.1
            ));
        }
    }
    // Rule 2.2: a runtime package that both profiles accept runs the same under both.
    if wrong.is_empty() && !added && runs(package, format) && runner_runs(package) {
        let mut run = vec!["run".to_owned(), ".".to_owned()];
        prepared(package, work, format, false).unwrap();
        run.extend(grants(package, work));
        let first = una(work, &run);
        prepared(package, work, format, true).unwrap();
        let second = una(work, &run);
        if first != second {
            // A run whose document differs from itself, through the clock or the order of
            // tasks, is no evidence either way: it runs under `app` once more.
            prepared(package, work, format, false).unwrap();
            let again = una(work, &run);
            if again == first {
                wrong.push(format!(
                    "`una run` differs: {:?} under `app`, {:?} under `systems`",
                    first, second
                ));
            }
        }
    }
    if work.exists() {
        std::fs::remove_dir_all(work).unwrap();
    }
    if wrong.is_empty() {
        Ok(true)
    } else {
        Err(wrong.join("; "))
    }
}

#[test]
fn the_edited_manifests_force_the_profile_and_add_the_budget() {
    let toml = "[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n\n[run]\nentry = \"main\"\ngrants = [\"console\"]\n";
    assert_eq!(
        edited(toml, Format::Toml, true).unwrap(),
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"0.3\"\nprofile = \"systems\"\n\n[run]\nentry = \"main\"\nmax_memory = 64\ngrants = [\"console\"]\n"
    );
    assert_eq!(
        edited(toml, Format::Toml, false).unwrap(),
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n\n[run]\nentry = \"main\"\nmax_memory = 64\ngrants = [\"console\"]\n"
    );
    let ita = "ita case\n  version 0.1.0\n  edition 0.4\n  profile app\n\nrun\n  entry m\n  grants\n    console\n";
    assert_eq!(
        edited(ita, Format::Ita, true).unwrap(),
        "ita case\n  version 0.1.0\n  edition 0.4\n  profile systems\n\nrun\n  entry m\n  max_memory 64\n  grants\n    console\n"
    );
    let budgeted = "ita case\n  version 0.1.0\n  edition 0.4\n  profile systems\n\nrun\n  entry main\n  max_memory 64\n";
    assert_eq!(edited(budgeted, Format::Ita, true).unwrap(), budgeted);
    assert_eq!(
        edited(budgeted, Format::Ita, false).unwrap(),
        "ita case\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry main\n  max_memory 64\n"
    );
    assert_eq!(
        edited(
            "ita case\n  edition 0.4\n  profile kernel\n",
            Format::Ita,
            true
        ),
        None
    );
}

#[test]
fn systems_only_adds_overflow_obligations_and_changes_no_run() {
    let cases = packages();
    assert!(
        cases.len() >= 100,
        "only {} packages of editions 0.3 and 0.4",
        cases.len()
    );
    let workers = 4;
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("profiles");
    let results: Vec<(String, Result<bool, String>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                let cases = &cases;
                let work = scratch.join(format!("w{worker}"));
                scope.spawn(move || {
                    cases
                        .iter()
                        .skip(worker)
                        .step_by(workers)
                        .map(|(name, package, format)| {
                            (name.clone(), compare(package, *format, &work))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    });
    let compared = results
        .iter()
        .filter(|(_, result)| matches!(result, Ok(true)))
        .count();
    let mut wrong: Vec<String> = results
        .into_iter()
        .filter_map(|(name, result)| result.err().map(|why| format!("{name}: {why}")))
        .collect();
    wrong.sort();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(compared >= 100, "only {compared} packages compared");
}
