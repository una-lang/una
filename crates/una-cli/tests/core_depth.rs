//! The depth of Core IR (#2264): elaboration makes each statement of a block and each record,
//! variant or list an expression builds one `let` of a chain, so a long body is a deep Core
//! expression, which every later pass walks by recursion. A body at the bound passes every
//! command on the real main thread of `una`; one past it is U0101 at check time, never a stack
//! overflow, wherever it is: a function body, an `expect` line, or a block of statements.
//! Each command runs as a child process, so the stack is the one `una` really has.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The workspace root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A package `name` of edition 0.5 whose module `m` is `program`.
fn package(name: &str, program: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("core-depth-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita {name}\n  version 0.1.0\n  edition 0.5\n\nrun\n  entry m\n  grants\n    console\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/types/module_constant_too_large/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/m.una"), program).unwrap();
    // The canonical form wraps the long lines; `una check` refuses any other (U7001).
    let formatted = una(&dir, &["fmt"]);
    assert_eq!(formatted.status.code(), Some(0), "{formatted:?}");
    dir
}

fn una(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .args(args)
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("NO_COLOR", "1")
        .current_dir(dir)
        .output()
        .unwrap()
}

/// A list literal of `n` lists, each one `let` of Core IR.
fn lists(n: usize) -> String {
    format!("[{}]", vec!["[0, 1]"; n].join(", "))
}

/// A program whose function `big` builds `n` lists, with a test of it.
fn in_body(n: usize) -> String {
    format!(
        "module m\n\npub big() -> Int = {}.len\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"{{big()}}\")\n\ntest \"t\"\n  do big()\n  expect result == {n}\n",
        lists(n)
    )
}

/// A program whose test builds `n` lists in an `expect` line.
fn in_expect(n: usize) -> String {
    format!(
        "module m\n\npub id(x: Int) -> Int = x\n\ntest \"t\"\n  do id(1)\n  expect {}.len == {n}\n",
        lists(n)
    )
}

/// A program whose function `count` is a block of `n` statements.
fn in_block(n: usize) -> String {
    let mut text = String::from("module m\n\npub count() -> Int\n=\n  const x0 = 0\n");
    for i in 1..n {
        text.push_str(&format!("  const x{i} = x{} + 1\n", i - 1));
    }
    text.push_str(&format!("  x{}\n", n - 1));
    text
}

fn overflowed(output: &Output) -> bool {
    String::from_utf8_lossy(&output.stderr).contains("overflowed its stack")
}

#[test]
fn a_body_just_under_the_bound_passes_every_command() {
    let dir = package("under", &in_body(8_000));
    for args in [
        &["check"][..],
        &["test"],
        &["run"],
        &["core"],
        &["core", "--json"],
        &["report"],
        &["build"],
    ] {
        let output = una(&dir, args);
        assert!(!overflowed(&output), "una {args:?}: {output:?}");
        assert_eq!(output.status.code(), Some(0), "una {args:?}: {output:?}");
    }
    let run = una(&dir, &["run"]);
    assert_eq!(String::from_utf8_lossy(&run.stdout), "8000\n");
    let native = Command::new(dir.join("target/una/under")).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&native.stdout),
        "8000\n",
        "{native:?}"
    );
}

#[test]
fn a_body_past_the_bound_is_u0101_in_every_command_never_a_stack_overflow() {
    for (name, program) in [
        ("body", in_body(13_000)),
        ("expect", in_expect(13_000)),
        ("block", in_block(9_000)),
    ] {
        let dir = package(name, &program);
        for args in [
            &["check"][..],
            &["test"],
            &["core"],
            &["core", "--json"],
            &["report"],
            &["build"],
        ] {
            let output = una(&dir, args);
            assert!(!overflowed(&output), "{name}: una {args:?}: {output:?}");
            assert_ne!(output.status.code(), Some(0), "{name}: una {args:?}");
            let all = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                all.contains("U0101") && all.contains("nests deeper than 8192 levels"),
                "{name}: una {args:?}: {all}"
            );
        }
    }
}
