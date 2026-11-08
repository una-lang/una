//! The report's `task switch` entries (`spec/13 §6`, `spec/27 §8` rule 5, #1708) over every
//! refused `task_switch_*` conformance case: its `internal` twin, `secret` relabeled
//! `internal`, is accepted and lists a `task switch` at `internal` exactly where the case is
//! U4002, and its `public` twin, the labels removed, lists none (a package with a `serve`
//! section has none: its requests are `internal`). No native build. Every package
//! lies below a sealed directory (#1560).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/hermetic.rs"]
mod hermetic;

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

/// A site, as `file:line:col`.
type Sites = BTreeSet<String>;

/// Copies the package `from` to `to`, replacing `secret ` by `with` in its modules.
fn twin(from: &Path, to: &Path, with: &str) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            twin(&path, &target, with);
        } else if path.extension().is_some_and(|e| e == "una")
            && path.file_name().unwrap() != "ita.una"
        {
            let text = std::fs::read_to_string(&path).unwrap();
            std::fs::write(&target, text.replace("secret ", with)).unwrap();
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

/// The U4002 sites that the case's `-- expect:` lines name.
fn refused(case: &Path) -> Sites {
    let mut sites = Sites::new();
    let src = case.join("src");
    for entry in std::fs::read_dir(&src).unwrap() {
        let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        for line in text.lines() {
            if let Some(site) = line.strip_prefix("-- expect: error U4002 at ") {
                sites.insert(site.trim().to_owned());
            }
        }
    }
    sites
}

/// The `task switch` sites of the report of the package in `dir`, each at `internal`.
fn listed(dir: &Path, name: &str) -> Sites {
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("NO_COLOR", "1")
        .args(["--json", "report", "."])
        .current_dir(dir)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{name}: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut sites = Sites::new();
    for entry in report["information"]["timing"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if entry["construct"] != "task switch" {
            continue;
        }
        assert_eq!(entry["label"], "internal", "{name}: {entry}");
        assert!(
            !entry["function"].as_str().unwrap().is_empty(),
            "{name}: {entry}"
        );
        let site = &entry["site"];
        sites.insert(format!(
            "{}:{}:{}",
            site["file"].as_str().unwrap(),
            site["start"]["line"],
            site["start"]["col"]
        ));
    }
    sites
}

#[test]
fn the_internal_twin_of_each_refused_case_lists_its_sites_and_the_public_one_none() {
    let security = hermetic::repository().join("tests/conformance/security");
    let base = std::env::temp_dir().join(format!("una-switch-report-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    hermetic::seal(&base);
    let mut cases = 0;
    let mut names: Vec<String> = std::fs::read_dir(&security)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("task_switch_"))
        .collect();
    names.sort();
    for name in names {
        let case = security.join(&name);
        let sites = refused(&case);
        if sites.is_empty() {
            continue;
        }
        cases += 1;
        let internal = base.join(format!("{name}_internal"));
        twin(&case, &internal, "internal ");
        assert_eq!(listed(&internal, &name), sites, "{name}: internal twin");
        // A request is `internal` whatever the code says (`spec/31 §1`), so a package with a
        // `serve` section has no `public` twin.
        let manifest = std::fs::read_to_string(case.join("ita.una")).unwrap();
        if manifest.lines().any(|line| line == "serve") {
            continue;
        }
        let public = base.join(format!("{name}_public"));
        twin(&case, &public, "");
        assert_eq!(listed(&public, &name), Sites::new(), "{name}: public twin");
    }
    // The 12 refused cases of #1684 and the 4 shapes of #1708.
    assert!(cases >= 16, "{cases} refused cases");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn an_entry_in_a_handler_s_operation_body_names_the_handler() {
    // Review L1 of #1742: a check point in an operation body lies outside every function, and
    // its entry names the handler, `m.h` (`spec/13 §6`), never an empty name.
    let case = hermetic::repository().join("tests/conformance/runtime/operations_under_secret");
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("NO_COLOR", "1")
        .args(["--json", "report", "."])
        .current_dir(&case)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let entry = report["information"]["timing"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["site"]["start"]["line"] == 19)
        .unwrap();
    assert_eq!(entry["construct"], "task switch");
    assert_eq!(entry["function"], "operations.box");
}

#[test]
fn a_check_point_that_only_a_test_reaches_is_not_listed() {
    // #1707: code that no run of the program reaches is neither refused nor listed. The
    // `internal` twin of `task_switch_example_only` lists no `task switch`, while that of its
    // twin `task_switch_reached`, whose task calls the same function, lists the loop.
    let security = hermetic::repository().join("tests/conformance/security");
    let base = std::env::temp_dir().join(format!("una-switch-reach-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    hermetic::seal(&base);
    let only = base.join("example_only_internal");
    twin(
        &security.join("task_switch_example_only"),
        &only,
        "internal ",
    );
    assert_eq!(listed(&only, "task_switch_example_only"), Sites::new());
    let reached = base.join("reached_internal");
    twin(&security.join("task_switch_reached"), &reached, "internal ");
    assert_eq!(
        listed(&reached, "task_switch_reached"),
        refused(&security.join("task_switch_reached"))
    );
    let _ = std::fs::remove_dir_all(&base);
}
