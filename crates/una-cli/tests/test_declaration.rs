//! The test declaration of edition 0.3, written `test` (ADR 0105): the output of `una test`
//! names it by the package's edition and prints `una.test/2`; an `example` declaration there is
//! U0103 with the exact rewrite that `una fix` applies; `test` as a name is U0101; the messages
//! that name the declaration follow the edition; and the canonical form is a fixed point of
//! `una fmt`. Editions 0.1 and 0.2 do not change.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn una_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).unwrap()
}

fn json(output: &Output) -> Value {
    serde_json::from_str(stdout(output)).unwrap()
}

/// A package of `edition` under the test's temporary directory, whose only module is `m`,
/// holding `source`, with the repository's `verification.lock`.
fn package(name: &str, edition: &str, source: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("test_declaration")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"{edition}\"\n"),
    )
    .unwrap();
    std::fs::copy(
        repository().join("verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(root.join("src/m.una"), source).unwrap();
    root
}

/// Two tests of `double`, under the keyword `word`.
fn doubling(word: &str) -> String {
    format!(
        "module m\n\npub kind Refusal = | TooLarge\n\npub double(x: Int) -> Int\n  fails Refusal\n=\n  if x > 100 then fail TooLarge else x * 2\n\n{word} \"doubling works\"\n  do double(2)\n  expect result == 4\n\n{word} \"too large fails\"\n  do double(101)\n  expect fails TooLarge\n"
    )
}

#[test]
fn una_test_names_the_declaration_by_the_edition() {
    // ADR 0105 rule 6: `test` and `tests` with `una.test/2` from edition 0.3; `example` and
    // `examples` with `una.test/1` before, byte for byte as today.
    let root = package("output-0-3", "0.3", &doubling("test"));
    let output = una_in(&root, &["test", "."]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        stdout(&output),
        "test m \"doubling works\" ... ok\ntest m \"too large fails\" ... ok\n\n2 tests: 2 passed, 0 failed\n"
    );
    let document = json(&una_in(&root, &["--json", "test", "."]));
    assert_eq!(document["schema"], "una.test/2");
    assert_eq!(document["tests"].as_array().unwrap().len(), 2);
    assert!(document.get("examples").is_none());
    assert_eq!(document["passed"], 2);
    let keys: Vec<&String> = document.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        ["schema", "tier", "redacted", "diagnostics", "tests", "passed", "failed"]
    );

    let root = package("output-0-2", "0.2", &doubling("example"));
    let output = una_in(&root, &["test", "."]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        stdout(&output),
        "example m \"doubling works\" ... ok\nexample m \"too large fails\" ... ok\n\n2 examples: 2 passed, 0 failed\n"
    );
    let document = json(&una_in(&root, &["--json", "test", "."]));
    assert_eq!(document["schema"], "una.test/1");
    assert_eq!(document["examples"].as_array().unwrap().len(), 2);
    assert!(document.get("tests").is_none());

    // Edition 0.4 has the keyword of edition 0.3 (`spec/13 §7`).
    let root = package("output-0-4", "0.3", &doubling("test"));
    std::fs::remove_file(root.join("una.toml")).unwrap();
    std::fs::write(
        root.join("ita.una"),
        "ita output_0_4\n  version 0.1.0\n  edition 0.4\n",
    )
    .unwrap();
    std::fs::rename(root.join("verification.lock"), root.join("ita.lock")).unwrap();
    let output = una_in(&root, &["test", "."]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(stdout(&output).ends_with("2 tests: 2 passed, 0 failed\n"));
}

#[test]
fn an_example_in_edition_0_3_is_rewritten_to_test_by_una_fix() {
    // ADR 0105 rule 3: U0103 at the keyword, the exact rewrite is its first repair and a
    // candidate of `una fix`, one run migrates the package, and the result checks and runs.
    let root = package("example-0-3", "0.3", &doubling("example"));
    let document = json(&una_in(&root, &["--json", "check", "."]));
    let diagnostics = document["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    for (diagnostic, line) in diagnostics.iter().zip([10, 14]) {
        assert_eq!(diagnostic["code"], "U0103");
        assert_eq!(diagnostic["primary"]["start"]["line"], line);
        assert_eq!(diagnostic["primary"]["start"]["col"], 1);
        assert_eq!(
            diagnostic["message"],
            "`example` is written `test` in edition 0.3"
        );
        let repair = &diagnostic["repairs"][0];
        assert_eq!(repair["description"], "write `test`");
        assert_eq!(repair["edit"]["replacement"], "test");
        assert_eq!(repair["escape_cost"], 0);
    }
    assert_eq!(document["fixable"], 2);

    let fix = una_in(&root, &["fix", "."]);
    assert_eq!(fix.status.code(), Some(0), "{fix:?}");
    assert_eq!(
        std::fs::read_to_string(root.join("src/m.una")).unwrap(),
        doubling("test")
    );
    let check = una_in(&root, &["check", "."]);
    assert_eq!(check.status.code(), Some(0), "{check:?}");
    let output = una_in(&root, &["test", "."]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(stdout(&output).ends_with("2 tests: 2 passed, 0 failed\n"));
    // The rewritten file is canonical: `una fmt` changes nothing.
    let fmt = una_in(&root, &["fmt", "--check", "."]);
    assert_eq!(fmt.status.code(), Some(0), "{fmt:?}");
}

#[test]
fn a_name_test_in_edition_0_3_is_renamed_or_advised() {
    // ADR 0105 rule 4 with ADR 0096: a local gets the exact scoped rename to `test_value`; a
    // function gets the rename as advice, since its callers name it.
    let root = package(
        "name-0-3",
        "0.3",
        "module m\n\npub add(x: Int) -> Int\n=\n  let test = x + 1\n  test\n",
    );
    let document = json(&una_in(&root, &["--json", "check", "."]));
    let diagnostic = &document["diagnostics"][0];
    assert_eq!(diagnostic["code"], "U0101");
    assert_eq!(diagnostic["primary"]["start"]["line"], 5);
    assert_eq!(diagnostic["primary"]["start"]["col"], 7);
    let repair = &diagnostic["repairs"][0];
    assert_eq!(
        repair["description"],
        "rename the binding and its uses to `test_value`"
    );
    assert_eq!(
        repair["edit"]["replacement"],
        "test_value = x + 1\n  test_value"
    );
    assert_eq!(document["fixable"], 1);
    let fix = una_in(&root, &["fix", "."]);
    assert_eq!(fix.status.code(), Some(0), "{fix:?}");
    assert_eq!(
        std::fs::read_to_string(root.join("src/m.una")).unwrap(),
        "module m\n\npub add(x: Int) -> Int\n=\n  let test_value = x + 1\n  test_value\n"
    );

    let root = package(
        "function-0-3",
        "0.3",
        "module m\n\ntest(x: Int) -> Int = x\n\npub g() -> Int = test(1)\n",
    );
    let document = json(&una_in(&root, &["--json", "check", "."]));
    let diagnostic = &document["diagnostics"][0];
    assert_eq!(diagnostic["code"], "U0101");
    assert_eq!(diagnostic["primary"]["start"]["line"], 3);
    assert_eq!(
        diagnostic["message"],
        "`test` is a reserved keyword, so it cannot be a function name"
    );
    let repair = &diagnostic["repairs"][0];
    assert_eq!(
        repair["description"],
        "choose another name, such as `test_fn`"
    );
    assert!(repair["edit"].is_null());

    // Editions 0.1 and 0.2: `test` is a name.
    let root = package(
        "name-0-2",
        "0.2",
        "module m\n\ntest(x: Int) -> Int = x\n\npub g() -> Int = test(1)\n",
    );
    let check = una_in(&root, &["check", "."]);
    assert_eq!(check.status.code(), Some(0), "{check:?}");
}

#[test]
fn messages_name_the_declaration_by_the_edition() {
    // ADR 0105 rule 6: U3014 and U1002 say "test" in edition 0.3 and "example" before.
    let host = |word: &str| {
        format!(
            "module m\n\npub greet(name: Text) -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"hello {{name}}\")\n\n{word} \"a greeting\"\n  do greet(\"Ana\")\n  expect true\n\n{word} \"a greeting\"\n  do greet(\"Bo\")\n  expect true\n"
        )
    };
    let messages = |name: &str, edition: &str, word: &str| -> Vec<String> {
        let root = package(name, edition, &host(word));
        let document = json(&una_in(&root, &["--json", "check", "."]));
        document["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| format!("{} {}", d["code"].as_str().unwrap(), d["message"].as_str().unwrap()))
            .collect()
    };
    let in_0_3 = messages("messages-0-3", "0.3", "test");
    assert!(
        in_0_3.contains(&"U1002 test \"a greeting\" is defined twice in this module".to_owned()),
        "{in_0_3:#?}"
    );
    assert!(
        in_0_3.iter().any(|m| m.starts_with("U3014") && m.contains("tests cannot use host capabilities")),
        "{in_0_3:#?}"
    );
    let in_0_2 = messages("messages-0-2", "0.2", "example");
    assert!(
        in_0_2.contains(&"U1002 example \"a greeting\" is defined twice in this module".to_owned()),
        "{in_0_2:#?}"
    );
    assert!(
        in_0_2.iter().any(|m| m.starts_with("U3014") && m.contains("examples cannot use host capabilities")),
        "{in_0_2:#?}"
    );
}
