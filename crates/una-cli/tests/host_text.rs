//! Host text that is not valid UTF-8 (S-16, ADR 0086): the content of `Files.read` and the
//! value of `Env.get` abort the run with U6005 at the operation, in both engines, instead of a
//! failure or a `None` that a public branch could see. Valid text reads as before.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Builds the runtime library next to the `una` binary, once.
fn runtime() {
    static BUILT: OnceLock<()> = OnceLock::new();
    BUILT.get_or_init(|| {
        let status = Command::new(env!("CARGO"))
            .args(["build", "--quiet", "-p", "una-rt"])
            .current_dir(root())
            .status()
            .unwrap();
        assert!(status.success(), "cargo build -p una-rt failed");
    });
}

/// A package of edition 0.2 named `name`, with `files` and the bytes of `data`.
fn package(name: &str, grants: &str, source: &str, data: &[(&str, &[u8])]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("una-host-text-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("data")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"0.2\"\n\n[run]\nentry = \"main\"\ngrants = [{grants}]\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("examples/hello/verification.lock"),
        dir.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/main.una"), source).unwrap();
    for (path, bytes) in data {
        std::fs::write(dir.join("data").join(path), bytes).unwrap();
    }
    dir
}

/// Runs `una` with `args` in `dir`, with `UNA_TEST_TEXT` set to `value` when there is one.
fn una(dir: &Path, args: &[&str], value: Option<&[u8]>) -> Output {
    runtime();
    let mut command = Command::new(env!("CARGO_BIN_EXE_una"));
    command
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("NO_COLOR", "1")
        .env_remove("UNA_TEST_TEXT")
        .args(args)
        .current_dir(dir);
    if let Some(value) = value {
        command.env("UNA_TEST_TEXT", OsStr::from_bytes(value));
    }
    command.output().unwrap()
}

/// Runs the executable `una build` made for `name` in `dir`, with its trace in `trace.json`.
fn native(dir: &Path, name: &str, value: Option<&[u8]>) -> Output {
    let mut command = Command::new(dir.join("target/una").join(name));
    command
        .arg("--trace")
        .arg(dir.join("trace.json"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("NO_COLOR", "1")
        .env_remove("UNA_TEST_TEXT")
        .current_dir(dir);
    if let Some(value) = value {
        command.env("UNA_TEST_TEXT", OsStr::from_bytes(value));
    }
    command.output().unwrap()
}

/// Checks `dir`, runs it in both engines and returns the JSON document of `una run`, the human
/// stderr of `una run`, and the native run with its trace.
fn both(
    dir: &Path,
    name: &str,
    value: Option<&[u8]>,
) -> (serde_json::Value, String, Output, serde_json::Value) {
    let checked = una(dir, &["check", "."], None);
    assert_eq!(checked.status.code(), Some(0), "{checked:?}");
    let json = una(dir, &["--json", "run", "."], value);
    let document: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    let human = una(dir, &["run", "."], value);
    let built = una(dir, &["build", "."], None);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let run = native(dir, name, value);
    let trace: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("trace.json")).unwrap()).unwrap();
    (
        document,
        String::from_utf8_lossy(&human.stderr).into_owned(),
        run,
        trace,
    )
}

const READ: &str = r#"module main

show(name: Text) -> Unit
  uses @console: Console[print]
  uses @files: Files[read]
=
  let r = attempt @files.read(name)
  when r
    is Ok(t) -> @console.print("{name}: {t}")
    is Err(e) -> @console.print("{name}: failed")

pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[read]
=
  show("plain.txt")
  show("missing.txt")
  show("legacy.txt")
  @console.print("after")
"#;

#[test]
fn a_file_that_is_not_utf_8_aborts_the_run_in_both_engines() {
    // S-16: whether a file is valid UTF-8 is its content, which is `internal`, so it is no
    // failure a public branch can see: the run aborts with U6005 at the read, after the output
    // so far, with the same trace in both engines. A valid file, and a missing one, read as
    // before.
    let dir = package(
        "read",
        "\"console\", \"files:data:read\"",
        READ,
        &[("plain.txt", b"hello"), ("legacy.txt", b"caf\xe9")],
    );
    let (document, stderr, run, trace) = both(&dir, "read", None);
    assert_eq!(document["status"], "aborted", "{document}");
    assert_eq!(
        document["output"],
        serde_json::json!(["plain.txt: hello", "missing.txt: failed"]),
        "{document}"
    );
    assert_eq!(document["diagnostics"][0]["code"], "U6005", "{document}");
    assert!(stderr.contains("[U6005]"), "{stderr}");
    assert!(
        stderr.contains("the content of this file is not valid UTF-8"),
        "{stderr}"
    );
    assert_eq!(run.status.code(), Some(1), "{run:?}");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "plain.txt: hello\nmissing.txt: failed\n"
    );
    let native_stderr = String::from_utf8_lossy(&run.stderr);
    assert!(native_stderr.contains("[U6005]"), "{native_stderr}");
    assert!(
        native_stderr.contains("the content of this file is not valid UTF-8"),
        "{native_stderr}"
    );
    assert_eq!(document["trace"], trace);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn valid_files_read_as_before_in_both_engines() {
    // The regression side of S-16: valid UTF-8, multi-byte text included, is read unchanged.
    let source = READ.replace("  show(\"legacy.txt\")\n", "  show(\"accented.txt\")\n");
    let dir = package(
        "valid",
        "\"console\", \"files:data:read\"",
        &source,
        &[("plain.txt", b"hello"), ("accented.txt", "café ✓".as_bytes())],
    );
    let (document, _, run, trace) = both(&dir, "valid", None);
    assert_eq!(document["status"], "ok", "{document}");
    let lines = ["plain.txt: hello", "missing.txt: failed", "accented.txt: café ✓", "after"];
    assert_eq!(document["output"], serde_json::json!(lines), "{document}");
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        format!("{}\n", lines.join("\n"))
    );
    assert_eq!(document["trace"], trace);
    let _ = std::fs::remove_dir_all(&dir);
}

const ENV: &str = r#"module main

pub main() -> Unit
  uses @console: Console[print]
  uses @env: Env[get]
=
  @console.print("before")
  let v = @env.get("UNA_TEST_TEXT")
  when v
    is Some(t) -> @console.print("set")
    is None -> @console.print("unset")
"#;

#[test]
fn an_environment_variable_that_is_not_utf_8_aborts_the_run_in_both_engines() {
    // S-16: the tag of `Env.get`'s `Option` is `public`, so a value that is not valid UTF-8
    // cannot turn it into `None`: the run aborts with U6005 at the operation. A valid value,
    // and an unset variable, read as before.
    let dir = package("env", "\"console\", \"env\"", ENV, &[]);
    let (document, stderr, run, trace) = both(&dir, "env", Some(b"caf\xe9"));
    assert_eq!(document["status"], "aborted", "{document}");
    assert_eq!(document["output"], serde_json::json!(["before"]), "{document}");
    assert_eq!(document["diagnostics"][0]["code"], "U6005", "{document}");
    assert!(
        stderr.contains("the value of this environment variable is not valid UTF-8"),
        "{stderr}"
    );
    assert_eq!(run.status.code(), Some(1), "{run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stdout), "before\n");
    let native_stderr = String::from_utf8_lossy(&run.stderr);
    assert!(native_stderr.contains("[U6005]"), "{native_stderr}");
    assert_eq!(document["trace"], trace);

    for (value, line) in [(Some("café".as_bytes()), "set"), (None, "unset")] {
        let (document, _, run, trace) = both(&dir, "env", value);
        assert_eq!(document["status"], "ok", "{document}");
        assert_eq!(
            document["output"],
            serde_json::json!(["before", line]),
            "{document}"
        );
        assert_eq!(String::from_utf8_lossy(&run.stdout), format!("before\n{line}\n"));
        assert_eq!(document["trace"], trace);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_read_that_is_not_utf_8_in_a_task_ends_the_run_in_both_engines() {
    // Under `together` (`spec/19`), the task's abort ends the run with U6005, as U6004 does.
    let source = "module main\n\npub main() -> Unit\n  uses @console: Console[print]\n  uses @files: Files[read]\n=\n  @console.print(\"before\")\n  together\n    let a = attempt @files.read(\"legacy.txt\")\n    let b = attempt @files.read(\"plain.txt\")\n  @console.print(\"after\")\n";
    let dir = package(
        "tasked",
        "\"console\", \"files:data:read\"",
        source,
        &[("plain.txt", b"hello"), ("legacy.txt", b"\xff\xfe")],
    );
    let interpreted = una(&dir, &["run", "."], None);
    assert_eq!(interpreted.status.code(), Some(1), "{interpreted:?}");
    assert_eq!(String::from_utf8_lossy(&interpreted.stdout), "before\n");
    assert!(
        String::from_utf8_lossy(&interpreted.stderr).contains("[U6005]"),
        "{interpreted:?}"
    );
    let built = una(&dir, &["build", "."], None);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let run = native(&dir, "tasked", None);
    assert_eq!(run.status.code(), Some(1), "{run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stdout), "before\n");
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("[U6005]"),
        "{run:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
