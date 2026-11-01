//! `una check` under an outer sandbox on macOS (ADR 0124 rule 7, Question 6, #1360): under a
//! profile that meets Question 6's minimum, as the toolchain-free gate's does, the solver
//! children take the outer sandbox as their confinement and every obligation of the events
//! example is proven; under a profile short of that minimum no child answers, and the run
//! fails with U5003, naming the probe it allows (#1363, #1364). Under a sandbox of its own that
//! refuses a nested one, each test fails unless `UNA_SKIP_NESTED_SANDBOX_TESTS=1` is set
//! (`docs/compiler/testing.md`). Tier 1: it needs the solver built into `una`.
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
/// root, as `scripts/toolchain-free.sh` runs it. `None` when this process is itself under a
/// sandbox that refuses a nested one and `UNA_SKIP_NESTED_SANDBOX_TESTS=1` is set; without it
/// the test fails there (#1364 L1).
fn una_sandboxed(own: &Path, profile: &str, args: &[&str]) -> Option<Output> {
    una_sandboxed_in(own, profile, args, &repository())
}

/// As [`una_sandboxed`], from the working directory `cwd`.
fn una_sandboxed_in(own: &Path, profile: &str, args: &[&str], cwd: &Path) -> Option<Output> {
    let output = Command::new("/usr/bin/sandbox-exec")
        .arg("-p")
        .arg(profile)
        .arg(env!("CARGO_BIN_EXE_una"))
        .args(args)
        .env_clear()
        .env("HOME", own.join("home"))
        .env("TMPDIR", own.join("tmp"))
        .env("PATH", own.join("tmp"))
        .current_dir(cwd)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.starts_with("sandbox-exec: sandbox_apply") {
        #[allow(clippy::disallowed_methods)]
        let skip = std::env::var_os("UNA_SKIP_NESTED_SANDBOX_TESTS").is_some_and(|v| v == "1");
        assert!(
            skip,
            "this test runs under a sandbox that refuses a nested one, and \
             UNA_SKIP_NESTED_SANDBOX_TESTS=1 is not set: {stderr}"
        );
        let _ = writeln!(
            std::io::stderr(),
            "skipped (UNA_SKIP_NESTED_SANDBOX_TESTS=1): this test runs under a sandbox that refuses a nested one: {stderr}"
        );
        return None;
    }
    Some(output)
}

/// The toolchain-free gate's profile for `own`: the network and signals to other processes
/// denied, writes only in `own`, and no program started but `una`.
fn compliant(own: &Path) -> String {
    let una = Path::new(env!("CARGO_BIN_EXE_una")).canonicalize().unwrap();
    format!(
        "(version 1)(allow default)(deny network*)(deny signal (target others))(deny file-write*)\
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
    // Every probe the minimum requires is named in the report (#1364).
    let denied: Vec<String> = json["tcb"]["solver_outer_denied"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    let mut required = Vec::new();
    for op in ["network-outbound", "network-inbound", "network-bind"] {
        required.push(op.to_owned());
        for filter in [
            "tcp 127.0.0.1",
            "tcp ::1",
            "udp 127.0.0.1",
            "udp ::1",
            "unix-socket",
        ] {
            required.push(format!("{op} {filter}"));
        }
    }
    for op in [
        "data",
        "create",
        "unlink",
        "xattr",
        "mode",
        "flags",
        "times",
        "owner",
        "acl",
        "finderinfo",
    ] {
        for place in [
            "/",
            "/Users",
            "/private/var",
            "/private/tmp",
            "/Volumes",
            "(home)",
            "(temporary directory)",
        ] {
            required.push(format!("file-write-{op} {place}"));
        }
    }
    for name in &required {
        assert!(denied.contains(name), "{name}: {denied:?}");
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
            "the outer sandbox is too permissive for the solver: file-write-data / is allowed"
        ),
        "{text}"
    );
}

/// `una --json check` of a fresh copy under `profile`, from `cwd`: it must fail with U5003
/// alone, naming `allowed` (a prefix of the probe's name), and prove nothing.
fn refused_naming(label: &str, profile: impl Fn(&Path) -> String, cwd: &Path, allowed: &str) {
    let own = fresh(label);
    let package = own.join("events");
    let Some(check) = una_sandboxed_in(
        &own,
        &profile(&own),
        &["--json", "check", package.to_str().unwrap()],
        cwd,
    ) else {
        return;
    };
    let text = String::from_utf8_lossy(&check.stdout).into_owned();
    assert_ne!(check.status.code(), Some(0), "{text}");
    let found = diagnostics(&check);
    assert!(!found.is_empty(), "{text}");
    assert!(found.iter().all(|d| d["code"] == "U5003"), "{text}");
    let refused = format!("the outer sandbox is too permissive for the solver: {allowed}");
    assert!(text.contains(&refused), "{refused}: {text}");
}

#[test]
fn an_outer_sandbox_that_allows_outbound_network_to_localhost_is_refused() {
    // #1363: before the fix, 11 of 11 proven under this profile, `outer`.
    refused_naming(
        "localhost",
        |own| {
            format!(
                "(version 1)(allow default)(deny network*)\
                 (allow network-outbound (remote ip \"localhost:*\"))(deny file-write*)\
                 (allow file-write* (subpath \"{}\"))(allow file-write* (literal \"/dev/null\"))",
                own.display()
            )
        },
        &repository(),
        "network-outbound tcp 127.0.0.1 is allowed",
    );
}

/// The resolved home directory of this user's account, from the directory service (the passwd
/// source on macOS), as the code takes it from the passwd entry, not `HOME`, which
/// `una_sandboxed` sets to a directory inside the run: a profile that allows writes under this
/// directory is what the solver's probe must refuse. `una-cli` forbids `unsafe`, so the passwd
/// entry is read through `dscl` rather than `getpwuid`.
fn home() -> PathBuf {
    let output = Command::new("/usr/bin/dscl")
        .args([".", "-read"])
        .arg(format!("/Users/{}", whoami()))
        .arg("NFSHomeDirectory")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let dir = text
        .strip_prefix("NFSHomeDirectory:")
        .unwrap()
        .trim()
        .to_owned();
    PathBuf::from(dir).canonicalize().unwrap()
}

/// This user's short name (`id -un`), the key of its directory-service record.
fn whoami() -> String {
    let output = Command::new("/usr/bin/id").arg("-un").output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn an_outer_sandbox_that_allows_writes_under_home_is_refused_from_any_working_directory() {
    // #1364: from `/`, the old path "outside the temporary directory" was `/` again.
    refused_naming(
        "home",
        |_| {
            format!(
                "(version 1)(allow default)(deny network*)(deny file-write*)\
                 (allow file-write* (subpath \"{}\"))(allow file-write* (literal \"/dev/null\"))",
                home().display()
            )
        },
        Path::new("/"),
        "file-write-data (home) is allowed",
    );
}

#[test]
fn an_outer_sandbox_that_denies_writes_at_two_paths_only_is_refused() {
    // #1364: writes denied at `/` and at the working directory, allowed everywhere else.
    refused_naming(
        "two-paths",
        |_| {
            format!(
                "(version 1)(allow default)(deny network*)\
                 (deny file-write* (literal \"/\") (literal \"{}\"))",
                repository().display()
            )
        },
        &repository(),
        "file-write-data /Users is allowed",
    );
}

#[test]
fn an_outer_sandbox_that_leaves_extended_attributes_and_modes_open_is_refused() {
    // #1364: data, create and unlink denied, the other writes open.
    refused_naming(
        "xattr",
        |_| {
            "(version 1)(allow default)(deny network*)(deny file-write-data)\
             (deny file-write-create)(deny file-write-unlink)"
                .to_owned()
        },
        &repository(),
        "file-write-xattr ",
    );
}
