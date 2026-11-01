//! `una check` under an outer sandbox on macOS (ADR 0124 rule 7, Question 6, #1360): under a
//! profile that meets Question 6's minimum, as the toolchain-free gate's does, the solver
//! children take the outer sandbox as their confinement and every obligation of the events
//! example is proven; under a profile short of that minimum no child answers, and the run
//! fails with U5003. Tier 1: it needs the solver built into `una`.
#![cfg(target_os = "macos")]
// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn copy_package(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == ".una" {
            continue;
        }
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            copy_package(&entry.path(), &to.join(&name));
        } else if kind.is_file() {
            std::fs::copy(entry.path(), to.join(&name)).unwrap();
        }
    }
}

/// A run's own directory, holding a fresh copy of the events example (no proof cache, so
/// that the solver runs), HOME and TMPDIR, all by their resolved paths.
fn fresh(label: &str) -> PathBuf {
    let own = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("outer-sandbox-{label}"));
    let _ = std::fs::remove_dir_all(&own);
    std::fs::create_dir_all(own.join("home")).unwrap();
    std::fs::create_dir_all(own.join("tmp")).unwrap();
    let own = own.canonicalize().unwrap();
    copy_package(&repository().join("examples/events"), &own.join("events"));
    own
}

/// `una args` under `sandbox-exec` with `profile` and an empty PATH, from the repository's
/// root, where the profile denies writes (the path outside the temporary directory that the
/// child checks), as `scripts/toolchain-free.sh` runs it. `None` when this process is itself
/// under a sandbox that refuses a nested one.
fn una_sandboxed(own: &Path, profile: &str, args: &[&str]) -> Option<Output> {
    let output = Command::new("/usr/bin/sandbox-exec")
        .arg("-p")
        .arg(profile)
        .arg(env!("CARGO_BIN_EXE_una"))
        .args(args)
        .env_clear()
        .env("HOME", own.join("home"))
        .env("TMPDIR", own.join("tmp"))
        .env("PATH", own.join("tmp"))
        .current_dir(repository())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.starts_with("sandbox-exec: sandbox_apply") {
        let _ = writeln!(
            std::io::stderr(),
            "this test runs under a sandbox that refuses a nested one: {stderr}"
        );
        return None;
    }
    Some(output)
}

/// The toolchain-free gate's profile for `own`: the network denied, writes only in `own`,
/// and no program started but `una`.
fn compliant(own: &Path) -> String {
    let una = Path::new(env!("CARGO_BIN_EXE_una")).canonicalize().unwrap();
    format!(
        "(version 1)(allow default)(deny network*)(deny file-write*)\
         (allow file-write* (subpath \"{}\"))\
         (allow file-write* (literal \"/dev/null\") (literal \"/dev/tty\") (literal \"/dev/dtracehelper\"))\
         (deny process-exec*)(allow process-exec* (literal \"{}\"))",
        own.display(),
        una.display()
    )
}

fn diagnostics(output: &Output) -> Vec<Value> {
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    json["diagnostics"].as_array().cloned().unwrap_or_default()
}

#[test]
fn under_a_compliant_outer_sandbox_every_obligation_is_proven() {
    let own = fresh("compliant");
    let package = own.join("events");
    let Some(check) = una_sandboxed(
        &own,
        &compliant(&own),
        &["--json", "check", package.to_str().unwrap()],
    ) else {
        return;
    };
    let text = String::from_utf8_lossy(&check.stdout).into_owned();
    // #1360: before the fix every obligation ended as `unknown (crash)`, U5002.
    assert_eq!(check.status.code(), Some(0), "{text}");
    assert!(diagnostics(&check).is_empty(), "{text}");
    // The report of another fresh copy starts the solver again and names the confinement.
    std::fs::remove_dir_all(own.join("events/.una")).ok();
    let report = una_sandboxed(
        &own,
        &compliant(&own),
        &["--json", "report", package.to_str().unwrap()],
    )
    .unwrap();
    let text = String::from_utf8_lossy(&report.stdout).into_owned();
    assert_eq!(report.status.code(), Some(0), "{text}");
    let json: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["tcb"]["solver_confinement"], "outer", "{text}");
    let denied = json["tcb"]["solver_outer_denied"].to_string();
    for op in [
        "network-outbound",
        "network-inbound",
        "file-write-data",
        "file-write-unlink",
    ] {
        assert!(denied.contains(op), "{denied}");
    }
    assert!(
        json["obligations"]["proven"]
            .as_u64()
            .is_some_and(|n| n > 0),
        "{text}"
    );
    assert_eq!(json["obligations"]["unknown"], 0, "{text}");
}

#[test]
fn under_a_loose_outer_sandbox_no_obligation_is_proven_and_the_run_fails_with_u5003() {
    let own = fresh("loose");
    let package = own.join("events");
    // The network denied, but writes allowed everywhere: short of Question 6's minimum.
    let Some(check) = una_sandboxed(
        &own,
        "(version 1)(allow default)(deny network*)",
        &["--json", "check", package.to_str().unwrap()],
    ) else {
        return;
    };
    let text = String::from_utf8_lossy(&check.stdout).into_owned();
    assert_ne!(check.status.code(), Some(0), "{text}");
    let found = diagnostics(&check);
    assert!(!found.is_empty(), "{text}");
    assert!(
        found.iter().all(|d| d["code"] == "U5003"),
        "only U5003, no proof and no other outcome: {text}"
    );
    assert!(
        text.contains(
            "the outer sandbox is too permissive for the solver: file-write-data is allowed"
        ),
        "{text}"
    );
}
