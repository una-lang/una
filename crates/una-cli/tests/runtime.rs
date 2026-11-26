//! End-to-end tests of `una run` and `una test` (`spec/11`, `spec/13 §5`, ADR 0032). Both check
//! at tier 1 first (`spec/12 §1`), so they need the solver of `verification.lock`.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/cpu.rs"]
mod cpu;
#[path = "common/hermetic.rs"]
mod hermetic;
#[path = "common/timing.rs"]
mod timing;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pretty_assertions::assert_eq;
use serde_json::{Value, json};

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A sealed copy of the examples and of the conformance corpus, which the tests run in place
/// of the checkout's: no manifest above the checkout changes the edition of their single files
/// (#1560).
fn corpus() -> PathBuf {
    hermetic::checkout(&["examples", "tests/conformance"])
}

/// The command that runs the `una` binary in `dir`, relative to the root of [`corpus`], or
/// absolute.
fn una_command(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_una"));
    command
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(args)
        .current_dir(corpus().join(dir));
    command
}

/// Runs the `una` binary in `dir`, relative to the root of [`corpus`], or absolute.
fn una_in(dir: &Path, args: &[&str]) -> Output {
    una_command(dir, args).output().unwrap()
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

/// An example whose `do` line returns a record with a labeled field, and fails.
const BADGE: &str = "module badge

pub kind Badge
  holder: Text
  code: secret Text

pub make(code: secret Text) -> Badge = Badge(holder: \"Ana\", code: code)

example \"a badge holds its code\"
  do make(\"QX-7731\")
  expect result.holder == \"Bo\"
";

#[test]
fn example_results_redact_labeled_fields_in_edition_0_1_only() {
    // ADR 0078: an example's data come from the source, so edition 0.2 shows its results and
    // reasons with every field; edition 0.1 keeps its output, with labeled fields redacted.
    // The document says which rule it follows in `redacted`.
    let root = package("badge-0-1", "", &[("src/badge.una", BADGE)]);
    let output = una(&["--json", "test", root.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let document = json(&output);
    assert_eq!(document["redacted"], true);
    let redacted = "Badge(holder: \"Ana\", code: <redacted:secret>)";
    assert_eq!(document["examples"][0]["result"]["value"], redacted);
    assert_eq!(
        document["examples"][0]["reason"],
        format!("it is false, with `result` = `{redacted}`")
    );
    let root = package("badge-0-2", "", &[("src/badge.una", BADGE)]);
    let manifest = root.join("una.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    let output = una(&["--json", "test", root.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let document = json(&output);
    assert_eq!(document["redacted"], false);
    let shown = "Badge(holder: \"Ana\", code: \"QX-7731\")";
    assert_eq!(document["examples"][0]["result"]["value"], shown);
    assert_eq!(
        document["examples"][0]["reason"],
        format!("it is false, with `result` = `{shown}`")
    );
    let output = una(&["test", root.to_str().unwrap()]);
    assert!(
        stdout(&output).contains(&format!("  it is false, with `result` = `{shown}`\n")),
        "{}",
        stdout(&output)
    );
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
// Each level reads its `t` after the call, so it keeps its own copy of the text: the program
// holds O(n^2) bytes. A level that gave `t` up would let the callee append in place (#261,
// ADR 0072), and the program would hold only O(n).
const HOG: &str = "module hog

grow(t: Text, n: Int) -> Int = if n <= 0 then t.len else max(grow(\"{t}0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\", n: n - 1), b: t.len)

pub main() -> Unit
  uses @console: Console[print]
=
  let size = grow(\"\", n: STEPS)
  @console.print(\"held {size}\")
";

/// A loop of `STEPS` iterations whose state is a counter and a list that `append` or `set`
/// updates (#201).
const UPDATES: &str = "module updates

pub main() -> Unit
  uses @console: Console[print]
=
  let edit n = 0
  let edit xs: List[Int] = []
  for i in 0..<STEPS
    n = n + 1
    xs = xs.append(item: i)
  let kept = xs
  let edit ys = xs
  for i in 0..<STEPS
    if i < ys.len
      edit ys.set(index: i, item: 7)
  let first = when kept.first()
    is Some(v) -> v
    is None -> -1
  let changed = when ys.first()
    is Some(v) -> v
    is None -> -1
  @console.print(\"{n} {xs.len} {first} {changed}\")
";

#[test]
fn checking_a_body_takes_linear_time_in_its_locals() {
    // #326: the checker resolves a local by an index from its name, and saves the taken state
    // of the locals around a branch in proportion to the taken ones, so a body of n bindings,
    // read again under `if`s, and of editable bindings updated in loops, checks in linear
    // time. The fixed cost of a check is measured on 10 bindings, the best of three, and taken
    // out of both sizes: 8x the bindings took about 7x as long measured, and about 60x
    // when each lookup scanned every local; the bound of 24x is between them. A miss is
    // measured twice more before it fails (#341).
    let root = package("many-locals", "", &[]);
    let manifest = std::fs::read_to_string(root.join("una.toml")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let dir = root.to_str().unwrap();
    // Writes the body of `bindings` bindings (untimed), then times its check.
    let measure = |bindings: usize| {
        let mut source = String::from(
            "module many\n\npub build(x: Int) -> Int\n=\n  let first = x\n  let edit acc = 0\n",
        );
        // Bindings each read again, some under an `if`, and editable bindings each updated in
        // a loop: two shapes that were quadratic.
        for i in 0..bindings / 2 {
            source.push_str(&format!(
                "  let v{i} = first + {i}\n  if v{i} > 3\n    acc = acc + 1\n"
            ));
        }
        for i in 0..bindings / 2 {
            source.push_str(&format!(
                "  let edit e{i} = first\n  for k in 0..<2\n    e{i} = e{i} + k\n"
            ));
        }
        source.push_str("  acc\n");
        std::fs::write(root.join("src/many.una"), source).unwrap();
        let (output, seconds) = timing::timed(|| una(&["check", "--tier", "0", dir]));
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        seconds
    };
    let fixed = timing::best_of_three(|| measure(10));
    timing::assert_growth(
        "bindings",
        &[2_000, 16_000],
        fixed,
        timing::FLOOR,
        24.0,
        measure,
    );
}

#[test]
fn checking_deeply_nested_types_is_not_cubic_in_their_depth() {
    // #331: unification and resolution follow solved variables by reference instead of
    // copying their whole value at each node, where checking was cubic in d. ADR 0077 (#335)
    // shares the checker's type nodes, keeps unchanged ones, erases each labeled node once per
    // declaration and resolves each variable once at its end, so the checker makes nodes in
    // proportion to the program (`una-check`'s scaling test counts them). What stays
    // quadratic is outside the checker's types: Core IR gives each binder its own owned type
    // (ADR 0080 keeps it); the editor index keeps each expression's type as a shared copy and
    // prints it only when a hover asks (ADR 0080, option B). The
    // fixed cost is measured at depth 10, the best of three, and taken out of both sizes:
    // 4x the depth took about 14x as long measured alone (20x before ADR 0077), and up
    // to about 25x under the load of the whole test suite; quadratic is 16x, and it was about
    // 64x when it was cubic. The bound of 40x keeps the margin that load needs and still fails
    // on cubic growth; the scaling test of `una-check` counts the checker's nodes exactly. A
    // miss is measured twice more before it fails (#341).
    let root = package("deep-nesting", "", &[]);
    let manifest = std::fs::read_to_string(root.join("una.toml")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let dir = root.to_str().unwrap();
    // Writes the types `depth` deep (untimed), then times their check.
    let measure = |depth: usize| {
        let mut value = String::from("1");
        for level in 0..depth {
            value = if level % 2 == 0 {
                format!("[{value}]")
            } else {
                format!("Some({value})")
            };
        }
        let mut source = String::from("module deep\n\npub f() -> Int\n=\n");
        for k in 0..10 {
            source.push_str(&format!("  let edit x{k} = {value}\n  x{k} = {value}\n"));
        }
        source.push_str("  1\n");
        std::fs::write(root.join("src/deep.una"), source).unwrap();
        let (output, seconds) = timing::timed(|| una(&["check", "--tier", "0", dir]));
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        seconds
    };
    let fixed = timing::best_of_three(|| measure(10));
    timing::assert_growth("depth", &[40, 160], fixed, timing::FLOOR, 40.0, measure);
}

#[test]
fn checking_many_generic_functions_is_linear_in_their_number() {
    // #375: the least fixpoint of result summaries built each component's callers from every
    // edge, checked one function by scanning every declaration, and the index looked up each
    // recorded type's declaration by a linear search; each was quadratic in the number of
    // functions. They are now one pass over the edges, a map from function to declaration
    // and a binary search: O((n + E) log n). A chain of n generic functions, each calling the
    // previous one, is checked at two sizes; the fixed cost is measured on 50 functions, the
    // best of three, and taken out of both. 4x the functions took about 4x as long
    // measured, and about 8.6x before the fix; the bound of 6.5x is between them. A miss is
    // measured twice more before it fails (#341).
    let root = package("many-generics", "", &[]);
    let manifest = std::fs::read_to_string(root.join("una.toml")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let dir = root.to_str().unwrap();
    // Writes the chain of `count` functions (untimed), then times its check.
    let measure = |count: usize| {
        let functions: Vec<String> = (0..count)
            .map(|i| {
                let body = if i == 0 {
                    "[Some(x), None]".to_owned()
                } else {
                    format!("f{}(x, k: k, y: y)", i - 1)
                };
                format!("f{i}[T](x: T, k: Int, y: T) -> List[Option[T]] = {body}\n")
            })
            .collect();
        let source = format!("module chain\n\n{}", functions.join("\n"));
        std::fs::write(root.join("src/chain.una"), source).unwrap();
        let (output, seconds) = timing::timed(|| una(&["check", "--tier", "0", dir]));
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        seconds
    };
    let fixed = timing::best_of_three(|| measure(50));
    timing::assert_growth(
        "functions",
        &[2000, 8000],
        fixed,
        timing::FLOOR,
        6.5,
        measure,
    );
}

#[test]
fn a_text_appended_in_a_loop_grows_in_linear_cpu_time() {
    // Each run is measured by the processor time of `una` (`cpu::cpu_output`, #1662), which
    // other tests running beside it do not change, so the test needs no exclusive slot.
    // ADR 0070 X10, `spec/22 §6` rule 2 (#261): `t = "{t}x"` appends in place when nothing
    // else holds `t`, so ten times the appends take about ten times as long, not a hundred. The
    // interpreter counts no copies, so the check is timing. The fixed cost of a run (checking,
    // starting) is measured on 10 appends, once the proof cache is warm, the best of three, and
    // taken out of both sizes, so the ratio is the growth of the appends alone: about 11x
    // measured, about 490x when every append copies, with a bound of 30x between them.
    // A miss is measured twice more before it fails (#341).
    let root = package("text-append", "\n[run]\nentry = \"grow\"\n", &[]);
    let manifest = std::fs::read_to_string(root.join("una.toml")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let dir = root.to_str().unwrap();
    // Writes the program of `steps` appends (untimed), then times its run.
    let measure = |steps: usize| {
        std::fs::write(
            root.join("src/grow.una"),
            format!(
                "module grow\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit t = \"\"\n  for i in 0..<{steps}\n    t = \"{{t}}x\"\n  @console.print(\"{{t.len}}\")\n"
            ),
        )
        .unwrap();
        let (output, seconds) = cpu::cpu_output(&una_command(
            Path::new("."),
            &["run", dir, "--grant", "console"],
        ));
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        assert_eq!(stdout(&output), format!("{steps}\n"));
        seconds
    };
    // The first run of 10 warms the proof cache; the next three measure the fixed cost.
    measure(10);
    let fixed = timing::best_of_three(|| measure(10));
    timing::assert_growth("appends", &[200_000, 2_000_000], fixed, 0.05, 30.0, measure);
}

#[test]
fn a_loop_runs_in_constant_memory_and_updates_a_list_in_place() {
    // #201: a loop whose state takes constant space runs in constant space, and `append` and
    // `set` on a list nothing else holds update it in place, in linear time, as the native code
    // does (#30). A list that another binding holds is copied: `kept` keeps its values while
    // `ys` changes. 1,000,000 iterations fit in 64 MiB; with a copy at each update, 100,000
    // appends took about two minutes in a debug build, and now take well under a second.
    let root = package("updates", "\n[run]\nentry = \"updates\"\n", &[]);
    let manifest = std::fs::read_to_string(root.join("una.toml")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let file = root.join("src/updates.una");
    let dir = root.to_str().unwrap();
    for (steps, expected) in [("3", "3 3 0 7"), ("100000", "100000 100000 0 7")] {
        std::fs::write(&file, UPDATES.replace("STEPS", steps)).unwrap();
        let started = std::time::Instant::now();
        let output = una(&["run", dir, "--grant", "console", "--max-memory", "64"]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        assert_eq!(stdout(&output), format!("{expected}\n"));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(30),
            "{steps} updates took {:?}",
            started.elapsed()
        );
    }
    std::fs::write(
        &file,
        "module updates\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit n = 0\n  for i in 0..<1000000\n    n = n + 1\n  @console.print(\"{n}\")\n",
    )
    .unwrap();
    let output = una(&["run", dir, "--grant", "console", "--max-memory", "64"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "1000000\n");
}

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
fn a_memory_budget_of_zero_is_a_usage_error() {
    // HOST-4 (#185): no run fits in 0 MiB, so `--max-memory 0` is a usage error, exit code 2,
    // on `una run` and `una test`, before anything runs.
    let root = package("hog-zero", "", &[]);
    let file = root.join("hog.una");
    std::fs::write(&file, HOG.replace("STEPS", "1")).unwrap();
    let file = file.to_str().unwrap();
    for args in [
        ["run", file, "--grant", "console", "--max-memory", "0"].as_slice(),
        ["test", file, "--max-memory", "0"].as_slice(),
    ] {
        let output = una(args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            stderr(&output)
        );
        assert_eq!(stdout(&output), "", "{args:?}");
        assert!(
            stderr(&output).contains("--max-memory"),
            "{args:?}: {}",
            stderr(&output)
        );
    }
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
    // write never creates a file through a link. D-133: a path naming the prefix
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
    // S-45 (#1552, `spec/11 §5`): `hmac`'s result is `internal`, so its outcomes are redacted,
    // its failure `UnknownKey` too; the program's output shows it.
    assert_eq!(
        document["trace"][0]["outcome"],
        json!({"kind": "redacted", "level": "internal"})
    );
    assert_eq!(
        document["trace"][2]["outcome"],
        json!({"kind": "redacted", "level": "internal"})
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
fn check_and_run_agree_on_the_listed_grants() {
    // #826, #925, `spec/13 §5`: a listed `[run] grants` is the final set, an empty one included,
    // so `una check` fails closed with U3012 wherever `una run` would stop with it. A package
    // that lists no grants leaves them to `--grant`: `una check` accepts it, and `una run`
    // without a flag reports U3012 before the program starts.
    const MAIN: &str = "module hello

pub main() -> Unit
  uses @console: Console[print]
=
  @console.print(\"hi\")
";
    let u3012 = "`main` uses `print` of `Console`, which neither `[run] grants` nor a `--grant` flag grants";
    for (name, run, check_rejects) in [
        (
            "grants-empty",
            "\n[run]\nentry = \"hello\"\ngrants = []\n",
            true,
        ),
        ("grants-absent", "\n[run]\nentry = \"hello\"\n", false),
        (
            "grants-lacking",
            "\n[run]\nentry = \"hello\"\ngrants = [\"clock\"]\n",
            true,
        ),
        (
            "grants-granted",
            "\n[run]\nentry = \"hello\"\ngrants = [\"console\"]\n",
            false,
        ),
    ] {
        let root = package(name, run, &[("src/hello.una", MAIN)]);
        let check = una_in(&root, &["--json", "check"]);
        let document = json(&check);
        if check_rejects {
            assert_eq!(check.status.code(), Some(1), "{name}: {document}");
            assert_eq!(document["diagnostics"][0]["code"], "U3012", "{name}");
            assert_eq!(
                document["diagnostics"][0]["message"],
                "`main` uses `print` of `Console`, which `[run] grants` does not grant",
                "{name}"
            );
            assert_eq!(
                document["diagnostics"][0]["primary"],
                json!({"file": "src/hello.una", "start": {"line": 4, "col": 3}, "end": {"line": 4, "col": 32}}),
                "{name}"
            );
        } else {
            assert_eq!(check.status.code(), Some(0), "{name}: {document}");
            assert_eq!(document["diagnostics"], json!([]), "{name}");
        }
        let output = una_in(&root, &["--json", "run"]);
        if name == "grants-granted" {
            assert_eq!(output.status.code(), Some(0), "{name}: {}", stderr(&output));
            assert_eq!(json(&output)["output"], json!(["hi"]), "{name}");
        } else {
            // `una run` stops every package but the granted one, `grants-absent` included.
            assert_eq!(output.status.code(), Some(1), "{name}");
            let document = json(&output);
            assert_eq!(document["status"], "rejected", "{name}");
            assert_eq!(document["diagnostics"][0]["code"], "U3012", "{name}");
            assert_eq!(document["diagnostics"][0]["message"], u3012, "{name}");
        }
    }
}

#[test]
fn run_reports_bad_grants_entries_and_mains_as_usage_errors() {
    let output = una(&["run", "examples/hello", "--grant", "network"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        stderr(&output),
        "error: `network` is not a grant; grants are console, clock, env, keys:<file>, files:<path>:<access> and http:<origin>[:<methods>]\n"
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

/// The keys of every trace event's `args` in a `una.run/1`, `una.test/1` or `una.test/2`
/// document.
fn trace_keys(document: &Value) -> Vec<String> {
    let mut traces = vec![&document["trace"]];
    let runs = if document["schema"] == "una.test/2" {
        "tests"
    } else {
        "examples"
    };
    if let Some(examples) = document[runs].as_array() {
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
    let mut targets: Vec<PathBuf> = std::fs::read_dir(corpus().join("examples"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    targets.extend(
        std::fs::read_dir(corpus().join("tests/conformance/runtime"))
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
            (corpus(), vec!["--json".into(), "test".into(), file])
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
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
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
    // S-45 (#1552, `spec/11 §5`): an outcome whose result is `internal` is redacted whole.
    let redacted = json!({"kind": "redacted", "level": "internal"});
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
    // Linear, not quadratic: a quadratic render of a million levels takes most of an hour. The
    // ceiling is a single generous deadline, not a ratio, wide enough for a loaded machine
    // (#341).
    assert!(started.elapsed().as_secs() < 300, "{:?}", started.elapsed());
}

#[test]
fn together_traces_list_tasks_in_branch_order_with_their_paths() {
    // spec/19 §7: an event inside a task carries the task's path, nested ones joined by `.`;
    // events outside any task carry none; `seq` and installation ids follow branch order.
    let root = package(
        "tasks",
        "\n[run]\nentry = \"tasks\"\ngrants = [\"console\"]\n",
        &[],
    );
    let manifest = std::fs::read_to_string(root.join("una.toml")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/tasks.una"),
        "module tasks

pub capability Log
  op write(line: Text) -> Unit

handler quiet() for Log
  op write(line) = unit

shout(line: Text) -> Int
  uses @console: Console[print]
=
  @console.print(line)
  1

note(line: Text) -> Int
  uses @log: Log[write]
=
  @log.write(line)
  1

inner() -> Int
  uses @console: Console[print]
=
  together
    let p = shout(\"b1\")
    let q = note(\"b2\") with @log = quiet()
  p + q

pub main() -> Unit
  uses @console: Console[print]
=
  @console.print(\"before\")
  together
    let a = shout(\"a\")
    let b = inner()
  @console.print(\"after {a + b}\")
",
    )
    .unwrap();
    let output = una(&["--json", "run", root.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    let document = json(&output);
    let events: Vec<Value> = document["trace"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| {
            json!({
                "seq": event["seq"],
                "task": event.get("task").cloned().unwrap_or(Value::Null),
                "instance": event["instance"],
                "args": event["args"],
            })
        })
        .collect();
    assert_eq!(
        events,
        vec![
            json!({"seq": 1, "task": null, "instance": "host:Console", "args": {"line": "\"before\""}}),
            json!({"seq": 2, "task": "1", "instance": "host:Console", "args": {"line": "\"a\""}}),
            json!({"seq": 3, "task": "2.1", "instance": "host:Console", "args": {"line": "\"b1\""}}),
            json!({"seq": 4, "task": "2.2", "instance": "with:quiet#1", "args": {"line": "\"b2\""}}),
            json!({"seq": 5, "task": null, "instance": "host:Console", "args": {"line": "\"after 3\""}}),
        ]
    );
}

/// Runs the `una` binary in the repository root with the descriptor `fd` closed, as
/// `una ... 1>&-` does in a shell.
fn una_closed(fd: u8, args: &[&str]) -> Output {
    Command::new("sh")
        .arg("-c")
        .arg(format!("exec \"$0\" \"$@\" {fd}>&-"))
        .arg(env!("CARGO_BIN_EXE_una"))
        .args(args)
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .current_dir(corpus())
        .output()
        .unwrap()
}

const PRINTS: &str = "module closed\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"hello\")\n";

const FAILS: &str = "module closed\n\npub kind Stop = | Stopped\n\npub main() -> Unit\n  fails Stop\n=\n  fail Stopped\n";

#[test]
fn a_closed_stdout_discards_the_output() {
    // `spec/13 §5` (#259): writing to a closed stdout counts as discarded output, as
    // `/dev/null` does, so the run, its `--json` document and `una test` exit as they would
    // with stdout open.
    let prints = package(
        "closed-stdout",
        "\n[run]\nentry = \"closed\"\n",
        &[("src/closed.una", PRINTS)],
    );
    let dir = prints.to_str().unwrap();
    let run = una_closed(1, &["run", dir, "--grant", "console"]);
    assert_eq!(run.status.code(), Some(0), "{}", stderr(&run));
    assert_eq!(stderr(&run), "");
    let json = una_closed(1, &["--json", "run", dir, "--grant", "console"]);
    assert_eq!(json.status.code(), Some(0), "{}", stderr(&json));
    let test = una_closed(1, &["test", "examples/events"]);
    assert_eq!(test.status.code(), Some(0), "{}", stderr(&test));
}

#[test]
fn a_closed_stderr_discards_the_diagnostics() {
    // #259: with stderr closed, the U6001 of a failing `main` is discarded, and the run
    // still exits with 1, as it does with stderr open.
    let fails = package(
        "closed-stderr",
        "\n[run]\nentry = \"closed\"\n",
        &[("src/closed.una", FAILS)],
    );
    let run = una_closed(2, &["run", fails.to_str().unwrap()]);
    assert_eq!(run.status.code(), Some(1), "{run:?}");
    let open = una(&["run", fails.to_str().unwrap()]);
    assert_eq!(open.status.code(), Some(1), "{}", stderr(&open));
    assert!(stderr(&open).contains("U6001"), "{}", stderr(&open));
}

/// A program that prints more than a pipe holds, so that a write meets a reader that went
/// away.
const FLOODS: &str = "module closed\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  for i in 0..<20000\n    @console.print(\"a line of output that fills the pipe\")\n";

#[test]
fn a_reader_that_went_away_is_a_failed_write() {
    // `spec/13 §5`: unlike a closed stdout, a broken pipe (EPIPE) is a failure to write
    // output, which exits with code 2 (#259).
    let floods = package(
        "broken-pipe",
        "\n[run]\nentry = \"closed\"\n",
        &[("src/closed.una", FLOODS)],
    );
    let manifest = std::fs::read_to_string(floods.join("una.toml")).unwrap();
    std::fs::write(
        floods.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(["run", floods.to_str().unwrap(), "--grant", "console"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
}

const MAPS: &str = "module maps

pub kind Color = Red | Green | Blue

pub main() -> Unit
  uses @console: Console[print]
=
  let edit counts: Map[Text, Int] = empty_map()
  for word in [\"b\", \"a\", \"b\", \"c\", \"b\"]
    let n = counts.get(key: word)
      if none use 0
    edit counts.set(key: word, value: n + 1)
  @console.print(\"{counts}\")
  let before = counts
  edit counts.remove(key: \"b\")
  @console.print(\"{counts} {before} {counts.len} {before.len}\")
  @console.print(\"{counts.keys()} {counts.values()} {counts.entries()}\")
  let none_yet: Map[Int, Bool] = empty_map()
  @console.print(\"{none_yet} {none_yet.has(key: 1)} {none_yet.get(key: 1)}\")
  let by_int = none_yet.with(key: -3, value: true).with(key: 10, value: false).with(key: 2, value: true)
  @console.print(\"{by_int}\")
  let by_color: Map[Color, Text] = empty_map().with(key: Blue, value: \"b\").with(key: Red, value: \"r\")
  @console.print(\"{by_color} {by_color == by_color} {by_color == by_color.without(key: Red)}\")
";

#[test]
fn maps_list_their_entries_in_key_order_and_keep_value_semantics() {
    // `spec/21 §2` rules 3 and 4, `§5`: entries in ascending key order, for integers, texts
    // and variants; `{k: v}` rendering; a `remove` leaves another binding's map as it was;
    // `==` is structural.
    let root = package(
        "maps",
        "\n[run]\nentry = \"maps\"\n",
        &[("src/maps.una", MAPS)],
    );
    let manifest = std::fs::read_to_string(root.join("una.toml")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    let output = una(&["run", root.to_str().unwrap(), "--grant", "console"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "{\"a\": 1, \"b\": 3, \"c\": 1}\n\
         {\"a\": 1, \"c\": 1} {\"a\": 1, \"b\": 3, \"c\": 1} 2 3\n\
         [\"a\", \"c\"] [1, 1] [Entry(key: \"a\", value: 1), Entry(key: \"c\", value: 1)]\n\
         {} false None\n\
         {-3: true, 2: true, 10: false}\n\
         {Red: \"r\", Blue: \"b\"} true false\n"
    );
}

const MAP_SCALE: &str = "module scale

pub main() -> Unit
  uses @console: Console[print]
=
  let edit m: Map[Int, Int] = empty_map()
  for i in 0..<N
    edit m.set(key: (i * 7919) mod N, value: i)
  let edit found = 0
  for i in 0..<N
    if m.has(key: i)
      found = found + 1
  for i in 0..<N
    edit m.remove(key: i)
  @console.print(\"{found} {m.len}\")
";

#[test]
fn map_updates_and_lookups_scale_as_n_log_n_in_cpu_time() {
    // The performance rule, `spec/21 §5` rule 2: `set`, `has` and `remove` are O(log n), and an
    // update of a map nothing else holds is in place. Ten times the keys take under 40 times
    // as long: n log n gives about 12, a copy per update about 100, and the bound of 40x is
    // between them. A miss is measured twice more before it fails (#341).
    let root = package("map_scale", "\n[run]\nentry = \"scale\"\n", &[]);
    let manifest = std::fs::read_to_string(root.join("una.toml")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let dir = root.to_str().unwrap();
    // Writes the program of `n` keys (untimed), then times its run.
    let measure = |n: usize| {
        std::fs::write(
            root.join("src/scale.una"),
            MAP_SCALE.replace('N', &n.to_string()),
        )
        .unwrap();
        let (output, seconds) = cpu::cpu_output(&una_command(
            Path::new("."),
            &["run", dir, "--grant", "console", "--max-memory", "1024"],
        ));
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        assert_eq!(stdout(&output), format!("{n} 0\n"));
        seconds
    };
    timing::assert_growth(
        "keys",
        &[100_000, 1_000_000],
        0.0,
        timing::FLOOR,
        40.0,
        measure,
    );
}

const SEQUENCE_SCALE: &str = "module scale

import std.collections (Queue, Stack, empty_queue, empty_stack)

pub main() -> Unit
  uses @console: Console[print]
=
  var q: Queue[Int] = empty_queue()
  var s: Stack[Int] = empty_stack()
  for i in 0..<COUNT
    edit q.push(item: i)
    edit s.push(item: i)
  for i in 0..<COUNT
    const front = edit q.pop()
    const top = edit s.pop()
    unit
  @console.print(\"{q.len} {s.len}\")
";

#[test]
fn queue_and_stack_pushes_and_pops_scale_linearly_in_cpu_time() {
    // ADR 0119 rules 5 and 6: `push` and `pop` are O(1) amortized, and an update of a sequence
    // nothing else holds is in place. Ten times the items take under 30 times as long: linear
    // time gives about 10, a copy per update about 100. A miss is measured twice more before it
    // fails (#341).
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("runtime")
        .join("sequence_scale");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("ita.una"),
        "ita sequence_scale\n  version 0.1.0\n  edition 0.5\n\nrun\n  entry scale\n  grants\n    console\n",
    )
    .unwrap();
    std::fs::copy(
        repository().join("tests/conformance/runtime/stack_queue_run/ita.lock"),
        root.join("ita.lock"),
    )
    .unwrap();
    let dir = root.to_str().unwrap();
    // Writes the program of `n` items (untimed), then times its run.
    let measure = |n: usize| {
        std::fs::write(
            root.join("src/scale.una"),
            SEQUENCE_SCALE.replace("COUNT", &n.to_string()),
        )
        .unwrap();
        let (output, seconds) = cpu::cpu_output(&una_command(
            Path::new("."),
            &["run", dir, "--grant", "console", "--max-memory", "1024"],
        ));
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        assert_eq!(stdout(&output), "0 0\n");
        seconds
    };
    timing::assert_growth(
        "items",
        &[100_000, 1_000_000],
        0.0,
        timing::FLOOR,
        30.0,
        measure,
    );
}

/// A `Set` of `COUNT` items built by `add`, each looked up, intersected with a copy missing one
/// item, then emptied by `remove` (`spec/28 §6`).
const SET_SCALE: &str = "module scale

import std.collections (Set, empty_set)

pub main() -> Unit
  uses @console: Console[print]
=
  var s: Set[Int] = empty_set()
  for i in 0..<COUNT
    edit s.add(item: (i * 7919) mod COUNT)
  var found = 0
  for i in 0..<COUNT
    if s.has(item: i)
      found = found + 1
  const common = s.intersection(other: s.without(item: 0))
  for i in 0..<COUNT
    edit s.remove(item: i)
  @console.print(\"{found} {common.len} {s.len}\")
";

#[test]
fn set_updates_lookups_and_operations_scale_as_n_log_n_in_cpu_time() {
    // `spec/28 §6`: `add`, `has` and `remove` are O(log n), `intersection` is O(n + m), and an
    // update of a set nothing else holds is in place. Ten times the items take under 40 times
    // as long: n log n gives about 12, a copy per update about 100, and the bound of 40x is
    // between them, as for maps.
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("runtime")
        .join("set_scale");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("ita.una"),
        "ita set_scale\n  version 0.1.0\n  edition 0.5\n\nrun\n  entry scale\n  grants\n    console\n",
    )
    .unwrap();
    std::fs::copy(
        repository().join("tests/conformance/names/collections_exports_in_0_5/ita.lock"),
        root.join("ita.lock"),
    )
    .unwrap();
    let dir = root.to_str().unwrap();
    // Writes the program of `n` items (untimed), then times its run.
    let measure = |n: usize| {
        std::fs::write(
            root.join("src/scale.una"),
            SET_SCALE.replace("COUNT", &n.to_string()),
        )
        .unwrap();
        let (output, seconds) = cpu::cpu_output(&una_command(
            Path::new("."),
            &["run", dir, "--grant", "console", "--max-memory", "1024"],
        ));
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        assert_eq!(stdout(&output), format!("{n} {} 0\n", n - 1));
        seconds
    };
    timing::assert_growth(
        "items",
        &[100_000, 1_000_000],
        0.0,
        timing::FLOOR,
        40.0,
        measure,
    );
}

/// A `HashMap` and a `HashSet` of `COUNT` keys built by `set` and `add`, each key looked up in
/// both, a set intersected with a copy missing one item, then both emptied by `remove`
/// (`spec/28 §7`).
const HASH_SCALE: &str = "module scale

import std.collections (HashMap, HashSet, empty_hash_map, empty_hash_set)

pub main() -> Unit
  uses @console: Console[print]
=
  var m: HashMap[Int, Int] = empty_hash_map()
  for i in 0..<COUNT
    edit m.set(key: (i * 7919) mod COUNT, value: i)
  var s: HashSet[Int] = empty_hash_set()
  for i in 0..<COUNT
    edit s.add(item: i)
  var found = 0
  for i in 0..<COUNT
    if m.has(key: i) and s.has(item: i)
      found = found + 1
  const common = s.intersection(other: s.without(item: 0))
  for i in 0..<COUNT
    edit m.remove(key: i)
    edit s.remove(item: i)
  @console.print(\"{found} {common.len} {m.len} {s.len}\")
";

#[test]
fn hash_table_updates_lookups_and_operations_scale_linearly_in_cpu_time() {
    // `spec/28 §7`: `set`, `add`, `has` and `remove` are O(|k|) expected, amortized, and
    // `intersection` is O(n · |k|); an update of a table nothing else holds is in place. Ten
    // times the keys take under 25 times as long: expected O(1) per operation gives about 10,
    // a copy per update or a hash that collides about 100.
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("runtime")
        .join("hash_scale");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("ita.una"),
        "ita hash_scale\n  version 0.1.0\n  edition 0.5\n\nrun\n  entry scale\n  grants\n    console\n",
    )
    .unwrap();
    std::fs::copy(
        repository().join("tests/conformance/names/collections_exports_in_0_5/ita.lock"),
        root.join("ita.lock"),
    )
    .unwrap();
    let dir = root.to_str().unwrap();
    // Writes the program of `n` keys (untimed), then times its run.
    let measure = |n: usize| {
        std::fs::write(
            root.join("src/scale.una"),
            HASH_SCALE.replace("COUNT", &n.to_string()),
        )
        .unwrap();
        let (output, seconds) = cpu::cpu_output(&una_command(
            Path::new("."),
            &["run", dir, "--grant", "console", "--max-memory", "1024"],
        ));
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        assert_eq!(stdout(&output), format!("{n} {} 0 0\n", n - 1));
        seconds
    };
    timing::assert_growth(
        "keys",
        &[100_000, 1_000_000],
        0.0,
        timing::FLOOR,
        25.0,
        measure,
    );
}

/// A `PriorityQueue` of `COUNT` items pushed with priorities that repeat, so that ties are
/// common, then popped empty, checking that the priorities come out in order (`spec/28 §8`).
const PRIORITY_SCALE: &str = "module scale

import std.collections (PriorityQueue, lowest_first)

pub main() -> Unit
  uses @console: Console[print]
=
  var q: PriorityQueue[Int, Int] = lowest_first()
  for i in 0..<COUNT
    edit q.push(item: i, priority: (i * 7919) mod 1000)
  var popped = 0
  var ordered = 0
  var last = 0
  for i in 0..<COUNT
    const p = q.peek_priority()
      if none use -1
    if p >= last
      ordered = ordered + 1
    last = p
    const item = edit q.pop()
    if item != None
      popped = popped + 1
  @console.print(\"{popped} {ordered} {q.len}\")
";

#[test]
fn priority_queue_pushes_and_pops_scale_as_n_log_n_in_cpu_time() {
    // `spec/28 §8`: `push` and `pop` make O(log n) comparisons, `peek_priority` is O(1), and an
    // update of a queue nothing else holds is in place. Ten times the items take under 40 times
    // as long: n log n gives about 12, a copy per update about 100, and the bound of 40x is
    // between them, as for sets.
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("runtime")
        .join("priority_scale");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("ita.una"),
        "ita priority_scale\n  version 0.1.0\n  edition 0.5\n\nrun\n  entry scale\n  grants\n    console\n",
    )
    .unwrap();
    std::fs::copy(
        repository().join("tests/conformance/names/collections_exports_in_0_5/ita.lock"),
        root.join("ita.lock"),
    )
    .unwrap();
    let dir = root.to_str().unwrap();
    // Writes the program of `n` items (untimed), then times its run.
    let measure = |n: usize| {
        std::fs::write(
            root.join("src/scale.una"),
            PRIORITY_SCALE.replace("COUNT", &n.to_string()),
        )
        .unwrap();
        let (output, seconds) = cpu::cpu_output(&una_command(
            Path::new("."),
            &["run", dir, "--grant", "console", "--max-memory", "1024"],
        ));
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        assert_eq!(stdout(&output), format!("{n} {n} 0\n"));
        seconds
    };
    timing::assert_growth(
        "items",
        &[100_000, 1_000_000],
        0.0,
        timing::FLOOR,
        40.0,
        measure,
    );
}

/// A `BitSet` of `COUNT` bits filled by `add` in a scattered order, each looked up,
/// intersected with a copy missing one member, then emptied by `remove` (`spec/28 §9`).
const BIT_SET_SCALE: &str = "module scale

import std.collections (empty_bit_set)

pub main() -> Unit
  uses @console: Console[print]
=
  var b = empty_bit_set(size: COUNT)
  for i in 0..<COUNT
    const j = (i * 7919) mod COUNT
    if 0 <= j and j < b.size
      edit b.add(item: j)
  var found = 0
  for i in 0..<COUNT
    if b.has(item: i)
      found = found + 1
  const common = b.intersection(other: b.without(item: 0))
  for i in 0..<COUNT
    edit b.remove(item: i)
  @console.print(\"{found} {common.len} {b.len}\")
";

#[test]
fn bit_set_updates_and_lookups_scale_linearly_in_cpu_time() {
    // `spec/28 §9`: `add`, `has` and `remove` are O(1), `intersection` is O(size / 64), and an
    // update of a bit set nothing else holds is in place. Ten times the bits take under 30 times
    // as long: linear time gives about 10, a copy of the words per update about 100. A miss is
    // measured twice more before it fails (#341).
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("runtime")
        .join("bit_set_scale");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("ita.una"),
        "ita bit_set_scale\n  version 0.1.0\n  edition 0.5\n\nrun\n  entry scale\n  grants\n    console\n",
    )
    .unwrap();
    std::fs::copy(
        repository().join("tests/conformance/runtime/bit_set_run/ita.lock"),
        root.join("ita.lock"),
    )
    .unwrap();
    let dir = root.to_str().unwrap();
    // Writes the program of `n` bits (untimed), then times its run.
    let measure = |n: usize| {
        std::fs::write(
            root.join("src/scale.una"),
            BIT_SET_SCALE.replace("COUNT", &n.to_string()),
        )
        .unwrap();
        let (output, seconds) = cpu::cpu_output(&una_command(
            Path::new("."),
            &["run", dir, "--grant", "console", "--max-memory", "1024"],
        ));
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        assert_eq!(stdout(&output), format!("{n} {} 0\n", n - 1));
        seconds
    };
    timing::assert_growth(
        "bits",
        &[100_000, 1_000_000],
        0.0,
        timing::FLOOR,
        30.0,
        measure,
    );
}

/// A program that appends `steps` items to a list in a loop, through `shape`: an `edit`
/// argument one, two or three calls deep, or a binding rebound from a call's result (#262).
fn appends_through_calls(shape: &str, steps: usize) -> String {
    let call = match shape {
        "edit1" => "push_one(edit a, i: i)",
        "edit2" => "push_two(edit a, i: i)",
        "edit3" => "push_three(edit a, i: i)",
        _ => "a = pushed(a, i: i)",
    };
    format!(
        "module grow\n\npush_one(edit acc: List[Int], i: Int) -> Unit\n=\n  acc = acc.append(item: i)\n\npush_two(edit acc: List[Int], i: Int) -> Unit\n=\n  push_one(edit acc, i: i)\n\npush_three(edit acc: List[Int], i: Int) -> Unit\n=\n  push_two(edit acc, i: i)\n\npushed(acc: List[Int], i: Int) -> List[Int] = acc.append(item: i)\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit a: List[Int] = []\n  for i in 0..<{steps}\n    {call}\n  @console.print(\"{{a.len}}\")\n"
    )
}

#[test]
fn a_list_appended_through_calls_grows_in_linear_cpu_time() {
    // ADR 0072 D1 and D3 (#262): an `edit` argument, or a binding at its last use rebound from
    // the result, moves into the callee, which appends in place through any number of call
    // levels. Each tenfold step stays under 30x: about 4x to 10x with the run's fixed cost,
    // against about 100x when every call copies the list. A miss is measured twice more before
    // it fails (#341).
    let root = package("edit-append", "\n[run]\nentry = \"grow\"\n", &[]);
    let manifest = std::fs::read_to_string(root.join("una.toml")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let dir = root.to_str().unwrap();
    let runs: &[(&str, &[usize])] = &[
        ("edit1", &[10_000, 100_000]),
        ("edit2", &[10_000, 100_000]),
        ("edit3", &[10_000, 100_000, 1_000_000]),
        ("rebound", &[10_000, 100_000]),
    ];
    for &(shape, sizes) in runs {
        // Writes the program of `steps` appends (untimed), then times its run.
        let measure = |steps: usize| {
            std::fs::write(
                root.join("src/grow.una"),
                appends_through_calls(shape, steps),
            )
            .unwrap();
            let (output, seconds) = cpu::cpu_output(&una_command(
                Path::new("."),
                &["run", dir, "--grant", "console"],
            ));
            assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
            assert_eq!(stdout(&output), format!("{steps}\n"));
            seconds
        };
        timing::assert_growth(shape, sizes, 0.0, 0.05, 30.0, measure);
    }
}

#[test]
fn appends_in_nested_loops_and_tasks_grow_in_linear_time() {
    // #263: the `later` entry of an enclosing loop or `together` block counts the bindings of
    // its own body as rebound, so a list appended in an inner loop, or in a loop inside a task
    // of `together for`, is updated in place. Each tenfold step stays under 30x, against about
    // 100x when every append copies. A miss is measured twice more before it fails (#341).
    let root = package("nested-append", "\n[run]\nentry = \"grow\"\n", &[]);
    let manifest = std::fs::read_to_string(root.join("una.toml")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let dir = root.to_str().unwrap();
    for shape in ["nested", "task"] {
        // Writes the program of `steps` appends (untimed), then times its run.
        let measure = |steps: usize| {
            let body = if shape == "nested" {
                format!(
                    "  let edit acc: List[Int] = []\n  for i in 0..<2\n    for j in 0..<{steps}\n      acc = acc.append(item: j)\n  @console.print(\"{{acc.len}}\")\n"
                )
            } else {
                format!(
                    "  let sizes = together for r in [0, 1]\n    let edit acc: List[Int] = []\n    for j in 0..<{steps}\n      acc = acc.append(item: j)\n    acc.len\n  @console.print(\"{{sizes}}\")\n"
                )
            };
            std::fs::write(
                root.join("src/grow.una"),
                format!(
                    "module grow\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n{body}"
                ),
            )
            .unwrap();
            let (output, seconds) = timing::timed(|| una(&["run", dir, "--grant", "console"]));
            assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
            let expected = if shape == "nested" {
                format!("{}\n", 2 * steps)
            } else {
                format!("[{steps}, {steps}]\n")
            };
            assert_eq!(stdout(&output), expected);
            seconds
        };
        timing::assert_growth(shape, &[10_000, 100_000], 0.0, 0.05, 30.0, measure);
    }
}

/// A program that builds a JSON array of `steps` items, writes it with `to_json`, parses the
/// text back and checks the round trip (#238).
fn json_round_trip(steps: usize) -> String {
    format!(
        "module grow\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit items: List[Json] = []\n  for i in 0..<{steps}\n    items = items.append(item: Number(i))\n  for i in 0..<{steps}\n    items = items.append(item: Str(\"item {{i}}\"))\n  let text = Array(items).to_json()\n  let back = text.parse_json()\n    if error use Null\n  let again = back.to_json()\n  @console.print(\"{{text.len}} {{again == text}}\")\n"
    )
}

#[test]
fn json_parses_and_writes_in_linear_time() {
    // ADR 0073 J6 (#238): writing is O(n) and parsing O(n) for canonical input. Each tenfold
    // step stays under 30x, against about 100x for quadratic work. A miss is measured twice
    // more before it fails (#341).
    let root = package("json-scaling", "\n[run]\nentry = \"grow\"\n", &[]);
    let manifest = std::fs::read_to_string(root.join("una.toml")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let dir = root.to_str().unwrap();
    // Writes the program of `steps` items (untimed), then times its run.
    let measure = |steps: usize| {
        std::fs::write(root.join("src/grow.una"), json_round_trip(steps)).unwrap();
        let (output, seconds) = timing::timed(|| una(&["run", dir, "--grant", "console"]));
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        assert!(stdout(&output).ends_with(" true\n"), "{}", stdout(&output));
        seconds
    };
    timing::assert_growth("items", &[10_000, 100_000], 0.0, 0.05, 30.0, measure);
}

/// A text of `COUNT` copies of `ab`, searched, replaced and split.
const TEXT_SCALE: &str = "module scale\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let edit parts: List[Text] = []\n  for i in 0..<COUNT\n    parts = parts.append(item: \"ab\")\n  let empty = \"\"\n  let t = parts.join(separator: empty)\n  let c = \"c\"\n  let a = \"a\"\n  let b = \"b\"\n  let missing = t.find(text: c)\n  let swapped = t.replace(pattern: a, by: c)\n  let pieces = t.split(on: b)\n  @console.print(\"{t.len} {missing} {swapped.len} {pieces.len}\")\n";

#[test]
fn text_search_replace_and_split_take_linear_cpu_time() {
    // Each run is measured by the processor time of `una` (`cpu::cpu_output`, #1662), which
    // other tests running beside it do not change, so the test needs no exclusive slot.
    // `spec/22 §2` rule 8 and the performance rule: `find`, `replace` and `split` are linear
    // in the text, the search included. Ten times the text takes under 30 times as long, about
    // 10x when linear against about 100x when quadratic. A miss is measured twice more before
    // it fails (#341).
    let root = package("text_scale", "\n[run]\nentry = \"scale\"\n", &[]);
    let manifest = std::fs::read_to_string(root.join("una.toml")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let dir = root.to_str().unwrap();
    // Writes the program of `count` pieces (untimed), then times its run.
    let measure = |count: usize| {
        std::fs::write(
            root.join("src/scale.una"),
            TEXT_SCALE.replace("COUNT", &count.to_string()),
        )
        .unwrap();
        let (output, seconds) = cpu::cpu_output(&una_command(
            Path::new("."),
            &["run", dir, "--grant", "console", "--max-memory", "1024"],
        ));
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let (len, pieces) = (2 * count, count + 1);
        assert_eq!(stdout(&output), format!("{len} None {len} {pieces}\n"));
        seconds
    };
    timing::assert_growth(
        "text",
        &[100_000, 1_000_000],
        0.0,
        timing::FLOOR,
        30.0,
        measure,
    );
}

/// A non-ASCII text of `COUNT` scalar values, built by appends, then each one sliced.
const SLICE_SCALE: &str = "module scale

pub main() -> Unit
  uses @console: Console[print]
=
  let edit t = \"\"
  for i in 0..<COUNT
    t = \"{t}é\"
  let edit total = 0
  for i in 0..<COUNT
    if i + 1 <= t.len
      total = total + t.slice(start: i, end: i + 1).len + t.len
  @console.print(\"{t.len} {total}\")
";

#[test]
fn text_len_and_slice_take_constant_cpu_time_on_any_text() {
    // `spec/22 §6` rule 1 and the performance rule: `.len` is O(1), and a position O(1) once
    // the text's index is built, which appends extend; so a slice at each of n positions of a
    // non-ASCII text takes linear time. Ten times the text takes under 30 times as long, about
    // 10x when linear against about 100x when a position costs O(n). A miss is measured twice
    // more before it fails (#341).
    let root = package("slice_scale", "\n[run]\nentry = \"scale\"\n", &[]);
    let manifest = std::fs::read_to_string(root.join("una.toml")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let dir = root.to_str().unwrap();
    // Writes the program of `count` scalar values (untimed), then times its run.
    let measure = |count: usize| {
        std::fs::write(
            root.join("src/scale.una"),
            SLICE_SCALE.replace("COUNT", &count.to_string()),
        )
        .unwrap();
        let (output, seconds) = cpu::cpu_output(&una_command(
            Path::new("."),
            &["run", dir, "--grant", "console", "--max-memory", "1024"],
        ));
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        // Each of the `count` slices adds 1 and the length `count`.
        let total = count * (count + 1);
        assert_eq!(stdout(&output), format!("{count} {total}\n"));
        seconds
    };
    timing::assert_growth(
        "text",
        &[100_000, 1_000_000],
        0.0,
        timing::FLOOR,
        30.0,
        measure,
    );
}

/// A program that appends `steps` items to a list through a function value, by `shape`: a
/// named function passed as a value (`named`), a lambda (`lambda`), two layers of lambdas over
/// a function value (`layers`), or `fold` over a list (`fold`) or into a text (`text`).
fn appends_through_function_values(shape: &str, steps: usize) -> String {
    let body = match shape {
        "named" => format!("  let a = run_with(r1, n: {steps})\n  @console.print(\"{{a.len}}\")\n"),
        "lambda" => format!(
            "  let a = run_with(fn(acc, i) -> acc.append(item: i), n: {steps})\n  @console.print(\"{{a.len}}\")\n"
        ),
        "layers" => {
            format!("  let a = twice_layered(r1, n: {steps})\n  @console.print(\"{{a.len}}\")\n")
        }
        "fold" => format!(
            "  let a = numbers({steps}).fold(initial: [], step: fn(acc, x) -> acc.append(item: x))\n  @console.print(\"{{a.len}}\")\n"
        ),
        _ => format!(
            "  let t = numbers({steps}).fold(initial: \"\", step: fn(line, x) -> \"{{line}}x\")\n  @console.print(\"{{t.len}}\")\n"
        ),
    };
    format!(
        "module grow\n\nr1(acc: List[Int], i: Int) -> List[Int] = acc.append(item: i)\n\nrun_with(f: fn(List[Int], Int) -> List[Int], n: Int) -> List[Int]\n=\n  let edit acc: List[Int] = []\n  for i in 0..<n\n    acc = f(acc, i)\n  acc\n\nlayered(f: fn(List[Int], Int) -> List[Int], n: Int) -> List[Int] = run_with(fn(acc, i) -> f(acc, i), n: n)\n\ntwice_layered(f: fn(List[Int], Int) -> List[Int], n: Int) -> List[Int] = layered(fn(acc, i) -> f(acc, i), n: n)\n\nnumbers(n: Int) -> List[Int]\n=\n  let edit xs: List[Int] = []\n  for i in 0..<n\n    xs = xs.append(item: i)\n  xs\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n{body}"
    )
}

/// Function values that a list goes through twice, or that capture it, never change it.
const ALIASED: &str = "module aliased\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let base = [1, 2]\n  let f = fn(acc: List[Int], i: Int) -> acc.append(item: i)\n  let a = f(base, 3)\n  let b = f(base, 4)\n  let g = fn(i: Int) -> base.append(item: i)\n  let c = g(5)\n  let d = g(6)\n  let folded = [7, 8].fold(initial: base, step: fn(acc, x) -> acc.append(item: x))\n  @console.print(\"{base} {a} {b} {c} {d} {folded} {base}\")\n";

/// What [`ALIASED`] prints.
const ALIASED_OUTPUT: &str = "[1, 2] [1, 2, 3] [1, 2, 4] [1, 2, 5] [1, 2, 6] [1, 2, 7, 8] [1, 2]\n";

#[test]
fn a_list_built_through_function_values_grows_in_linear_cpu_time() {
    // Each run is measured by the processor time of `una` (`cpu::cpu_output`, #1662), which
    // other tests running beside it do not change, so the test needs no exclusive slot.
    // #312: in the interpreter too, a binding at its last use moves into a call through
    // a function value, so appends through function values and `fold` stay in place. Each
    // tenfold step stays under 30x, against about 100x when every call copies. A miss is
    // measured twice more before it fails (#341).
    let root = package("value-append", "\n[run]\nentry = \"grow\"\n", &[]);
    let manifest = std::fs::read_to_string(root.join("una.toml")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let dir = root.to_str().unwrap();
    for shape in ["named", "lambda", "layers", "fold", "text"] {
        // Writes the program of `steps` appends (untimed), then times its run.
        let measure = |steps: usize| {
            std::fs::write(
                root.join("src/grow.una"),
                appends_through_function_values(shape, steps),
            )
            .unwrap();
            let (output, seconds) = cpu::cpu_output(&una_command(
                Path::new("."),
                &["run", dir, "--grant", "console"],
            ));
            assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
            assert_eq!(stdout(&output), format!("{steps}\n"));
            seconds
        };
        timing::assert_growth(shape, &[10_000, 100_000], 0.0, 0.05, 30.0, measure);
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn function_values_never_change_a_list_another_name_holds() {
    // As natively.
    let root = package("value-aliased", "\n[run]\nentry = \"aliased\"\n", &[]);
    let manifest = std::fs::read_to_string(root.join("una.toml")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        manifest.replace("edition = \"0.1\"", "edition = \"0.2\""),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/aliased.una"), ALIASED).unwrap();
    let output = una(&["run", root.to_str().unwrap(), "--grant", "console"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), ALIASED_OUTPUT);
    let _ = std::fs::remove_dir_all(&root);
}

/// A package of edition 0.4 under the test's temporary directory, with `ita.una` (its
/// sections `sections`), the lock of the conformance cases of edition 0.4, and `files`.
fn ita_package(name: &str, sections: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("runtime")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("ita.una"),
        format!("ita {name}\n  version 0.1.0\n  edition 0.4\n{sections}"),
    )
    .unwrap();
    std::fs::copy(
        repository().join("tests/conformance/runtime/for_ever_on_stop/ita.lock"),
        root.join("ita.lock"),
    )
    .unwrap();
    for (path, text) in files {
        std::fs::write(root.join(path), text).unwrap();
    }
    root
}

const TICKS: &str = "module ticks

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let edit n = 0
  for ever
    @console.print(\"tick {n}\")
    n = n + 1
  on stop
    @console.print(\"stopped at {n}\")
  @console.print(\"after {n}\")
";

const LONG_STOP: &str = "module ticks

pub main() -> Unit
  diverges
=
  let edit n = 0
  for ever
    n = n + 1
  on stop
    for i in 0..<1000000000000
      n = n + 1
  unit
";

#[test]
fn a_for_ever_stops_after_its_iterations_and_runs_on_stop() {
    // ADR 0116 rules 14 and 19, `spec/26 §6` rule 1: the run's own stop arrives once three
    // iterations have started; the loop stops at the fourth start with a `stop` event, and
    // `on stop` and the code after the loop run.
    let root = ita_package(
        "ticks",
        "\nrun\n  entry ticks\n  grants\n    console\n",
        &[("src/ticks.una", TICKS)],
    );
    let output = una_in(&root, &["--json", "run", "--stop-after-iterations", "3"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let document = json(&output);
    assert_eq!(
        document["output"],
        json!(["tick 0", "tick 1", "tick 2", "stopped at 3", "after 3"])
    );
    assert_eq!(
        document["trace"][3],
        json!({"seq": 4, "outcome": {"kind": "stop"}})
    );
}

#[test]
fn the_hard_stop_after_a_deadline_is_u6007_with_the_trace_of_an_abort() {
    // ADR 0116 rules 14 and 16, `spec/26 §6` rule 6: `--deadline 1` requests the stop, the
    // `on stop` outlasts `--stop-grace 1`, and the run is cancelled at a loop iteration.
    let root = ita_package(
        "long",
        "\nrun\n  entry ticks\n",
        &[("src/ticks.una", LONG_STOP)],
    );
    let output = una_in(
        &root,
        &["--json", "run", "--deadline", "1", "--stop-grace", "1"],
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let document = json(&output);
    assert_eq!(document["status"], "aborted");
    assert_eq!(document["diagnostics"][0]["code"], "U6007");
    assert_eq!(
        document["diagnostics"][0]["message"],
        "the host stopped the run before it finished, after the grace period of 1 second"
    );
    assert_eq!(document["diagnostics"][0]["primary"]["start"]["line"], 10);
    assert_eq!(
        document["trace"],
        json!([{"seq": 1, "outcome": {"kind": "stop"}}])
    );
}

#[test]
fn deadline_and_stop_grace_take_whole_seconds_in_range() {
    // ADR 0116 Diagnostics: usage errors, exit code 2, before anything runs.
    for (flag, value) in [
        ("--deadline", "0"),
        ("--deadline", "31536001"),
        ("--stop-grace", "0"),
        ("--stop-grace", "86401"),
        ("--stop-grace", "ten"),
    ] {
        let output = una(&["run", "examples/hello", flag, value]);
        assert_eq!(output.status.code(), Some(2), "{flag} {value}");
        assert_eq!(stdout(&output), "");
    }
}
