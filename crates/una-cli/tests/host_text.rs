//! Host text that is not valid UTF-8 (S-16, ADR 0086): the content of `Files.read` and the
//! value of `Env.get` abort the run with U6005 at the operation, instead of a failure or a
//! `None` that a public branch could see. Valid text reads as before.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// The policy the tests print a read's `internal` text through, so that they can compare it.
const RELEASE: &str = r#"trusted module release

pub policy shown
  from internal Text
  to public Text
  via as_text
  purpose "show what a read returned, to compare it in a test"
  owner tests

as_text(t: internal Text) -> public Text = t
"#;

/// A package of edition 0.1 named `name`, with `src/main.una`, the trusted module `release`,
/// and the bytes of `data`.
fn package(name: &str, grants: &str, source: &str, data: &[(&str, &[u8])]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("una-host-text-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("data")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n\n[trusted]\nmodules = [\"release\"]\n\n[run]\nentry = \"main\"\ngrants = [{grants}]\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("examples/hello/verification.lock"),
        dir.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/main.una"), source).unwrap();
    std::fs::write(dir.join("src/release.una"), RELEASE).unwrap();
    for (path, bytes) in data {
        std::fs::write(dir.join("data").join(path), bytes).unwrap();
    }
    dir
}

/// Runs `una` with `args` in `dir`, with `UNA_TEST_TEXT` set to `value` when there is one.
fn una(dir: &Path, args: &[&str], value: Option<&[u8]>) -> Output {
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

/// Checks `dir` and runs it: the JSON document of `una run`, and the human stderr.
fn run(dir: &Path, value: Option<&[u8]>) -> (serde_json::Value, String) {
    let checked = una(dir, &["check", "."], None);
    assert_eq!(checked.status.code(), Some(0), "{checked:?}");
    let json = una(dir, &["--json", "run", "."], value);
    let document: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    let human = una(dir, &["run", "."], value);
    (
        document,
        String::from_utf8_lossy(&human.stderr).into_owned(),
    )
}

const READ: &str = r#"module main

import release (shown)

show(name: Text) -> Unit
  uses @console: Console[print]
  uses @files: Files[read]
=
  let r = attempt @files.read(name)
  when r
    is Ok(t) -> @console.print("{name}: {shown(t)}")
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
fn a_file_that_is_not_utf_8_aborts_the_run() {
    // S-16: whether a file is valid UTF-8 is its content, which is `internal`, so it is no
    // failure a public branch can see: the run aborts with U6005 at the read, after the output
    // so far. A valid file, and a missing one, read as before.
    let dir = package(
        "read",
        "\"console\", \"files:data:read\"",
        READ,
        &[("plain.txt", b"hello"), ("legacy.txt", b"caf\xe9")],
    );
    let (document, stderr) = run(&dir, None);
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
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn valid_files_read_as_before() {
    // The regression side of S-16: valid UTF-8, multi-byte text included, is read unchanged.
    let source = READ.replace("  show(\"legacy.txt\")\n", "  show(\"accented.txt\")\n");
    let dir = package(
        "valid",
        "\"console\", \"files:data:read\"",
        &source,
        &[
            ("plain.txt", b"hello"),
            ("accented.txt", "café ✓".as_bytes()),
        ],
    );
    let (document, _) = run(&dir, None);
    assert_eq!(document["status"], "ok", "{document}");
    assert_eq!(
        document["output"],
        serde_json::json!([
            "plain.txt: hello",
            "missing.txt: failed",
            "accented.txt: café ✓",
            "after"
        ]),
        "{document}"
    );
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
fn an_environment_variable_that_is_not_utf_8_aborts_the_run() {
    // S-16: the tag of `Env.get`'s `Option` is `public`, so a value that is not valid UTF-8
    // cannot turn it into `None`: the run aborts with U6005 at the operation. A valid value,
    // and an unset variable, read as before.
    let dir = package("env", "\"console\", \"env\"", ENV, &[]);
    let (document, stderr) = run(&dir, Some(b"caf\xe9"));
    assert_eq!(document["status"], "aborted", "{document}");
    assert_eq!(
        document["output"],
        serde_json::json!(["before"]),
        "{document}"
    );
    assert_eq!(document["diagnostics"][0]["code"], "U6005", "{document}");
    assert!(
        stderr.contains("the value of this environment variable is not valid UTF-8"),
        "{stderr}"
    );
    for (value, line) in [(Some("café".as_bytes()), "set"), (None, "unset")] {
        let (document, _) = run(&dir, value);
        assert_eq!(document["status"], "ok", "{document}");
        assert_eq!(
            document["output"],
            serde_json::json!(["before", line]),
            "{document}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
