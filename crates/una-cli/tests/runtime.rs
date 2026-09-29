//! End-to-end tests of `una run` and `una test` (`spec/11`, `spec/13 §5`, ADR 0027, ADR 0032).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pretty_assertions::assert_eq;
use serde_json::{Value, json};

/// The note of ADR 0027 rule 2, on stderr in human mode.
const NOTE: &str = "note: proof obligations are not verified yet (tier 1 arrives in M8)\n";

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Runs the `una` binary in `dir`, relative to the repository root, or absolute.
fn una_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .args(args)
        .current_dir(repository().join(dir))
        .output()
        .unwrap()
}

fn una(args: &[&str]) -> Output {
    una_in(Path::new("."), args)
}

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).unwrap()
}

fn stderr(output: &Output) -> &str {
    std::str::from_utf8(&output.stderr).unwrap()
}

fn json(output: &Output) -> Value {
    serde_json::from_str(stdout(output)).unwrap()
}

/// A package under the test's temporary directory, with `una.toml` (the package table and
/// `extra`), the repository's `verification.lock` and the files `files` relative to it.
fn package(name: &str, extra: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("runtime")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("una.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n{extra}"),
    )
    .unwrap();
    std::fs::copy(
        repository().join("verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    for (path, text) in files {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    root
}

#[test]
fn test_runs_the_examples_of_the_reference_example() {
    // M7 acceptance criterion: `una test examples/events` passes.
    let output = una(&["test", "examples/events"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stderr(&output), NOTE);
    assert_eq!(
        stdout(&output),
        "example events.checkin \"a registered attendee is checked in and audited\" ... ok\n\
         example events.checkin \"an unknown attendee fails and is not audited\" ... ok\n\
         example events.seating \"an event without seats is sold out\" ... ok\n\
         example events.seating \"an event with many seats is open\" ... ok\n\
         example events.seating \"raw seat counts are clamped\" ... ok\n\
         example events.types \"malformed input is rejected\" ... ok\n\
         \n\
         6 examples: 6 passed, 0 failed\n"
    );
}

#[test]
fn run_prints_the_greeting() {
    // M7 acceptance criterion: `una run examples/hello --grant console` prints the greeting.
    let output = una(&["run", "examples/hello", "--grant", "console"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "Hello, Una\n");
    assert_eq!(stderr(&output), NOTE);
    let output = una(&["run", "examples/events"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "availability of Launch: open\n");
}

#[test]
fn json_documents_carry_the_tier_and_no_note() {
    // ADR 0027 rule 3; ADR 0032 rules 4 and 5.
    let output = una(&["--json", "run", "examples/hello"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stderr(&output), "");
    insta::assert_snapshot!("run_hello_json", stdout(&output));
    assert_eq!(
        json(&output),
        json!({
            "schema": "una.run/1",
            "tier": 0,
            "status": "ok",
            "output": ["Hello, Una"],
            "failure": null,
            "diagnostics": [],
            "trace": [{
                "seq": 1,
                "instance": "host:Console",
                "capability": "Console",
                "op": "print",
                "args": {"line": "\"Hello, Una\""},
                "outcome": {"kind": "ok", "value": "unit"}
            }]
        })
    );
    let output = una(&["--json", "test", "examples/events"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stderr(&output), "");
    let again = una(&["--json", "test", "examples/events"]);
    assert_eq!(output.stdout, again.stdout);
    insta::assert_snapshot!("test_events_json", stdout(&output));
    let document = json(&output);
    assert_eq!(document["schema"], "una.test/1");
    assert_eq!(document["tier"], 0);
    assert_eq!(document["passed"], 6);
    assert_eq!(document["failed"], 0);
}

/// A capability whose arguments carry labeled data, and a record with a labeled field.
const VAULT: &str = "module vault

pub kind Badge
  holder: Text
  code: secret Text

pub capability Locker
  op store(badge: Badge, passcode: secret Int) -> Unit
    accepts secret
  op note(line: Text) -> Unit

pub handler memory() for Locker
  op store(badge, passcode) = unit
  op note(line) = unit

pub keep(badge: Badge, passcode: secret Int) -> Unit
  uses @locker: Locker[store, note]
=
  @locker.store(badge, passcode)
  @locker.note(\"stored for {badge.holder}\")

pub main() -> Unit
=
  keep(Badge(holder: \"Ana\", code: \"QX-7731\"), passcode: 9876) with @locker = memory()

example \"a badge is kept\"
  given @locker = memory()
  do keep(Badge(holder: \"Ana\", code: \"QX-7731\"), passcode: 9876)
  expect performed @locker.store
";

#[test]
fn traces_never_show_labeled_arguments() {
    // M7 acceptance criterion: labeled arguments never appear in traces (`spec/11 §5`).
    let root = package(
        "vault",
        "\n[run]\nentry = \"vault\"\n",
        &[("src/vault.una", VAULT)],
    );
    for command in ["run", "test"] {
        let output = una_in(&root, &["--json", command]);
        assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
        let text = stdout(&output);
        assert!(!text.contains("QX-7731"), "{command}: {text}");
        assert!(!text.contains("9876"), "{command}: {text}");
        let document = json(&output);
        let trace = if command == "run" {
            document["trace"].clone()
        } else {
            document["examples"][0]["trace"].clone()
        };
        let instance = if command == "run" {
            "with:memory#1"
        } else {
            "given:locker"
        };
        assert_eq!(
            trace,
            json!([
                {
                    "seq": 1,
                    "instance": instance,
                    "capability": "Locker",
                    "op": "store",
                    "args": {
                        "badge": "Badge(holder: \"Ana\", code: <redacted:secret>)",
                        "passcode": "<redacted:secret>"
                    },
                    "outcome": {"kind": "ok", "value": "unit"}
                },
                {
                    "seq": 2,
                    "instance": instance,
                    "capability": "Locker",
                    "op": "note",
                    "args": {"line": "\"stored for Ana\""},
                    "outcome": {"kind": "ok", "value": "unit"}
                }
            ]),
            "{command}"
        );
    }
}

#[test]
fn a_failing_main_is_u6001_and_shows_only_public_fields() {
    // `spec/04 §8`: the variant goes to stderr with exit code 1, and payload fields only when
    // their labels are `public`.
    let root = package(
        "refusal",
        "\n[run]\nentry = \"refusal\"\ngrants = [\"console\"]\n",
        &[(
            "src/refusal.una",
            "module refusal

pub kind Refusal = | Blocked(reason: Text, code: secret Int)

pub main() -> Unit
  uses @console: Console[print]
  fails Refusal
=
  @console.print(\"starting\")
  fail Blocked(reason: \"maintenance\", code: 4242)
",
        )],
    );
    let output = una_in(&root, &["run", "--color", "never"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "starting\n");
    assert!(stderr(&output).starts_with(NOTE));
    assert!(
        stderr(&output)
            .contains("[U6001] Error: `main` failed with `Blocked(reason: \"maintenance\")`"),
        "{}",
        stderr(&output)
    );
    assert!(!stderr(&output).contains("4242"));
    let output = una_in(&root, &["--json", "run"]);
    assert_eq!(output.status.code(), Some(1));
    let document = json(&output);
    assert!(!stdout(&output).contains("4242"));
    assert_eq!(document["status"], "failed");
    assert_eq!(document["output"], json!(["starting"]));
    assert_eq!(
        document["failure"],
        json!({"variant": "Blocked", "fields": {"reason": "\"maintenance\""}})
    );
    let diagnostic = &document["diagnostics"][0];
    assert_eq!(diagnostic["code"], "U6001");
    assert_eq!(
        diagnostic["primary"],
        json!({
            "file": "src/refusal.una",
            "start": {"line": 5, "col": 5},
            "end": {"line": 5, "col": 9}
        })
    );
}

const COUNTDOWN: &str = "module countdown

countdown(n: Int) -> Int = if n == 0 then 0 else countdown(n - 1)

pub main() -> Unit
  uses @console: Console[print]
=
  let steps = countdown(DEPTH)
  @console.print(\"done {steps}\")
";

#[test]
fn a_call_deeper_than_the_limit_is_u6003() {
    // `spec/11 §7`: the default limit is 10,000 calls in progress, and `--max-depth` sets it.
    let file = |depth: &str| {
        let root = package("countdown", "", &[]);
        let file = root.join("countdown.una");
        std::fs::write(&file, COUNTDOWN.replace("DEPTH", depth)).unwrap();
        file
    };
    let deep = file("9999");
    let output = una(&["run", deep.to_str().unwrap(), "--grant", "console"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "done 0\n");
    let output = una(&[
        "--json",
        "run",
        deep.to_str().unwrap(),
        "--grant",
        "console",
        "--max-depth",
        "50",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let document = json(&output);
    assert_eq!(document["status"], "aborted");
    let diagnostic = &document["diagnostics"][0];
    assert_eq!(diagnostic["code"], "U6003");
    assert_eq!(
        diagnostic["message"],
        "this call exceeds the maximum call depth of 50"
    );
    assert_eq!(
        diagnostic["primary"]["start"],
        json!({"line": 3, "col": 50})
    );
    let never = file("-1");
    let output = una(&["run", never.to_str().unwrap(), "--grant", "console"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("maximum call depth of 10000"));
}

#[test]
fn a_depth_beyond_the_address_space_is_a_usage_error() {
    let root = package("too-deep", "", &[]);
    let file = root.join("countdown.una");
    std::fs::write(&file, COUNTDOWN.replace("DEPTH", "3")).unwrap();
    let output = una(&[
        "run",
        file.to_str().unwrap(),
        "--grant",
        "console",
        "--max-depth",
        "100000000000",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("--max-depth 100000000000"),
        "{}",
        stderr(&output)
    );
}

const FILES: &str = "module files

pub describe(failure: FileError) -> Text
=
  when failure
    is NotFound -> \"not found\"
    is Denied -> \"denied\"
    is Io -> \"io\"

pub show(path: Text) -> Text
  uses @files: Files[read]
=
  let content = attempt @files.read(path)
  when content
    is Ok -> \"read\"
    is Err(failure) -> describe(failure)

pub save(path: Text) -> Text
  uses @files: Files[write]
=
  let saved = attempt @files.write(path, content: \"entry\")
  when saved
    is Ok -> \"written\"
    is Err(failure) -> describe(failure)

pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[read, write]
=
  let note = show(\"note.txt\")
  @console.print(\"note.txt: {note}\")
  let around = show(\"sub/../note.txt\")
  @console.print(\"sub/../note.txt: {around}\")
  let outside = show(\"../secret.txt\")
  @console.print(\"../secret.txt: {outside}\")
  let link = show(\"link.txt\")
  @console.print(\"link.txt: {link}\")
  let missing = show(\"missing.txt\")
  @console.print(\"missing.txt: {missing}\")
  let absolute = show(\"/etc/hosts\")
  @console.print(\"/etc/hosts: {absolute}\")
  let log = save(\"log.txt\")
  @console.print(\"log.txt: {log}\")
  let escape = save(\"../escape.txt\")
  @console.print(\"../escape.txt: {escape}\")
";

#[test]
fn files_grants_confine_paths_to_their_prefix() {
    // `spec/14 §3`, DECISION D-103: relative to the prefix; `..`, absolute paths and links
    // that lead outside are `Denied`.
    let root = package(
        "files",
        "\n[run]\nentry = \"files\"\ngrants = [\"console\", \"files:data:read\", \"files:out:write\"]\n",
        &[
            ("src/files.una", FILES),
            ("data/note.txt", "inside\n"),
            ("secret.txt", "outside\n"),
        ],
    );
    std::fs::create_dir_all(root.join("out")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("../secret.txt", root.join("data/link.txt")).unwrap();
    let output = una_in(&root, &["run"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let link = if cfg!(unix) { "denied" } else { "not found" };
    assert_eq!(
        stdout(&output),
        format!(
            "note.txt: read\nsub/../note.txt: read\n../secret.txt: denied\nlink.txt: {link}\n\
             missing.txt: not found\n/etc/hosts: denied\nlog.txt: written\n../escape.txt: denied\n"
        )
    );
    assert_eq!(
        std::fs::read_to_string(root.join("out/log.txt")).unwrap(),
        "entry"
    );
    assert!(!root.join("escape.txt").exists());
}

const KEYS: &str = "module keys

pub known(name: Text) -> Text
  uses @keys: Keys[hmac]
=
  let mac = attempt @keys.hmac(key_name: name, data: \"payload\")
  when mac
    is Ok -> \"hashed\"
    is Err -> \"unknown key\"

pub main() -> Unit
  uses @console: Console[print]
  uses @keys: Keys[hmac]
=
  let pepper = known(\"pepper\")
  @console.print(\"pepper: {pepper}\")
  let salt = known(\"salt\")
  @console.print(\"salt: {salt}\")
";

#[test]
fn keys_grants_load_their_key_file() {
    // `spec/14 §3`, DECISION D-103: a TOML table of names to base64 keys; key material never
    // enters Una values or traces.
    let root = package(
        "keys",
        "\n[run]\nentry = \"keys\"\ngrants = [\"console\", \"keys:keys.toml\"]\n",
        &[
            ("src/keys.una", KEYS),
            ("keys.toml", "pepper = \"c2VjcmV0LXBlcHBlcg==\"\n"),
        ],
    );
    let output = una_in(&root, &["--json", "run"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let document = json(&output);
    assert_eq!(
        document["output"],
        json!(["pepper: hashed", "salt: unknown key"])
    );
    assert_eq!(
        document["trace"][0]["outcome"],
        json!({"kind": "ok", "value": "<redacted:internal>"})
    );
    assert_eq!(
        document["trace"][2]["outcome"],
        json!({"kind": "fail", "variant": "UnknownKey", "value": "UnknownKey"})
    );
    assert!(!stdout(&output).contains("c2VjcmV0"));
    std::fs::write(root.join("keys.toml"), "pepper = 3\n").unwrap();
    let output = una_in(&root, &["run"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("the key `pepper` is not a base64 string"),
        "{}",
        stderr(&output)
    );
    std::fs::remove_file(root.join("keys.toml")).unwrap();
    let output = una_in(&root, &["run"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("cannot read the key file"));
}

#[test]
fn grants_are_checked_before_running() {
    // `spec/04 §8`: U3012 before execution starts, against `[run] grants` and `--grant`
    // together (DECISION D-109).
    let root = package(
        "clock",
        "\n[run]\nentry = \"clock\"\ngrants = [\"console\"]\n",
        &[(
            "src/clock.una",
            "module clock

pub main() -> Unit
  uses @console: Console[print]
  uses @clock: Clock[now]
=
  let now = @clock.now()
  let started = if now > 0 then \"yes\" else \"no\"
  @console.print(\"started: {started}\")
",
        )],
    );
    let output = una_in(&root, &["--json", "run"]);
    assert_eq!(output.status.code(), Some(1));
    let document = json(&output);
    assert_eq!(document["status"], "rejected");
    assert_eq!(document["diagnostics"][0]["code"], "U3012");
    assert_eq!(
        document["diagnostics"][0]["message"],
        "`main` uses `now` of `Clock`, which neither `[run] grants` nor a `--grant` flag grants"
    );
    let output = una_in(&root, &["run", "--grant", "clock"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "started: yes\n");
    // A single file has no `una.toml`: every grant comes from flags.
    let file = root.join("src/clock.una");
    let output = una(&["run", file.to_str().unwrap(), "--grant", "console"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("[U3012]"));
}

#[test]
fn run_reports_bad_grants_entries_and_mains_as_usage_errors() {
    let output = una(&["run", "examples/hello", "--grant", "network"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        stderr(&output),
        "error: `network` is not a grant; grants are console, clock, env, keys:<file> and files:<path>:<access>\n"
    );
    assert_eq!(stdout(&output), "");
    let root = package(
        "no-entry",
        "",
        &[("src/app.una", "module app\n\npub main() -> Unit = unit\n")],
    );
    let output = una_in(&root, &["run"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        stderr(&output),
        "error: una.toml has no `[run] entry`; name the module whose `main` runs\n"
    );
    let root = package(
        "private-main",
        "\n[run]\nentry = \"app\"\n",
        &[("src/app.una", "module app\n\nmain() -> Unit = unit\n")],
    );
    let output = una_in(&root, &["run"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        stderr(&output),
        "error: the module `app` has no `pub main() -> Unit` to run\n"
    );
}

const DOUBLING: &str = "module doubling

pub kind Refusal = | TooLarge

pub double(x: Int) -> Int
  fails Refusal
=
  if x > 100 then fail TooLarge else x * 2

example \"doubling works\"
  do double(2)
  expect result == 4

example \"doubling is wrong here\"
  do double(2)
  expect result == 5

example \"the failure is not the one expected\"
  do double(2)
  expect fails TooLarge

example \"an unexpected failure\"
  do double(200)
  expect true
";

#[test]
fn test_reports_the_expectations_that_do_not_hold() {
    // `spec/08 §2`; DECISION D-102 and DECISION D-106.
    let root = package("doubling", "", &[("src/doubling.una", DOUBLING)]);
    let output = una_in(&root, &["test"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "example doubling \"doubling works\" ... ok\n\
         example doubling \"doubling is wrong here\" ... FAILED\n  \
         src/doubling.una:16: expect result == 5\n  \
         it is false, with `result` = `4`\n\
         example doubling \"the failure is not the one expected\" ... FAILED\n  \
         src/doubling.una:20: expect fails TooLarge\n  \
         the `do` line completed with `4`\n\
         example doubling \"an unexpected failure\" ... FAILED\n  \
         the `do` line failed with `TooLarge`, and no `expect fails` line expects a failure\n\
         \n\
         4 examples: 1 passed, 3 failed\n"
    );
    let output = una_in(&root, &["--json", "test"]);
    assert_eq!(output.status.code(), Some(1));
    let document = json(&output);
    assert_eq!(document["passed"], 1);
    assert_eq!(document["failed"], 3);
    assert_eq!(
        document["examples"][1],
        json!({
            "module": "doubling",
            "name": "doubling is wrong here",
            "outcome": "fail",
            "reason": "it is false, with `result` = `4`",
            "expectation": {
                "file": "src/doubling.una",
                "line": 16,
                "text": "expect result == 5"
            },
            "result": {"kind": "ok", "value": "4"},
            "trace": []
        })
    );
    assert_eq!(
        document["examples"][3]["result"],
        json!({"kind": "fail", "variant": "TooLarge", "value": "TooLarge"})
    );
}

#[test]
fn run_and_test_do_not_run_rejected_packages() {
    // ADR 0027 rule 1: a program with tier 0 errors is not run.
    let root = package(
        "broken",
        "\n[run]\nentry = \"broken\"\n",
        &[(
            "src/broken.una",
            "module broken\n\npub main() -> Unit = 1\n",
        )],
    );
    let output = una_in(&root, &["run", "--color", "never"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    assert!(!stderr(&output).contains("note:"), "{}", stderr(&output));
    assert!(stderr(&output).contains("[U2001]"), "{}", stderr(&output));
    let output = una_in(&root, &["--json", "test"]);
    assert_eq!(output.status.code(), Some(1));
    let document = json(&output);
    assert_eq!(document["examples"], json!([]));
    assert_eq!(document["diagnostics"][0]["code"], "U2001");
}

#[test]
fn handler_instances_keep_their_ids_and_come_before_their_bodies() {
    // `spec/11 §2`: rebinding shares the instance; §4 E-OpHandler: the operation's event comes
    // before the events of the handler's body; §5: `with:<handler>#<n>` counts installations.
    let output = una(&[
        "--json",
        "run",
        "tests/conformance/runtime/handlers.una",
        "--grant",
        "console",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let document = json(&output);
    let events: Vec<(String, String)> = document["trace"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| {
            (
                event["instance"].as_str().unwrap().to_owned(),
                format!("{}.{}", event["capability"], event["op"]).replace('"', ""),
            )
        })
        .collect();
    let expected = [
        ("with:starting_at#1", "Counter.next"),
        ("host:Console", "Console.print"),
        ("with:console_log#1", "Log.write"),
        ("host:Console", "Console.print"),
        ("with:console_log#1", "Log.write"),
        ("host:Console", "Console.print"),
        ("with:console_log#2", "Log.write"),
        ("host:Console", "Console.print"),
        ("with:console_log#2", "Log.write"),
        ("host:Console", "Console.print"),
    ];
    let expected: Vec<(String, String)> = expected
        .iter()
        .map(|(instance, op)| ((*instance).to_owned(), (*op).to_owned()))
        .collect();
    assert_eq!(events, expected);
    assert_eq!(
        document["trace"][0]["outcome"],
        json!({"kind": "ok", "value": "42"})
    );
}
