//! The Agent ABI of `spec/13`: the build report, `una trace`, one JSON document per command,
//! and determinism (`compiler/testing.md §4`). Tier 1 needs Z3 at the version of
//! `verification.lock` on the path.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pretty_assertions::assert_eq;
use serde_json::{Value, json};

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn una_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
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

#[test]
fn the_report_of_the_events_example() {
    // M9 acceptance criterion: a snapshot of `una report --json` for the events example, with
    // a warm cache so that every result comes from it (ADR 0041).
    let _ = una(&["--json", "report", "examples/events"]);
    let output = una(&["--json", "report", "examples/events"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stderr(&output), "");
    // The `native` section names the host target (ADR 0055) and the kernel variants compiled
    // for it (ADR 0120 rule 56), which differ between runners.
    let target = una_codegen::host_target();
    let variants = serde_json::to_string(una_core::dense::variants::variant_names()).unwrap();
    let text = stdout(&output)
        .replace(&format!("\"target\":\"{target}\""), "\"target\":\"<host>\"")
        .replace(
            &format!("\"variants\":{variants}"),
            "\"variants\":\"<host>\"",
        );
    insta::assert_snapshot!("report_events_json", text);
    let report = json(&output);
    assert_eq!(report["schema"], "una.report/2");
    assert_eq!(
        report["package"],
        json!({"name": "events-example", "version": "0.1.0"})
    );
    assert_eq!(
        report["obligations"],
        json!({"proven": 11, "refuted": 0, "unknown": 0, "cached": 11})
    );
    assert_eq!(
        report["proven"],
        json!({"semantic": 11, "security": "by typing", "authority": "by typing"})
    );
    assert_eq!(
        report["information"]["policy_calls"][0],
        json!({
            "policy": "events.keys.attendee_key",
            "purpose": "attendee lookup and audit correlation",
            "owner": "privacy",
            "site": {"file": "src/events/checkin.una", "start": {"line": 30, "col": 13}}
        })
    );
    assert_eq!(
        report["escapes"],
        json!({"checked": [{"module": "events.types", "count": 1, "budget": 1}]})
    );
    assert_eq!(
        report["tcb"]["trusted_modules"],
        json!([{"module": "events.keys", "lines": 17}])
    );
    // ADR 0041: the proof cache is part of the Trusted zone.
    assert_eq!(report["tcb"]["proof_cache"], ".una/cache/");
    assert_eq!(
        report["requirements"],
        json!([{"id": "EVENTS_3", "status": "active", "derived": ["events.seating.availability"]}])
    );
    assert_eq!(
        report["coverage"],
        json!({"pub_functions": 5, "with_ensures": 1, "with_derived_from": 1, "trivial_ensures": 0})
    );
    let keys: Vec<&str> = report
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    // ADR 0067 added `generic_results`, after `native`; ADR 0081 `handler_authority`, after
    // `authority`; ADR 0122 `dependencies`, after `handler_authority`, always present.
    assert_eq!(keys.len(), 17);
    assert_eq!(report["generic_results"], json!([]));
    assert_eq!(report["dependencies"], json!([]));
    assert_eq!(report["tcb"]["registries"], json!([]));
    assert_eq!(report["tcb"]["dependency_trust"], json!([]));
    assert_eq!(report["handler_authority"], json!([]));
    assert_eq!(report["native"]["target"], target.as_str());
    // `clamp_seats` returns `Seats`, `Int in 0..5000`, a machine width (FW-1); its parameter
    // `raw` is an unbounded `Int`.
    assert_eq!(
        report["native"]["arbitrary_precision"],
        json!(["events.seating.clamp_seats.raw"])
    );
    assert_eq!(
        report["native"]["kernels"]["variants"],
        json!(una_core::dense::variants::variant_names())
    );
    assert_eq!(
        report["native"]["kernels"]["fma_allowlist"],
        json!(una_core::dense::variants::ALLOWLIST)
    );
    let output = una(&["report", "examples/events"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(
        stdout(&output).contains(
            "\ncoverage: 5 pub functions, 1 with ensures, 1 with derived from, 0 trivial ensures\n"
        ),
        "{}",
        stdout(&output)
    );
    assert!(
        stdout(&output).contains("\nobligations: 11 proven, 0 refuted, 0 unknown; 11 cached\n"),
        "{}",
        stdout(&output)
    );
    assert!(
        stdout(&output).contains("\n  proof cache: .una/cache/\n"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn the_out_of_model_list_is_the_one_of_the_spec() {
    // R-041: `spec/07 §1`, `spec/13 §6` (the JSON example and the prose) and the report give
    // one list, in one order; ADR 0038 D9 rule 4 adds path races to it, ADR 0048 hard
    // links, and ADR 0065 the order of effects across `together` tasks.
    let output = una(&["--json", "report", "examples/events"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let channels: Vec<String> = json(&output)["out_of_model"]
        .as_array()
        .unwrap()
        .iter()
        .map(|channel| channel.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        channels,
        [
            "timing",
            "termination and divergence",
            "resource consumption and allocation",
            "stack exhaustion",
            "external systems beyond declared signatures",
            "races between the check and the use of a file path",
            "hard links",
            "order of effects across concurrent tasks",
            "the time of a host stop",
            "interference of the host's load with the timing of an expression",
            "uncatchable termination",
        ]
    );
    let (last, first) = channels.split_last().unwrap();
    let prose = format!("{}, and {last}", first.join(", "));
    let spec07 =
        std::fs::read_to_string(repository().join("docs/spec/07-information-flow.md")).unwrap();
    assert!(spec07.contains(&prose), "spec/07 §1 lacks: {prose}");
    let spec13 =
        std::fs::read_to_string(repository().join("docs/spec/13-diagnostics-and-abi.md")).unwrap();
    assert!(spec13.contains(&prose), "spec/13 §6 lacks: {prose}");
    let example = format!(
        "\"out_of_model\": {}",
        serde_json::to_string(&channels).unwrap()
    );
    assert!(
        spec13.contains(&example.replace("\",\"", "\", \"")),
        "spec/13 §6 example lacks: {example}"
    );
    let output = una(&["report", "examples/events"]);
    assert!(
        stdout(&output).contains(&format!("\nout of model: {}\n", channels.join(", "))),
        "{}",
        stdout(&output)
    );
}

#[test]
fn the_obligations_of_the_events_example() {
    // M9 acceptance criterion: a snapshot of `una obligations --json`, with a warm cache.
    let _ = una(&["--json", "obligations", "examples/events"]);
    let output = una(&["--json", "obligations", "examples/events"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    insta::assert_snapshot!("obligations_events_json", stdout(&output));
}

#[test]
fn trivial_ensures_count_as_missing_contracts() {
    // `spec/13 §6`, an `ensures` clause valid without any hypothesis.
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("abi-trivial");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        "[package]\nname = \"trivial\"\nversion = \"1.2.3\"\nedition = \"0.1\"\n",
    )
    .unwrap();
    std::fs::copy(
        repository().join("verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(
        root.join("src/numbers.una"),
        "module numbers

pub same(x: Int) -> Int
  ensures result == result
=
  x

pub twice(x: Int) -> Int
  ensures result == x + x
=
  x * 2

pub bounded(x: Int) -> Int
  ensures result >= 0 or result < 0
  ensures result == x
=
  x
",
    )
    .unwrap();
    let output = una(&["--json", "report", root.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let report = json(&output);
    assert_eq!(
        report["package"],
        json!({"name": "trivial", "version": "1.2.3"})
    );
    assert_eq!(
        report["coverage"],
        json!({"pub_functions": 3, "with_ensures": 2, "with_derived_from": 0, "trivial_ensures": 2})
    );
}

#[test]
fn a_single_file_is_named_after_its_module() {
    // A single file has no `una.toml`; its lock is the default one.
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("abi-single");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("tally.una");
    std::fs::write(
        &file,
        "module tally

pub next(x: Int) -> Int
  ensures result > x
=
  x + 1
",
    )
    .unwrap();
    let output = una(&["--json", "report", file.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let report = json(&output);
    assert_eq!(
        report["package"],
        json!({"name": "tally", "version": "0.0.0"})
    );
    assert_eq!(
        report["obligations"],
        json!({"proven": 1, "refuted": 0, "unknown": 0, "cached": 0})
    );
    assert_eq!(
        report["coverage"],
        json!({"pub_functions": 1, "with_ensures": 1, "with_derived_from": 0, "trivial_ensures": 0})
    );
    assert_eq!(report["lock"]["hash"].as_str().unwrap().len(), 64);
}

#[test]
fn report_and_trace_exit_with_1_on_an_unproven_obligation() {
    // `spec/13 §5` exit codes, the document still prints.
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("abi-refuted");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        "[package]\nname = \"refuted\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n",
    )
    .unwrap();
    std::fs::copy(
        repository().join("verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(
        root.join("src/numbers.una"),
        "module numbers

requirement NUM_1
  text \"Doubling never shrinks a number\"
  source human
  owner math
  status active

pub double(x: Int) -> Int
  ensures result >= x
  derived from NUM_1
=
  x * 2
",
    )
    .unwrap();
    let path = root.to_str().unwrap();
    // The cache counts below depend on the proof-cache key, so the test has a home of its own:
    // no other test makes or reads that key while it runs (#282).
    let home = root.join("home");
    let una = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_una"))
            .env("HOME", &home)
            .env_remove("UNA_PROOF_KEY")
            .args(args)
            .current_dir(repository())
            .output()
            .unwrap()
    };
    let output = una(&["--json", "report", path]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert_eq!(stderr(&output), "");
    assert_eq!(
        json(&output)["obligations"],
        json!({"proven": 0, "refuted": 1, "unknown": 0, "cached": 0})
    );
    let output = una(&["report", path]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("\nobligations: 0 proven, 1 refuted, 0 unknown; 1 cached\n"));
    assert!(stderr(&output).contains("U5001"), "{}", stderr(&output));
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", &home)
        .env_remove("UNA_PROOF_KEY")
        .args(["--json", "trace", "NUM_1"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let document = json(&output);
    assert_eq!(
        document["functions"][0]["obligations"][0]["status"],
        "refuted"
    );
    // A package that tier 0 rejects gets the diagnostic list of `una check`.
    std::fs::write(
        root.join("src/numbers.una"),
        "module numbers\n\npub double(x: Int) -> Int = \"two\"\n",
    )
    .unwrap();
    let output = una(&["--json", "report", path]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json(&output)["schema"], "una.diagnostics/1");
}

#[test]
fn trace_lists_the_functions_derived_from_a_requirement() {
    // `spec/08 §1`,
    let events = Path::new("examples/events");
    let output = una_in(events, &["trace", "EVENTS_3"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.starts_with("requirement EVENTS_3 (active)\n  text \"An event with no remaining seats is always shown as sold out\"\n  source policy \"Box Office Handbook 2.1\"\n  owner box_office\nevents.seating.availability\n  ensures e.remaining < 1 implies result is SoldOut\n"), "{text}");
    assert!(text.contains(" postcondition e.remaining < 1 implies result is SoldOut\n"));
    let output = una_in(events, &["--json", "trace", "EVENTS_3"]);
    assert_eq!(output.status.code(), Some(0));
    insta::assert_snapshot!("trace_events_json", stdout(&output));
    let document = json(&output);
    assert_eq!(document["schema"], "una.trace/1");
    assert_eq!(
        document["functions"][0]["function"],
        "events.seating.availability"
    );
    assert!(
        document["functions"][0]["obligations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["status"] == "proven")
    );
    let output = una_in(events, &["trace", "NOPE_1"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        stderr(&output),
        "error: the package has no requirement `NOPE_1`\n"
    );
}

#[test]
fn every_command_prints_one_json_document_with_its_schema() {
    // `spec/13 §5`, `--json` for every command, one document on stdout and
    // nothing on stderr.
    let events = Path::new("examples/events");
    let cases: Vec<(&Path, Vec<&str>, &str)> = vec![
        (
            Path::new("."),
            vec!["--json", "check", "examples/events"],
            "una.diagnostics/1",
        ),
        (
            Path::new("."),
            vec!["--json", "fmt", "--check", "examples/events"],
            "una.diagnostics/1",
        ),
        (
            Path::new("."),
            vec!["--json", "fix", "--dry-run", "examples/events"],
            "una.fix/1",
        ),
        (
            Path::new("."),
            vec!["--json", "run", "examples/hello"],
            "una.run/1",
        ),
        (
            Path::new("."),
            vec!["--json", "test", "examples/events"],
            "una.test/1",
        ),
        (
            Path::new("."),
            vec!["--json", "obligations", "examples/events"],
            "una.obligations/1",
        ),
        (
            events,
            vec!["--json", "contract", "events.checkin.check_in"],
            "una.contract/1",
        ),
        (events, vec!["--json", "trace", "EVENTS_3"], "una.trace/1"),
        (
            Path::new("."),
            vec!["--json", "report", "examples/events"],
            "una.report/2",
        ),
        (
            Path::new("."),
            vec!["--json", "explain", "U5001"],
            "una.explain/1",
        ),
        (Path::new("."), vec!["--json", "--version"], "una.version/1"),
    ];
    for (dir, args, schema) in cases {
        let output = una_in(dir, &args);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{args:?}: {}",
            stderr(&output)
        );
        assert_eq!(stderr(&output), "", "{args:?}");
        let text = stdout(&output);
        assert_eq!(text.matches('\n').count(), 1, "{args:?}: one line");
        let document: Value = serde_json::from_str(text).unwrap();
        assert_eq!(document["schema"], schema, "{args:?}");
        assert!(
            text.starts_with("{\"schema\":"),
            "{args:?}: schema comes first"
        );
    }
}

#[test]
fn json_outputs_are_byte_identical_across_runs_and_thread_counts() {
    // `compiler/testing.md §4`, for every command with a document, on the examples; the
    // obligation listing runs once first, so both runs read a warm cache.
    let events = Path::new("examples/events");
    let _ = una(&["--json", "obligations", "examples/events"]);
    let commands: Vec<(&Path, Vec<&str>)> = vec![
        (Path::new("."), vec!["--json", "report", "examples/events"]),
        (
            Path::new("."),
            vec!["--json", "obligations", "examples/events"],
        ),
        (Path::new("."), vec!["--json", "test", "examples/events"]),
        (Path::new("."), vec!["--json", "run", "examples/hello"]),
        (
            events,
            vec!["--json", "contract", "events.checkin.check_in"],
        ),
        (events, vec!["--json", "trace", "EVENTS_3"]),
    ];
    for (dir, args) in commands {
        let first = una_in(dir, &args);
        let second = una_in(dir, &args);
        let single = Command::new(env!("CARGO_BIN_EXE_una"))
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .env_remove("UNA_PROOF_KEY")
            .args(&args)
            .env("RAYON_NUM_THREADS", "1")
            .current_dir(repository().join(dir))
            .output()
            .unwrap();
        assert_eq!(first.stdout, second.stdout, "{args:?}");
        assert_eq!(first.stdout, single.stdout, "{args:?}");
    }
}
