//! End-to-end tests of `una run` and `una test` (`spec/11`, `spec/13 §5`, ADR 0032). Both check
//! at tier 1 first (`spec/12 §1`), so they need the solver of `verification.lock`.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pretty_assertions::assert_eq;
use serde_json::{Value, json};

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
    assert_eq!(stderr(&output), "");
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
    assert_eq!(stderr(&output), "");
    let output = una(&["run", "examples/events"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "availability of Launch: open\n");
}

#[test]
fn json_documents_carry_the_tier() {
    // ADR 0032 rules 4 and 5; since M8, `una run` and `una test` check at tier 1 (ADR 0027
    // rule 4).
    let output = una(&["--json", "run", "examples/hello"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stderr(&output), "");
    insta::assert_snapshot!("run_hello_json", stdout(&output));
    assert_eq!(
        json(&output),
        json!({
            "schema": "una.run/1",
            "tier": 1,
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
    assert_eq!(document["tier"], 1);
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
    assert!(
        stderr(&output).starts_with("[U6001]"),
        "{}",
        stderr(&output)
    );
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
fn a_huge_max_depth_reserves_a_bounded_stack_and_ends_with_u6003() {
    // ADR 0061 rule 1 (v0.1 review R-033): the stack reservation is capped at 1 GiB, so a depth
    // the address space could not hold runs, and a recursion that outgrows the capped stack is
    // U6003, never an OS kill.
    let root = package("too-deep", "", &[]);
    let file = root.join("countdown.una");
    std::fs::write(&file, COUNTDOWN.replace("DEPTH", "-1")).unwrap();
    let output = una(&[
        "--json",
        "run",
        file.to_str().unwrap(),
        "--grant",
        "console",
        "--max-depth",
        "100000000000",
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let document = json(&output);
    assert_eq!(document["status"], "aborted");
    assert_eq!(document["diagnostics"][0]["code"], "U6003");
    assert_eq!(
        document["diagnostics"][0]["message"],
        "this call exceeds the maximum call depth of 100000000000"
    );
}

/// A program that holds more text at each call: about 64 bytes times the square of its depth
/// over two, so 64 MiB by about 1,450 calls.
const HOG: &str = "module hog

grow(t: Text, n: Int) -> Int = if n <= 0 then t.len else grow(\"{t}0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\", n: n - 1)

pub main() -> Unit
  uses @console: Console[print]
=
  let size = grow(\"\", n: STEPS)
  @console.print(\"held {size}\")
";

#[test]
fn a_run_over_its_memory_budget_is_u6003() {
    // ADR 0061 rule 2: `--max-memory <MiB>` bounds the heap a run holds; past it, the call that
    // finds it over is U6003, with the status and trace of a depth abort.
    let root = package("hog", "", &[]);
    let file = root.join("hog.una");
    let program = |steps: &str| {
        std::fs::write(&file, HOG.replace("STEPS", steps)).unwrap();
        file.to_str().unwrap().to_owned()
    };
    let small = program("100");
    let output = una(&["run", &small, "--grant", "console", "--max-memory", "64"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "held 6400\n");
    let large = program("9000");
    let output = una(&[
        "--json",
        "run",
        &large,
        "--grant",
        "console",
        "--max-memory",
        "64",
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let document = json(&output);
    assert_eq!(document["status"], "aborted");
    assert_eq!(document["output"], json!([]));
    assert_eq!(document["diagnostics"][0]["code"], "U6003");
    assert_eq!(
        document["diagnostics"][0]["message"],
        "this call exceeds the memory limit of 64 MiB"
    );
    // The call that found the run over its budget.
    assert_eq!(document["diagnostics"][0]["primary"]["start"]["line"], 3);
    let output = una(&["run", &large, "--grant", "console", "--max-memory", "many"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn a_loop_over_its_memory_budget_is_u6003() {
    // ADR 0061 rule 2: a `for` iteration is not a call, and the budget is checked at each one
    // too; the abort points at the loop.
    let root = package("loop-hog", "\n[run]\nentry = \"hog\"\n", &[]);
    let manifest = std::fs::read_to_string(root.join("una.toml")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/hog.una"),
        "module hog

pub kind Node
  value: Int
  next: Option[Node]

pub main() -> Unit
  uses @console: Console[print]
=
  let edit chain: Option[Node] = None
  for i in 0..<100000000
    chain = Some(Node(value: i, next: chain))
  let built = chain.is_some()
  @console.print(\"built {built}\")
",
    )
    .unwrap();
    let dir = root.to_str().unwrap();
    let output = una(&[
        "--json",
        "run",
        dir,
        "--grant",
        "console",
        "--max-memory",
        "16",
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let document = json(&output);
    assert_eq!(document["status"], "aborted");
    assert_eq!(document["diagnostics"][0]["code"], "U6003");
    assert_eq!(
        document["diagnostics"][0]["message"],
        "this call exceeds the memory limit of 16 MiB"
    );
    // The loop, where the iteration found the run over its budget.
    assert_eq!(document["diagnostics"][0]["primary"]["start"]["line"], 11);
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
    // `spec/14 §3`, relative to the prefix; `..`, absolute paths and links
    // that lead outside are `Denied`. Hard links are outside the v0.1 model
    // (ADR 0048, `spec/14 §3` rule 5): a hard link is an ordinary entry, so no
    // case here expects it to be denied.
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

/// The program of the link cases: `main` performs each `(operation, path)` of `cases` in order
/// and prints `<operation> <path>: <outcome>` for each, with the helpers of [`FILES`].
#[cfg(unix)]
fn files_program(cases: &[(&str, &str)]) -> String {
    let helpers = FILES.split("pub main()").next().unwrap();
    let mut program = format!(
        "{helpers}pub main() -> Unit\n  uses @console: Console[print]\n  uses @files: Files[read, write]\n=\n"
    );
    for (index, (operation, path)) in cases.iter().enumerate() {
        let call = if *operation == "read" { "show" } else { "save" };
        program.push_str(&format!(
            "  let r{index} = {call}(\"{path}\")\n  @console.print(\"{operation} {path}: {{r{index}}}\")\n"
        ));
    }
    program
}

#[cfg(unix)]
#[test]
fn files_grants_never_follow_a_final_link_and_keep_directories_inside_r011() {
    // R-011, ADR 0038 D9, `spec/14 §3`: (1) paths normalize without following links, and an
    // absolute path or a `..` that leaves the prefix is `Denied`; (2) every intermediate
    // directory resolves, links followed, inside the canonical prefix; (3) the final component
    // is never followed: a link there, dangling or not, inside or outside, is `Denied`, and a
    // write never creates a file through a link. DECISION D-133: a path naming the prefix
    // itself (`.`, `sub/..`) has no final component and is `Denied`.
    use std::os::unix::fs::symlink;
    let absolute = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("runtime")
        .join("files-links")
        .join("box/note.txt");
    let absolute = absolute.to_str().unwrap();
    let cases = [
        ("write", "dangling"),
        ("read", "dangling"),
        ("read", "out_link"),
        ("write", "out_link"),
        ("read", "in_link"),
        ("write", "in_link"),
        ("read", "out_dir/secret.txt"),
        ("write", "out_dir/new.txt"),
        ("write", "dangling_dir/new.txt"),
        ("read", "in_dir/inner.txt"),
        ("write", "in_dir/new.txt"),
        ("read", "in_dir/../note.txt"),
        ("read", "sub/../note.txt"),
        ("read", "../outside/secret.txt"),
        ("write", "../outside/new.txt"),
        ("read", absolute),
        ("read", "."),
        ("write", "sub/.."),
        ("write", "missing/new.txt"),
        ("write", "note.txt"),
        ("write", "fresh.txt"),
    ];
    let program = files_program(&cases);
    let root = package(
        "files-links",
        "\n[run]\nentry = \"files\"\ngrants = [\"console\", \"files:box:read,write\"]\n",
        &[
            ("src/files.una", &program),
            ("box/note.txt", "inside\n"),
            ("box/sub/inner.txt", "inner\n"),
            ("outside/secret.txt", "secret\n"),
        ],
    );
    let outside = root.join("outside");
    symlink(outside.join("escaped.txt"), root.join("box/dangling")).unwrap();
    symlink(outside.join("secret.txt"), root.join("box/out_link")).unwrap();
    symlink("note.txt", root.join("box/in_link")).unwrap();
    symlink(&outside, root.join("box/out_dir")).unwrap();
    symlink(outside.join("newdir"), root.join("box/dangling_dir")).unwrap();
    symlink("sub", root.join("box/in_dir")).unwrap();
    let output = una_in(&root, &["run"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!(
            "write dangling: denied\n\
             read dangling: denied\n\
             read out_link: denied\n\
             write out_link: denied\n\
             read in_link: denied\n\
             write in_link: denied\n\
             read out_dir/secret.txt: denied\n\
             write out_dir/new.txt: denied\n\
             write dangling_dir/new.txt: denied\n\
             read in_dir/inner.txt: read\n\
             write in_dir/new.txt: written\n\
             read in_dir/../note.txt: read\n\
             read sub/../note.txt: read\n\
             read ../outside/secret.txt: denied\n\
             write ../outside/new.txt: denied\n\
             read {absolute}: denied\n\
             read .: denied\n\
             write sub/..: denied\n\
             write missing/new.txt: not found\n\
             write note.txt: written\n\
             write fresh.txt: written\n"
        )
    );
    // Nothing outside the grant was created or changed.
    assert!(!outside.join("escaped.txt").exists());
    assert!(!outside.join("new.txt").exists());
    assert!(!outside.join("newdir").exists());
    assert_eq!(
        std::fs::read_to_string(outside.join("secret.txt")).unwrap(),
        "secret\n"
    );
    // Writes inside the grant land where the path says; a link is never written through.
    assert_eq!(
        std::fs::read_to_string(root.join("box/sub/new.txt")).unwrap(),
        "entry"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("box/note.txt")).unwrap(),
        "entry"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("box/fresh.txt")).unwrap(),
        "entry"
    );
    assert!(
        std::fs::symlink_metadata(root.join("box/in_link"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
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
    // `spec/14 §3`, a TOML table of names to base64 keys; key material never
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
    // together.
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
    // `spec/08 §2`.
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

/// The keys of every trace event's `args` in a `una.run/1` or `una.test/1` document.
fn trace_keys(document: &Value) -> Vec<String> {
    let mut traces = vec![&document["trace"]];
    if let Some(examples) = document["examples"].as_array() {
        traces.extend(examples.iter().map(|example| &example["trace"]));
    }
    traces
        .into_iter()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|event| event["args"].as_object())
        .flat_map(|args| args.keys().cloned())
        .collect()
}

#[test]
fn trace_arguments_are_keyed_by_source_names() {
    // `spec/11 §5`: each entry of `args` is keyed by the operation parameter's source name,
    // even when Core renamed the binder because two operations share a parameter name.
    let case = "tests/conformance/runtime/op_arg_names";
    let run = una_in(Path::new(case), &["--json", "run", "--grant", "console"]);
    let test = una_in(Path::new(case), &["--json", "test"]);
    for (output, trace) in [
        (&run, json(&run)["trace"].clone()),
        (&test, json(&test)["examples"][0]["trace"].clone()),
    ] {
        assert_eq!(output.status.code(), Some(0), "{}", stdout(output));
        let args: Vec<(Value, Value)> = trace
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["capability"] == "Store")
            .map(|event| (event["op"].clone(), event["args"].clone()))
            .collect();
        assert_eq!(
            args,
            vec![
                (json!("put"), json!({"k": "<redacted:secret>", "v": "3"})),
                (json!("get"), json!({"k": "3"})),
            ]
        );
    }
}

#[test]
fn no_trace_key_carries_a_binder_suffix() {
    // `spec/11 §5`: trace keys are source names, so none contains Core's `%` suffix.
    let mut targets: Vec<PathBuf> = std::fs::read_dir(repository().join("examples"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    targets.extend(
        std::fs::read_dir(repository().join("tests/conformance/runtime"))
            .unwrap()
            .map(|entry| entry.unwrap().path()),
    );
    // Entries starting with `.` are not cases (`compiler/testing.md` §2).
    targets.retain(|path| !path.file_name().unwrap().to_string_lossy().starts_with('.'));
    targets.sort();
    for target in targets {
        let (dir, args): (PathBuf, Vec<String>) = if target.is_dir() {
            (target.clone(), vec!["--json".into(), "test".into()])
        } else {
            let file = target.to_string_lossy().into_owned();
            (repository(), vec!["--json".into(), "test".into(), file])
        };
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = una_in(&dir, &args);
        let document: Value = serde_json::from_str(stdout(&output))
            .unwrap_or_else(|_| panic!("{}: {}", target.display(), stderr(&output)));
        let keys = trace_keys(&document);
        assert!(
            keys.iter().all(|key| !key.contains('%')),
            "{}: {keys:?}",
            target.display()
        );
    }
}

/// The trace of each example of a conformance case under `una --json test`, as the `args` of
/// its events.
fn case_traces(case: &str) -> Vec<Value> {
    let output = una(&["--json", "test", &format!("tests/conformance/{case}")]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    json(&output)["examples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|example| {
            Value::Array(
                example["trace"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|event| event["args"].clone())
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn the_review_traces_run_left_to_right_and_redact_every_labeled_position() {
    // Review controls `runtime-evaluation-order` and `runtime-trace-redaction`: nested effects
    // on a `do` line run left to right and a short-circuited `and` or `or` skips its right
    // operand (`spec/08 §2`); a secret argument is redacted directly and inside a record, a
    // list and an option (`spec/11 §5`).
    assert_eq!(
        case_traces("runtime/evaluation_order_do_lines.una"),
        [
            json!([{"n": "1"}, {"n": "2"}]),
            json!([{"n": "0"}]),
            json!([{"n": "1"}]),
        ]
    );
    let traces = case_traces("runtime/trace_redaction.una");
    assert_eq!(
        traces,
        [json!([{
            "tag": "\"t\"",
            "card": "Card(owner: \"Ana\", number: <redacted:secret>)",
            "extra": "<redacted:secret>",
            "maybe": "<redacted:secret>",
            "raw": "<redacted:secret>"
        }])]
    );
    assert!(!traces[0].to_string().contains("4111-SECRET"));
}

/// The program of the review's `runtime-host-trace-redaction`: an `internal` value from `Env`
/// written to a file, read back and carried by the failure of `main`.
const HOST_REDACTION: &str = "module rr

pub kind Oops = Leak(detail: internal Text) | Plain

pub main() -> Unit
  uses @console: Console[print]
  uses @env: Env[get]
  uses @files: Files[read, write]
  fails Oops
=
  let token = @env.get(\"UNA_SECRET_TOKEN\")
    if none use \"none\"
  @files.write(\"out.txt\", content: token)
    if error fail Plain
  let back = @files.read(\"out.txt\")
    if error fail Plain
  @console.print(\"wrote and read a file\")
  fail Leak(detail: back)
";

#[test]
fn host_values_stay_redacted_through_files_and_the_failure_of_main() {
    // Review control `runtime-host-trace-redaction` (`spec/11 §5`, `spec/04 §8`): the value
    // of an environment variable is `internal`, so neither the trace of `Env.get`,
    // `Files.write` and `Files.read` nor the fields of `main`'s failure show it.
    let root = package(
        "host-redaction",
        "\n[run]\nentry = \"rr\"\ngrants = [\"console\", \"env\", \"files:./data:read,write\"]\n",
        &[("src/rr.una", HOST_REDACTION), ("data/.keep", "")],
    );
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .args(["--json", "run"])
        .env("UNA_SECRET_TOKEN", "hunter2")
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    assert!(!stdout(&output).contains("hunter2"), "{}", stdout(&output));
    assert!(!stderr(&output).contains("hunter2"), "{}", stderr(&output));
    assert_eq!(
        std::fs::read_to_string(root.join("data/out.txt")).unwrap(),
        "hunter2"
    );
    let document = json(&output);
    assert_eq!(document["output"], json!(["wrote and read a file"]));
    assert_eq!(
        document["failure"],
        json!({"variant": "Leak", "fields": {}})
    );
    let events: Vec<(String, Value, Value)> = document["trace"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| {
            (
                format!(
                    "{}.{}",
                    event["capability"].as_str().unwrap(),
                    event["op"].as_str().unwrap()
                ),
                event["args"].clone(),
                event["outcome"].clone(),
            )
        })
        .collect();
    let redacted = json!({"kind": "ok", "value": "<redacted:internal>"});
    let unit = json!({"kind": "ok", "value": "unit"});
    assert_eq!(
        events,
        [
            (
                "Env.get".to_owned(),
                json!({"name": "\"UNA_SECRET_TOKEN\""}),
                redacted.clone()
            ),
            (
                "Files.write".to_owned(),
                json!({"path": "\"out.txt\"", "content": "<redacted:internal>"}),
                unit.clone()
            ),
            (
                "Files.read".to_owned(),
                json!({"path": "\"out.txt\""}),
                redacted
            ),
            (
                "Console.print".to_owned(),
                json!({"line": "\"wrote and read a file\""}),
                unit
            ),
        ]
    );
}

#[test]
fn a_million_deep_value_is_released_compared_and_rendered_within_the_call_depth() {
    // ADR 0060 R5, `spec/11 §7`: a value has no depth limit, and only calls count toward the
    // call depth. Two nested folds build a chain of a million links with a few calls in
    // progress, under `--max-depth 100`. Releasing, comparing and rendering it never recurse
    // on the native stack, and rendering is linear: the failure message of the example holds
    // the whole chain.
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("runtime")
        .join("million");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        "[package]\nname = \"million\"\nversion = \"0.1.0\"\nedition = \"0.2\"\n\n[run]\nentry = \"deep\"\ngrants = [\"console\"]\n",
    )
    .unwrap();
    std::fs::copy(
        repository().join("verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    let numbers: Vec<String> = (0..1000).map(|n| n.to_string()).collect();
    let source = format!(
        "module deep\n\npub kind Chain = End | Link(value: Int, rest: Chain)\n\n\
         --- A chain of a million links, built by two nested folds over a thousand numbers.\n\
         pub build() -> Chain\n=\n  let xs = [{}]\n  xs.fold(initial: End, step: fn(outer: Chain, x: Int) -> xs.fold(initial: outer, step: fn(inner: Chain, y: Int) -> Link(value: y, rest: inner)))\n\n\
         pub main() -> Unit\n  uses @console: Console[print]\n=\n  let a = build()\n  let b = build()\n  let same = a == b\n  @console.print(\"same: {{same}}\")\n\n\
         example \"a million links render in the failure message\"\n  do build()\n  expect result == End\n",
        numbers.join(", ")
    );
    std::fs::write(root.join("src/deep.una"), source).unwrap();
    let started = std::time::Instant::now();
    let output = una_in(&root, &["run", "--max-depth", "100", "."]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "same: true\n");
    let output = una_in(&root, &["test", "--max-depth", "100", "."]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("FAILED"), "{}", &text[..text.len().min(500)]);
    // Every link is rendered: about 24 bytes each.
    assert!(text.len() > 20_000_000, "{}", text.len());
    assert!(text.contains("Link(value: 0, rest: End)"));
    // Linear, not quadratic: a quadratic render of a million levels takes most of an hour.
    assert!(started.elapsed().as_secs() < 120, "{:?}", started.elapsed());
}
