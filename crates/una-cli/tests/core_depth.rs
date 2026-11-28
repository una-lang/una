//! The depth of Core IR (#2264): elaboration makes each statement of a block and each record,
//! variant or list an expression builds one `let` of a chain, so a long body is a deep Core
//! expression, which every later pass walks by recursion. A body at the bound passes every
//! command in a real `una` process; one past it is U0101 at check time, never a stack
//! overflow, wherever it is: a function body, an `expect` line, or a block of statements.
//! The size of what substitution of module constants builds is bounded too (U2041), with
//! every reference to a text constant counted, since elaboration copies a text at each one.
//! Each command runs as a child process, so the stack is the one `una` really has, `una lsp`
//! included, whose tier 1 check runs on a thread of its own.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::channel;
use std::time::Duration;

use serde_json::{Value, json};

#[path = "common/native_build.rs"]
mod native_build;

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

/// One item of a list literal that builds seven lists, each one `let` of Core IR.
const ITEM: &str = "[[[0], [0]], [[0], [0]]]";

/// A list literal of `n` items, `7 * n` levels of Core IR.
fn lists(n: usize) -> String {
    format!("[{}]", vec![ITEM; n].join(", "))
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

/// A program whose function `count` is a block of `n` statements, each building seven lists.
fn in_block(n: usize) -> String {
    let mut text = String::from("module m\n\npub count() -> Int\n=\n");
    for i in 0..n {
        text.push_str(&format!("  const x{i} = {ITEM}\n"));
    }
    text.push_str(&format!("  x{}.len\n", n - 1));
    text
}

fn overflowed(output: &Output) -> bool {
    String::from_utf8_lossy(&output.stderr).contains("overflowed its stack")
}

#[test]
fn a_body_just_under_the_bound_passes_every_command() {
    // `una build` links the runtime library, which `cargo test` builds only when asked (#2339:
    // the Linux legs run `cargo test --workspace`, where no step built it beside `una`).
    native_build::runtime();
    // 18,000 items: 126,000 levels, under the bound of 131,072.
    let dir = package("under", &in_body(18_000));
    for args in [
        &["check"][..],
        &["test"],
        // The evaluator's stack follows `--max-depth`; the copy of the program does not
        // (#2264 review 2).
        &["test", "--max-depth", "1"],
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
    assert_eq!(String::from_utf8_lossy(&run.stdout), "18000\n");
    let native = Command::new(dir.join("target/una/under")).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&native.stdout),
        "18000\n",
        "{native:?}"
    );
}

/// `una lsp` on a package of edition 0.1 with a `una.toml`, the manifest the server finds the
/// root of a package by: opening and saving `src/m.una`, whose text is `program` in canonical
/// form, publishes no diagnostic at tier 1, and the server shuts down cleanly. The tier 1
/// check runs on a thread of the server, which used to have the default 2 MiB stack and
/// aborted on a program that `una check` accepts (#2264, review 1 M2).
fn lsp_checks_cleanly(name: &str, program: &str) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("core-depth-lsp-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n"),
    )
    .unwrap();
    std::fs::copy(
        root().join("verification.lock"),
        dir.join("verification.lock"),
    )
    .unwrap();
    let file = dir.join("src/m.una");
    std::fs::write(&file, program).unwrap();
    let formatted = una(&dir, &["fmt"]);
    assert_eq!(formatted.status.code(), Some(0), "{formatted:?}");
    let checked = una(&dir, &["check"]);
    assert_eq!(checked.status.code(), Some(0), "{checked:?}");
    let text = std::fs::read_to_string(&file).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_una"))
        .arg("lsp")
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .env_remove("UNA_SOLVER_STAND_IN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, messages) = channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        while let Ok(Some(message)) = una_lsp::read_message(&mut reader) {
            if sender.send(message).is_err() {
                return;
            }
        }
    });
    // A server that died takes no more input: the asserts below on its exit report it.
    let mut send = |message: Value| {
        let _ = una_lsp::write_message(&mut stdin, &message);
        let _ = stdin.flush();
    };
    let uri = una_lsp::uri_of(&file);
    // The next message for which `wanted` holds; `None` when the server ends or is silent.
    let next = |wanted: &dyn Fn(&Value) -> bool| loop {
        match messages.recv_timeout(Duration::from_secs(600)) {
            Ok(message) if wanted(&message) => return Some(message),
            Ok(_) => {}
            Err(_) => return None,
        }
    };
    let published = |message: &Value| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == uri.as_str()
    };
    send(
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } }),
    );
    assert!(
        next(&|message| message["id"] == 1).is_some(),
        "no initialize response"
    );
    send(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
    send(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": uri, "languageId": "una", "version": 1, "text": text } },
    }));
    let opened = next(&published);
    send(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didSave",
        "params": { "textDocument": { "uri": uri } },
    }));
    let saved = next(&published);
    send(json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": null }));
    let shut = next(&|message| message["id"] == 2);
    send(json!({ "jsonrpc": "2.0", "method": "exit", "params": null }));
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(!overflowed(&output), "una lsp: {output:?}");
    assert!(output.status.success(), "una lsp: {output:?}");
    for (what, message) in [("didOpen", opened), ("didSave", saved)] {
        assert!(message.is_some(), "una lsp published nothing after {what}");
        let message = message.unwrap();
        assert_eq!(
            message["params"]["diagnostics"],
            json!([]),
            "una lsp after {what}"
        );
    }
    assert!(shut.is_some(), "una lsp did not answer shutdown");
}

#[test]
fn a_body_just_under_the_bound_passes_una_lsp_at_tier_1() {
    // 18,000 items: 126,000 levels, under the bound of 131,072, as `una check` accepts.
    let program = format!("module m\n\npub big() -> Int = {}.len\n", lists(18_000));
    lsp_checks_cleanly("under", &program);
}

#[test]
fn a_body_past_the_bound_is_u0101_in_every_command_never_a_stack_overflow() {
    // `una build` among the commands: its runtime library, as above.
    native_build::runtime();
    for (name, program) in [
        // 19,000 items: 133,000 levels, past the bound.
        ("body", in_body(19_000)),
        ("expect", in_expect(19_000)),
        ("block", in_block(19_000)),
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
                all.contains("U0101") && all.contains("nests deeper than 131072 levels"),
                "{name}: una {args:?}: {all}"
            );
        }
    }
}

/// A text constant of 900,000 characters, named 12 times in one line: in an `expect` line, in
/// the `do` line, and in a function body. Elaboration copies a scalar value at every
/// reference, even in a part that shares records and lists, so the 12 copies, 10,800,000
/// characters, pass the cap of 10,000,000 on what substitution builds: U2041, not gigabytes
/// of Core IR (#2264 review 2 M1).
#[test]
fn a_text_constant_counts_at_every_reference_in_one_line() {
    let text = "a".repeat(900_000);
    let list = ["big"; 12].join(", ");
    let header =
        format!("module m\n\npub const big: Text = \"{text}\"\n\npub id(x: Int) -> Int = x\n");
    for (name, rest) in [
        (
            "text-expect",
            format!("\ntest \"t\"\n  do id(1)\n  expect [{list}].len == 12\n"),
        ),
        (
            "text-do",
            format!("\ntest \"t\"\n  do id([{list}].len)\n  expect result == 12\n"),
        ),
        (
            "text-body",
            format!("\npub twelve() -> Int = [{list}].len\n"),
        ),
    ] {
        let dir = package(name, &format!("{header}{rest}"));
        let output = una(&dir, &["check", "--tier", "0"]);
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_ne!(output.status.code(), Some(0), "{name}: {all}");
        assert!(
            all.contains("U2041") && all.contains("more than 10000000"),
            "{name}: {all}"
        );
    }
}
