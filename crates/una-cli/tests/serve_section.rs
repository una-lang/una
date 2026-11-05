//! The `serve` section of `ita.una` (`spec/31 §7`, ADR 0123 rules 16 and 22): the usage errors of
//! `una run` and `una build` for a module without the shapes of `handle` and `on_stop` and for a
//! run without a `serve:` grant, the driver's fail-closed usage error while it does not run, the
//! tests that call `handle` directly, and the report's server entries. Every package lies below a
//! sealed directory (#1560).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/hermetic.rs"]
mod hermetic;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn una(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("NO_COLOR", "1")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

const HEALTH: &str = "module health\n\nimport std.host (Reply, Request)\n\n--- Answers `GET /health` with 200, and every other request with 404.\npub handle(request: internal Request) -> internal Reply\n=\n  if request.method is Get and request.segments == [\"health\"]\n    Reply(status: 200, headers: [], body: \"ok\")\n  else\n    Reply(status: 404, headers: [], body: \"\")\n";

/// A package of edition 0.4 named `app` under a sealed directory, with `grants` lines under
/// `run grants`, a `serve` section for the module `health`, and `health` as its source.
fn package(name: &str, grants: &str, health: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("serve-section");
    hermetic::seal(&base);
    let dir = base.join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let run = if grants.is_empty() {
        String::new()
    } else {
        format!("run\n  grants\n{grants}\n")
    };
    std::fs::write(
        dir.join("ita.una"),
        format!("ita app\n  version 0.1.0\n  edition 0.4\n\n{run}serve\n  module health\n"),
    )
    .unwrap();
    std::fs::copy(
        hermetic::repository().join("tests/conformance/types/float_edition_0_4/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/health.una"), health).unwrap();
    // The sources are written for reading here, and put in canonical form for the checks.
    let formatted = una(&dir, &["fmt", "."]);
    assert!(formatted.status.success(), "{}", stderr(&formatted));
    dir
}

const LOOPBACK: &str = "    serve \"http://127.0.0.1:8080\"";

const PENDING: &str = "does not run in this release yet";

#[test]
fn a_module_without_handle_of_its_shape_is_a_usage_error() {
    let cases = [
        (
            "module health\n\npub value() -> Int = 1\n",
            "it declares no `handle`",
        ),
        (
            "module health\n\nimport std.host (Reply, Request)\n\nhandle(request: internal Request) -> internal Reply = Reply(status: 200,\n  headers: [], body: \"ok\"\n)\n\n--- Uses `handle`.\npub value(request: internal Request) -> internal Reply = handle(request)\n",
            "its `handle` is not public",
        ),
        (
            "module health\n\nimport std.host (Reply, Request)\n\n--- Answers every request.\npub handle(request: internal Request, extra: Int) -> internal Reply = Reply(status: 200,\n  headers: [], body: \"{extra}\"\n)\n",
            "does not take exactly one parameter",
        ),
        (
            "module health\n\nimport std.host (Reply)\n\n--- Answers every request.\npub handle(request: Int) -> internal Reply = Reply(status: 200, headers: [], body: \"{request}\")\n",
            "is not `internal Request` of `std.host`",
        ),
        (
            "module health\n\nimport std.host (Request)\n\n--- Answers every request.\npub handle(request: internal Request) -> internal Text = request.target\n",
            "is not `internal Reply` of `std.host`",
        ),
    ];
    for (index, (source, expected)) in cases.iter().enumerate() {
        let dir = package(&format!("shape-{index}"), LOOPBACK, source);
        let check = una(&dir, &["check", "."]);
        assert!(
            check.status.success(),
            "case {index}: the checks pass; the shape is the run's\n{}",
            stderr(&check)
        );
        for command in [vec!["run", "."], vec!["build", ".", "--out", "app"]] {
            let output = una(&dir, &command);
            let message = stderr(&output);
            assert_eq!(output.status.code(), Some(2), "case {index}: {message}");
            assert!(
                message.contains("the module `health` of the `serve` section has no `pub handle(request: internal Request) -> internal Reply`")
                    && message.contains(expected),
                "case {index} {command:?}: {message}"
            );
            assert!(!dir.join("app").exists(), "no executable is written");
        }
    }
}

#[test]
fn an_on_stop_of_another_shape_is_a_usage_error() {
    let source =
        format!("{HEALTH}\n--- Stops with a count.\npub on_stop(count: Int) -> Unit = unit\n");
    let dir = package("on-stop", LOOPBACK, &source);
    let output = una(&dir, &["run", "."]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains(
            "`on_stop` of the module `health` of the `serve` section is not `pub on_stop() -> Unit`"
        ),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_run_without_a_serve_grant_is_a_usage_error() {
    let dir = package("no-grant", "    console", HEALTH);
    let output = una(&dir, &["run", "."]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains(
            "neither `run grants` of ita.una nor a `--grant` flag grants an address to serve on"
        ),
        "{}",
        stderr(&output)
    );
    // A flag grants one, and the run reaches the driver, which fails closed.
    let output = una(
        &dir,
        &["run", ".", "--grant", "serve:http://127.0.0.1:8080"],
    );
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(stderr(&output).contains(PENDING), "{}", stderr(&output));
}

#[test]
fn the_final_grants_of_a_run_check_handle() {
    // `spec/31 §7` rule 3: `una run` checks the `uses` of `handle` against its final grants, the
    // flags included, as `main`'s, although the package has no `run entry`.
    let source = HEALTH.replace(
        "pub handle(request: internal Request) -> internal Reply\n=\n",
        "pub handle(request: internal Request) -> internal Reply\n  uses @console: Console[print]\n=\n  @console.print(\"request\")\n",
    );
    let dir = package("final-grants", "", &source);
    let output = una(
        &dir,
        &["run", ".", "--grant", "serve:http://127.0.0.1:8080"],
    );
    let message = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "{message}");
    assert!(
        message.contains("U3012") && message.contains("`handle` uses `print` of `Console`"),
        "{message}"
    );
    assert!(!message.contains(PENDING), "{message}");
}

#[test]
fn a_requires_on_handle_is_refused() {
    let source = HEALTH.replace(
        "pub handle(request: internal Request) -> internal Reply\n=\n",
        "pub handle(request: internal Request) -> internal Reply\n  requires request.body == \"\"\n=\n",
    );
    let dir = package("requires", LOOPBACK, &source);
    let check = una(&dir, &["check", "."]);
    assert_eq!(check.status.code(), Some(1), "{}", stderr(&check));
    assert!(stderr(&check).contains("U7008"), "{}", stderr(&check));
}

#[test]
fn the_driver_fails_closed_before_any_listener_is_bound() {
    // A port in use by this test: a run that bound its listener would fail on it differently.
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    let dir = package(
        "pending",
        &format!("    serve \"http://127.0.0.1:{port}\""),
        HEALTH,
    );
    let check = una(&dir, &["check", "."]);
    assert!(check.status.success(), "{}", stderr(&check));
    let run = una(&dir, &["run", "."]);
    assert_eq!(run.status.code(), Some(2), "{}", stderr(&run));
    assert!(
        stderr(&run).contains("the `serve` section's module `health` is checked")
            && stderr(&run).contains(PENDING),
        "{}",
        stderr(&run)
    );
    assert!(!stderr(&run).contains("address"), "{}", stderr(&run));
    let build = una(&dir, &["build", ".", "--out", "app"]);
    assert_eq!(build.status.code(), Some(2), "{}", stderr(&build));
    assert!(stderr(&build).contains(PENDING), "{}", stderr(&build));
    assert!(!dir.join("app").exists(), "no executable is written");
    drop(held);
}

#[test]
fn tests_call_handle_directly() {
    let source = format!(
        "{HEALTH}\n--- A `GET` of a path of one segment.\nget(segment: Text) -> internal Request\n=\n  Request(method: Get, target: \"/{{segment}}\", segments: [segment], params: [], headers: [],\n    body: \"\", peer: \"192.0.2.1\", listener: \"test\"\n  )\n\ntest \"the health endpoint answers ok\"\n  do handle(get(\"health\"))\n  expect result.status == 200\n"
    )
    .replace(
        "import std.host (Reply, Request)",
        "import std.host (Method, Reply, Request)",
    );
    let dir = package("tests", LOOPBACK, &source);
    let output = una(&dir, &["test", "."]);
    assert!(
        output.status.success(),
        "{}{}",
        stdout(&output),
        stderr(&output)
    );
    assert!(
        stdout(&output).contains("1 test: 1 passed, 0 failed"),
        "{}",
        stdout(&output)
    );
}

fn report(dir: &Path) -> serde_json::Value {
    let output = una(dir, &["report", "--json", "."]);
    assert!(output.status.success(), "{}", stderr(&output));
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn the_report_lists_the_servers_grants_and_warnings() {
    let grants = "    serve \"http://0.0.0.0:8080\"\n      hosts \"*\"\n      peer none\n    serve \"http://127.0.0.1:8081\"\n      at most 10 per second\n      peer_buffers none";
    let dir = package("report", grants, HEALTH);
    let document = report(&dir);
    let tcb = &document["tcb"];
    assert_eq!(tcb["host_capabilities"], serde_json::json!(["HttpServer"]));
    assert_eq!(tcb["server"], "una-http/serve");
    assert_eq!(tcb["tls"], serde_json::Value::Null);
    let grants = tcb["serve_grants"].as_array().unwrap();
    assert_eq!(grants.len(), 2);
    assert_eq!(grants[0]["address"], "0.0.0.0:8080");
    assert_eq!(grants[0]["scheme"], "http");
    assert_eq!(grants[0]["checks_host"], false);
    assert_eq!(grants[0]["limits"]["peer"], serde_json::Value::Null);
    assert_eq!(grants[0]["limits"]["once"], 256);
    assert_eq!(grants[0]["limits"]["answer"], 30);
    assert_eq!(grants[1]["checks_host"], true);
    assert_eq!(grants[1]["limits"]["rate"], "10/second");
    assert_eq!(grants[1]["limits"]["peer_buffers"], serde_json::Value::Null);
    let out_of_model = document["out_of_model"].as_array().unwrap();
    assert_eq!(out_of_model.len(), 16);
    assert_eq!(
        out_of_model[13],
        "what a server's peers send in reaction to its timing, and which requests its full queues refuse"
    );
    assert_eq!(
        out_of_model[14],
        "what a peer receives when the host's deadlines cut an exchange"
    );
    let warnings: Vec<&str> = document["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|warning| warning.as_str().unwrap())
        .collect();
    assert_eq!(warnings.len(), 4, "{warnings:?}");
    assert!(warnings[0].contains("`0.0.0.0:8080` serves plain `http`"));
    assert!(warnings[1].contains("`hosts=*`"));
    assert!(warnings[2].contains("`peer=none`"));
    assert!(warnings[3].contains("`127.0.0.1:8081` has `peer_buffers=none`"));
    // The human form shows the same facts.
    let human = una(&dir, &["report", "."]);
    let text = stdout(&human);
    assert!(text.contains("  server: una-http/serve"), "{text}");
    assert!(text.contains("servers:\n  http://0.0.0.0:8080:"), "{text}");
    assert!(
        text.contains("warning: the grant `0.0.0.0:8080` has `hosts=*`"),
        "{text}"
    );
}

#[test]
fn a_package_that_serves_nothing_has_no_server_entries() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("serve-section");
    hermetic::seal(&base);
    let dir = base.join(format!("plain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        "ita app\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry main\n  grants\n    console\n",
    )
    .unwrap();
    std::fs::copy(
        hermetic::repository().join("tests/conformance/types/float_edition_0_4/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(
        dir.join("src/main.una"),
        "module main\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"started\")\n",
    )
    .unwrap();
    let document = report(&dir);
    assert_eq!(
        document["tcb"]["host_capabilities"],
        serde_json::json!(["Console"])
    );
    assert_eq!(document["tcb"]["serve_grants"], serde_json::json!([]));
    assert_eq!(document["tcb"]["server"], serde_json::Value::Null);
    assert_eq!(document["out_of_model"].as_array().unwrap().len(), 13);
    assert_eq!(document["warnings"], serde_json::json!([]));
}
