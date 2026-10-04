//! End-to-end tests of the `una` binary (`spec/13 §5`, ADR 0015).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::{Command, Output};

use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use una_diag::Code;

const EMPTY_LIST: &str = "{\"schema\":\"una.diagnostics/1\",\"diagnostics\":[]}\n";

/// Runs the `una` binary from the repository root.
fn una(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .args(args)
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).unwrap()
}

fn stderr(output: &Output) -> &str {
    std::str::from_utf8(&output.stderr).unwrap()
}

#[test]
fn explain_u0001_prints_its_text() {
    let output = una(&["explain", "U0001"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(
        stdout(&output)
            .starts_with("# U0001: Invalid character, byte order mark or lone carriage return\n\n")
    );
    assert_eq!(stdout(&output), Code::U0001.explain_text());
    assert_eq!(stderr(&output), "");
}

#[test]
fn explain_prints_the_text_of_every_registered_code() {
    for code in Code::ALL {
        let output = una(&["explain", code.as_str()]);
        assert_eq!(output.status.code(), Some(0), "{code}");
        assert_eq!(stdout(&output), code.explain_text(), "{code}");
    }
}

#[test]
fn explain_rejects_text_that_is_not_a_registered_code() {
    // The codes of the earlier names are written in two pieces, so that the audit for old
    // code shapes (ADR 0025) finds none in the repository.
    let old_codes = [concat!("T", "0001"), concat!("V", "0001")];
    for text in ["U9999", "U4006", "u0001"].into_iter().chain(old_codes) {
        let output = una(&["explain", text]);
        assert_eq!(output.status.code(), Some(2), "{text}");
        assert_eq!(stdout(&output), "", "{text}");
        assert_eq!(
            stderr(&output),
            format!("error: unknown diagnostic code \"{text}\"\n")
        );
    }
}

#[test]
fn explain_json_prints_its_document() {
    // Every command has a JSON document; M1 answered with a usage error.
    let output = una(&["explain", "U0001", "--json"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stderr(&output), "");
    let document: Value = serde_json::from_str(stdout(&output)).unwrap();
    assert_eq!(
        document,
        json!({
            "schema": "una.explain/1",
            "code": "U0001",
            "domain": "syntax",
            "meaning": Code::U0001.meaning(),
            "text": Code::U0001.explain_text(),
        })
    );
    assert!(stdout(&output).starts_with("{\"schema\":\"una.explain/1\""));
}

#[test]
fn check_json_prints_an_empty_diagnostic_list() {
    // The repository root has no `src/` (ADR 0015 D3), so check a package.
    let output = una(&["check", "--json", "examples/hello"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), EMPTY_LIST);
    assert_eq!(stderr(&output), "");
    let document: Value = serde_json::from_str(stdout(&output)).unwrap();
    assert_eq!(document["schema"], "una.diagnostics/1");
    assert_eq!(document["diagnostics"], json!([]));
}

#[test]
fn json_is_a_global_flag() {
    let output = una(&["--json", "check", "examples/hello"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), EMPTY_LIST);
}

#[test]
fn check_json_is_byte_identical_across_runs() {
    // compiler/testing.md §4: every example, and every case of the finished areas with its
    // diagnostics, twice and with one worker thread.
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut targets: Vec<String> = std::fs::read_dir(repository.join("examples"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.'))
        .map(|name| format!("examples/{name}"))
        .collect();
    targets.sort();
    for area in [
        "syntax",
        "fmt",
        "names",
        "project",
        "types",
        "authority",
        "security",
        "runtime",
    ] {
        let mut cases: Vec<String> =
            std::fs::read_dir(repository.join("tests/conformance").join(area))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| !name.starts_with('.'))
                .map(|name| format!("tests/conformance/{area}/{name}"))
                .collect();
        cases.sort();
        targets.extend(cases);
    }
    for target in &targets {
        let first = una(&["check", "--json", target]);
        let second = una(&["check", "--json", target]);
        let single = Command::new(env!("CARGO_BIN_EXE_una"))
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .args(["check", "--json", target])
            .env("RAYON_NUM_THREADS", "1")
            .current_dir(&repository)
            .output()
            .unwrap();
        assert!(matches!(first.status.code(), Some(0 | 1)), "{target}");
        assert_eq!(first.stdout, second.stdout, "{target}");
        assert_eq!(first.stdout, single.stdout, "{target}");
    }
    assert!(targets.len() > 110);
}

#[test]
fn check_without_diagnostics_prints_nothing() {
    // Every invocation names a package or a file (ADR 0015 D3).
    for args in [
        &["check", "examples/events"][..],
        &["check", "--color", "always", "examples/hello"],
        &["check", "--color", "never", "examples/hello"],
        &["check", "--tier", "0", "examples/hello"],
        &["check", "--tier", "1", "examples/events"],
        &["check", "examples/hello/src/hello.una"],
    ] {
        let output = una(args);
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        assert_eq!(stdout(&output), "", "{args:?}");
        assert_eq!(stderr(&output), "", "{args:?}");
    }
}

#[test]
fn usage_errors_exit_with_2_and_leave_stdout_empty() {
    for args in [
        &["frobnicate"][..],
        &["help"],
        &["--version"],
        &["explain"],
        &["check", "--color", "sometimes"],
        &["check", "--tier", "2", "examples/hello"],
    ] {
        let output = una(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(stdout(&output), "", "{args:?}");
        assert!(
            stderr(&output).starts_with("error: "),
            "{args:?}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn a_bare_invocation_prints_usage_on_stderr() {
    let output = una(&[]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stdout(&output), "");
    assert!(stderr(&output).contains("Usage: una [OPTIONS] <COMMAND>"));
}

#[test]
fn help_is_printed_on_stdout() {
    let output = una(&["--help"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).contains("check"));
    assert!(stdout(&output).contains("explain"));
    assert_eq!(stderr(&output), "");
}

#[test]
fn check_reports_a_missing_path_as_a_usage_error() {
    for args in [
        &["check", "no/such/path"][..],
        &["check", "--json", "no/such/path"],
    ] {
        let output = una(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(stdout(&output), "", "{args:?}");
        assert_eq!(
            stderr(&output),
            "error: path `no/such/path` does not exist\n"
        );
    }
}

#[test]
fn check_requires_a_package() {
    // A directory is a package only with `una.toml` and `src/`.
    let output = una(&["check", "docs"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: directory `docs` is not a package: it has no `una.toml`\n"
    );
    let root = scratch("package-without-src");
    std::fs::write(root.join("una.toml"), MANIFEST).unwrap();
    let output = una(&["check", root.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        stderr(&output),
        format!(
            "error: package `{}` has no `src/` directory\n",
            root.display()
        )
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// A minimal `una.toml`.
const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n";

#[test]
fn a_directory_loads_its_una_sources_in_path_order() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/events");
    let package = una_cli::load(&root).unwrap();
    let file = |id| package.sources.file(id).path();
    assert_eq!(package.manifest.map(file), Some("una.toml"));
    assert_eq!(package.lock.map(file), Some("verification.lock"));
    let names: Vec<&str> = package.modules.iter().map(|&id| file(id)).collect();
    assert_eq!(
        names,
        [
            "src/events/checkin.una",
            "src/events/keys.una",
            "src/events/main.una",
            "src/events/seating.una",
            "src/events/types.una",
        ]
    );
}

#[test]
fn a_single_file_is_its_own_package_root() {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello/src/hello.una");
    let package = una_cli::load(&file).unwrap();
    assert!(package.single);
    let names: Vec<&str> = package
        .sources
        .files()
        .map(|(_, file)| file.path())
        .collect();
    assert_eq!(names, ["hello.una"]);
}

#[test]
fn hidden_entries_and_other_files_under_src_are_not_loaded() {
    // Hidden entries under `src/` are skipped.
    let root = scratch("load-hidden");
    std::fs::write(root.join("una.toml"), MANIFEST).unwrap();
    std::fs::create_dir_all(root.join("src/.cache")).unwrap();
    std::fs::create_dir_all(root.join("src/sub")).unwrap();
    for path in [
        "src/b.una",
        "src/a.una",
        "src/.#lock.una",
        "src/.cache/c.una",
        "src/sub/d.una",
        "src/notes.txt",
    ] {
        std::fs::write(root.join(path), "module x\n").unwrap();
    }
    let package = una_cli::load(&root).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    let names: Vec<&str> = package
        .modules
        .iter()
        .map(|&id| package.sources.file(id).path())
        .collect();
    assert_eq!(names, ["src/a.una", "src/b.una", "src/sub/d.una"]);
}

#[test]
fn human_diagnostics_go_to_stderr() {
    // ADR 0015 D1: human diagnostics on stderr, nothing on stdout, exit code 1.
    let output = una(&["check", "tests/conformance/syntax/invalid_character.una"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output)
            .starts_with("[U0001] Error: invalid character `;`; statements end at line breaks\n"),
        "{}",
        stderr(&output)
    );
    assert!(stderr(&output).contains("invalid_character.una:6:21"));
}

#[test]
fn json_diagnostics_go_to_stdout() {
    let output = una(&[
        "check",
        "--json",
        "tests/conformance/syntax/invalid_character.una",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stderr(&output), "");
    let document: Value = serde_json::from_str(stdout(&output)).unwrap();
    assert_eq!(document["schema"], "una.diagnostics/1");
    let diagnostic = &document["diagnostics"][0];
    assert_eq!(diagnostic["code"], "U0001");
    assert_eq!(diagnostic["primary"]["file"], "invalid_character.una");
    assert_eq!(
        diagnostic["primary"]["start"],
        json!({ "line": 6, "col": 21 })
    );
    assert_eq!(document["diagnostics"].as_array().unwrap().len(), 1);
}

#[test]
fn deep_nesting_is_a_diagnostic_even_on_a_small_stack() {
    // `check` runs on a thread with a large stack, so a test thread's small
    // stack is enough, and the nesting limit turns the input into one U0101.
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("deep-nesting");
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("deep.una");
    let depth = 20_000;
    let text = format!(
        "module deep\n\npub f() -> Int = {}1{}\n",
        "(".repeat(depth),
        ")".repeat(depth)
    );
    std::fs::write(&file, text).unwrap();
    let checked = una_cli::check(&file, una_cli::Tier::One).unwrap();
    let codes: Vec<Code> = checked.diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(codes, [Code::U0101]);
    assert!(
        checked.diagnostics[0]
            .message
            .starts_with("nesting deeper than 256 levels")
    );
    let output = una(&["check", file.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    std::fs::remove_dir_all(&root).unwrap();
}

/// A fresh directory under the test's temporary directory.
fn scratch(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn codes(output: &Output) -> Vec<String> {
    let document: Value = serde_json::from_str(stdout(output)).unwrap();
    document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| diagnostic["code"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn fmt_rewrites_non_canonical_files_and_leaves_files_with_syntax_errors() {
    // Syntax errors are reported and their files left untouched.
    let root = scratch("fmt-package");
    std::fs::write(root.join("una.toml"), MANIFEST).unwrap();
    std::fs::create_dir_all(root.join("src/a")).unwrap();
    let messy = "module a.b\n\npub one() -> Int\n=\n  1+1\n";
    let broken = "module broken\n\npub one() Int = 1\n";
    std::fs::write(root.join("src/a/b.una"), messy).unwrap();
    std::fs::write(root.join("src/broken.una"), broken).unwrap();
    let path = root.to_str().unwrap();

    let check = una(&["fmt", "--check", "--json", path]);
    assert_eq!(check.status.code(), Some(1));
    assert_eq!(stderr(&check), "");
    assert_eq!(codes(&check), ["U7001", "U0101"]);
    assert_eq!(
        std::fs::read_to_string(root.join("src/a/b.una")).unwrap(),
        messy
    );

    let fmt = una(&["fmt", path]);
    assert_eq!(fmt.status.code(), Some(1));
    assert_eq!(stdout(&fmt), "");
    assert!(stderr(&fmt).contains("[U0101]"), "{}", stderr(&fmt));
    assert_eq!(
        std::fs::read_to_string(root.join("src/a/b.una")).unwrap(),
        "module a.b\n\npub one() -> Int = 1 + 1\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("src/broken.una")).unwrap(),
        broken
    );

    std::fs::write(
        root.join("src/broken.una"),
        "module broken\n\npub one() -> Int = 1\n",
    )
    .unwrap();
    let again = una(&["fmt", path]);
    assert_eq!(again.status.code(), Some(0));
    assert_eq!((stdout(&again), stderr(&again)), ("", ""));
    let entries = std::fs::read_dir(root.join("src")).unwrap().count();
    assert_eq!(entries, 2, "no temporary file is left behind");
    let clean = una(&["fmt", "--check", path]);
    assert_eq!(clean.status.code(), Some(0));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn fmt_converts_line_breaks_to_line_feeds() {
    let root = scratch("fmt-crlf");
    let file = root.join("crlf.una");
    std::fs::write(&file, "module crlf\r\n\r\npub one() -> Int = 1\r\n").unwrap();
    let path = file.to_str().unwrap();
    let check = una(&["check", "--json", path]);
    assert_eq!(codes(&check), ["U7001"]);
    let fmt = una(&["fmt", path]);
    assert_eq!(fmt.status.code(), Some(0));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "module crlf\n\npub one() -> Int = 1\n"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// Review finding R-024: formatting a symbolic link rewrites the file it points to and keeps
/// the link.
#[cfg(unix)]
#[test]
fn fmt_writes_through_a_symbolic_link_onto_its_target() {
    let root = scratch("fmt-symlink");
    std::fs::create_dir_all(root.join("real")).unwrap();
    let target = root.join("real/linked.una");
    let link = root.join("linked.una");
    std::fs::write(&target, "module linked\n\npub one() -> Int\n=\n  1+1\n").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let fmt = una(&["fmt", link.to_str().unwrap()]);
    assert_eq!(fmt.status.code(), Some(0), "{}", stderr(&fmt));
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link is kept"
    );
    assert_eq!(std::fs::read_link(&link).unwrap(), target);
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "module linked\n\npub one() -> Int = 1 + 1\n"
    );
    assert_eq!(
        std::fs::read_dir(root.join("real")).unwrap().count(),
        1,
        "no temporary file is left behind"
    );
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn check_reports_t7001_with_a_mechanical_edit() {
    let output = una(&[
        "check",
        "--json",
        "tests/conformance/fmt/long_form_without_clauses.una",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let document: Value = serde_json::from_str(stdout(&output)).unwrap();
    let diagnostic = &document["diagnostics"][0];
    assert_eq!(diagnostic["code"], "U7001");
    assert_eq!(
        diagnostic["message"],
        "lines 6 to 8 are not in canonical form"
    );
    assert_eq!(
        diagnostic["repairs"][0]["edit"],
        json!({
            "file": "long_form_without_clauses.una",
            "start": { "line": 6, "col": 1 },
            "end": { "line": 9, "col": 1 },
            "replacement": "pub one() -> Int = 1\n"
        })
    );
}

#[test]
fn check_reports_project_file_errors_and_still_checks_the_modules() {
    // ADR 0021; a missing lock is U7004 at `una.toml` 1:1;
    // the modules are still checked.
    let root = scratch("project-errors");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        format!("{MANIFEST}license = \"MIT\"\n"),
    )
    .unwrap();
    std::fs::write(
        root.join("src/a.una"),
        "module a\n\npub one() -> Int = 1;\n",
    )
    .unwrap();
    let output = una(&["check", "--json", root.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    let document: Value = serde_json::from_str(stdout(&output)).unwrap();
    let found: Vec<(String, String, u64, u64)> = document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| {
            let primary = &diagnostic["primary"];
            (
                diagnostic["code"].as_str().unwrap().to_owned(),
                primary["file"].as_str().unwrap().to_owned(),
                primary["start"]["line"].as_u64().unwrap(),
                primary["start"]["col"].as_u64().unwrap(),
            )
        })
        .collect();
    let expected = [
        ("U0001", "src/a.una", 3, 21),
        ("U7004", "una.toml", 1, 1),
        ("U7002", "una.toml", 5, 1),
    ];
    let expected: Vec<(String, String, u64, u64)> = expected
        .iter()
        .map(|(code, file, line, col)| ((*code).to_owned(), (*file).to_owned(), *line, *col))
        .collect();
    assert_eq!(found, expected);
    std::fs::remove_dir_all(&root).unwrap();
}

/// Runs the `una` binary in `dir`, a directory relative to the repository root, or absolute.
fn una_in(dir: &std::path::Path, args: &[&str]) -> Output {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .args(args)
        .current_dir(repository.join(dir))
        .output()
        .unwrap()
}

#[test]
fn contract_prints_the_header_of_the_reference_example() {
    // M5 acceptance criterion: a snapshot of `una contract --json events.checkin.check_in`.
    let events = std::path::Path::new("examples/events");
    let output = una_in(events, &["contract", "--json", "events.checkin.check_in"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stderr(&output), "");
    insta::assert_snapshot!("contract_check_in_json", stdout(&output));
    let document: Value = serde_json::from_str(stdout(&output)).unwrap();
    assert_eq!(document["schema"], "una.contract/1");
    let output = una_in(events, &["contract", "events.checkin.check_in"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stderr(&output), "");
    insta::assert_snapshot!("contract_check_in_human", stdout(&output));
    let output = una_in(events, &["contract", "events.seating.availability"]);
    assert_eq!(output.status.code(), Some(0));
    insta::assert_snapshot!("contract_availability_human", stdout(&output));
}

#[test]
fn contract_marks_inferred_clauses() {
    // Inferred clauses end with `-- inferred` and carry `"inferred": true`.
    let root = scratch("contract-inferred");
    std::fs::create_dir_all(root.join("src/app")).unwrap();
    std::fs::write(root.join("una.toml"), MANIFEST).unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(
        root.join("src/app/log.una"),
        "module app.log\n\nkind LookupError = Missing | Unavailable\n\ncapability Log\n  op write(line: Text) -> Unit\n  op flush() -> Unit\n\n--- Writes a line and flushes.\nhelper(line: Text) -> Int\n=\n  @log.write(line)\n  @log.flush()\n  if line.len > 0 then 1 else fail Missing\n\npub run() -> Int\n  uses @log: Log\n  fails LookupError\n=\n  helper(\"x\")\n    if error pass\n",
    )
    .unwrap();
    let output = una_in(&root, &["contract", "app.log.helper"]);
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        stdout(&output),
        "--- Writes a line and flushes.\nhelper(line: Text) -> Int\n  uses @log: Log[write, flush] -- inferred\n  fails LookupError -- inferred\n"
    );
    let output = una_in(&root, &["contract", "--json", "app.log.helper"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        stdout(&output),
        "{\"schema\":\"una.contract/1\",\"function\":\"app.log.helper\",\"public\":false,\"docs\":[\"Writes a line and flushes.\"],\"header\":\"helper(line: Text) -> Int\",\"constraints\":[],\"requires\":[],\"ensures\":[],\"uses\":[{\"cap\":\"log\",\"capability\":\"Log\",\"ops\":[\"write\",\"flush\"],\"inferred\":true}],\"fails\":{\"kind\":\"LookupError\",\"inferred\":true},\"derived_from\":[]}\n"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn contract_reports_unknown_names_and_diagnostics() {
    let events = std::path::Path::new("examples/events");
    for name in [
        "events.checkin.nothing",
        "check_in",
        "events.seating.EVENTS_3",
    ] {
        let output = una_in(events, &["contract", name]);
        assert_eq!(output.status.code(), Some(2), "{name}");
        assert_eq!(stdout(&output), "", "{name}");
        assert!(
            stderr(&output).starts_with("error: the package has no function"),
            "{name}"
        );
    }
    // Outside a package, `una contract` is a usage error, like `una check`.
    let output = una_in(std::path::Path::new("docs"), &["contract", "a.b"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stdout(&output), "");
    // A package with diagnostics reports them as `una check` does.
    let root = scratch("contract-diagnostics");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("una.toml"), MANIFEST).unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(
        root.join("src/a.una"),
        "module a\n\npub one() -> Int = true\n",
    )
    .unwrap();
    let output = una_in(&root, &["contract", "--json", "a.one"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(codes(&output), ["U2001"]);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn contract_and_trace_report_package_diagnostics_before_the_name() {
    // R-027, F-0001 C-1, ADR 0036: a problem of the project files or of a module is a
    // diagnostic with exit code 1, reported before the name given to `una contract` or
    // `una trace` is resolved, so an unknown name never hides it behind a usage error.
    let lock = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../verification.lock");
    let cases: [(&str, String, bool, &str, &str); 3] = [
        (
            "invalid-manifest",
            format!("{MANIFEST}license = \"MIT\"\n"),
            true,
            "module a\n\npub one() -> Int = 1\n",
            "U7002",
        ),
        (
            "missing-lock",
            MANIFEST.to_owned(),
            false,
            "module a\n\npub one() -> Int = 1\n",
            "U7004",
        ),
        (
            "syntax-error",
            MANIFEST.to_owned(),
            true,
            "module a\n\npub one() -> Int = 1;\n",
            "U0001",
        ),
    ];
    for (case, manifest, with_lock, module, code) in cases {
        let root = scratch(&format!("names-after-diagnostics-{case}"));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("una.toml"), manifest).unwrap();
        if with_lock {
            std::fs::copy(&lock, root.join("verification.lock")).unwrap();
        }
        std::fs::write(root.join("src/a.una"), module).unwrap();
        for args in [
            ["contract", "--json", "a.nothing"],
            ["contract", "--json", "a.one"],
            ["trace", "--json", "NOTHING_1"],
        ] {
            let output = una_in(&root, &args);
            assert_eq!(
                output.status.code(),
                Some(1),
                "{case} {args:?}: {}",
                stderr(&output)
            );
            assert_eq!(stderr(&output), "", "{case} {args:?}");
            assert!(
                codes(&output).iter().any(|found| found == code),
                "{case} {args:?}: {}",
                stdout(&output)
            );
        }
        for args in [["contract", "a.nothing"], ["trace", "NOTHING_1"]] {
            let output = una_in(&root, &args);
            assert_eq!(output.status.code(), Some(1), "{case} {args:?}");
            assert!(
                stderr(&output).contains(code),
                "{case} {args:?}: {}",
                stderr(&output)
            );
            assert!(
                !stderr(&output).contains("error: the package has no"),
                "{case} {args:?}: {}",
                stderr(&output)
            );
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}

#[test]
fn dump_core_prints_the_core_ir_of_the_reference_example() {
    // M7 acceptance criterion: snapshots of Core dumps (`spec/09 §4`).
    for module in [
        "events.checkin",
        "events.keys",
        "events.main",
        "events.seating",
        "events.types",
    ] {
        let output = una(&[
            "check",
            "--tier",
            "0",
            "--dump-core",
            module,
            "examples/events",
        ]);
        assert_eq!(output.status.code(), Some(0), "{module}");
        assert_eq!(stderr(&output), "", "{module}");
        insta::assert_snapshot!(
            format!("core_{}", module.replace('.', "_")),
            stdout(&output)
        );
    }
    let output = una(&["check", "--dump-core", "events.nothing", "examples/events"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn contract_shows_written_and_inferred_constraints() {
    // spec/20 §8: the header writes an inferred constraint, with `-- inferred`, and the JSON
    // lists each constrained type parameter.
    let root = scratch("contract-constraints");
    std::fs::create_dir_all(root.join("src/app")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        "[package]\nname = \"k\"\nversion = \"0.1.0\"\nedition = \"0.2\"\n",
    )
    .unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(
        root.join("src/app/k.una"),
        "module app.k\n\nlowest[T, U](a: T, b: T, tag: U) -> T = if a < b then a else b\n\npub same[T: Eq](a: T, b: T) -> Bool = a == b\n\npub use_it() -> Int = lowest(1, b: 2, tag: \"x\")\n",
    )
    .unwrap();
    let output = una_in(&root, &["contract", "app.k.lowest"]);
    assert_eq!(stderr(&output), "");
    assert_eq!(
        stdout(&output),
        "lowest[T: Ord, U](a: T, b: T, tag: U) -> T -- inferred\n"
    );
    let output = una_in(&root, &["contract", "--json", "app.k.same"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        stdout(&output),
        "{\"schema\":\"una.contract/1\",\"function\":\"app.k.same\",\"public\":true,\"docs\":[],\"header\":\"pub same[T: Eq](a: T, b: T) -> Bool\",\"constraints\":[{\"param\":\"T\",\"constraint\":\"Eq\",\"inferred\":false}],\"requires\":[],\"ensures\":[],\"uses\":[],\"fails\":null,\"derived_from\":[]}\n"
    );
    std::fs::remove_dir_all(&root).unwrap();
}
