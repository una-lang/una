//! The `serve` section of `ita.una` (`spec/31 §7`, ADR 0123 rules 16 and 22): the usage errors of
//! `una run` and `una build` for a module without the shapes of `handle` and `on_stop` and for a
//! run without a `serve:` grant, the driver that `una run` runs and `una build` builds, the
//! `within` both refuse, the tests that call `handle` directly, and the report's server entries. Every package lies below a
//! sealed directory (#1560).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/hermetic.rs"]
mod hermetic;
#[path = "common/native_build.rs"]
mod native_build;

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

/// A port the kernel had free a moment ago.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// A run under test that is killed when it is dropped, so a test that fails never leaves it
/// serving.
struct Served(Option<std::process::Child>);

impl Drop for Served {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Served {
    fn child(&mut self) -> &mut std::process::Child {
        self.0.as_mut().unwrap()
    }

    /// SIGTERM, then its exit status within 30 s; past that the test fails and the run is
    /// killed.
    fn terminate(&mut self) -> std::process::ExitStatus {
        let killed = Command::new("kill")
            .args(["-TERM", &self.child().id().to_string()])
            .status()
            .unwrap();
        assert!(killed.success());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if let Some(status) = self.child().try_wait().unwrap() {
                self.0 = None;
                return status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the run did not end within 30 s of SIGTERM"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

/// Spawns `command`, which serves the section of `HEALTH` on `port`; waits for the first line
/// of its stderr, the notice of the address it serves (`spec/31 §6` rule 14), which comes before
/// any request; asks it `GET /health`, then sends it SIGTERM: the notice, the reply's status
/// line, the run's exit code and the rest of its stderr.
fn serve_once(mut command: Command, port: u16) -> (String, String, Option<i32>, String) {
    use std::io::{BufRead, Read, Write};
    let mut served = Served(Some(
        command
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let pipe = served.child().stderr.take().unwrap();
    let (sender, lines) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(pipe).lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let notice = lines
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("the run wrote no line on stderr");
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(30)))
        .unwrap();
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut reply = Vec::new();
    let _ = stream.read_to_end(&mut reply);
    let reply = String::from_utf8_lossy(&reply).into_owned();
    let status = reply.lines().next().unwrap_or("").to_owned();
    let code = served.terminate().code();
    let rest: Vec<String> = lines.iter().collect();
    (notice, status, code, rest.join("\n"))
}

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
    // A flag grants one, and the run's driver serves (`spec/31 §1` rule 10).
    let port = free_port();
    let mut command = Command::new(env!("CARGO_BIN_EXE_una"));
    command
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("NO_COLOR", "1")
        .env_remove("UNA_TEST_RUN_SERVER")
        .args(["run", ".", "--grant"])
        .arg(format!("serve:http://127.0.0.1:{port}"))
        .current_dir(&dir);
    let (notice, status, code, message) = serve_once(command, port);
    assert_eq!(
        notice,
        format!("serving on 127.0.0.1:{port} (granted by a `--grant` flag)")
    );
    assert_eq!(status, "HTTP/1.1 200 OK", "{message}");
    assert_eq!(code, Some(0), "{message}");
    let _ = std::fs::remove_dir_all(&dir);
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
fn una_run_serves_and_so_does_the_executable_una_build_makes() {
    // `spec/31 §1` rule 10, §7 rule 5: `una run` runs the driver, which answers `GET /health`
    // with `200` and ends with exit code 0 at a host stop; `una build`, with the plain
    // toolchain and no test flag, builds the driver into an executable that does the same.
    let port = free_port();
    let dir = package(
        "runs",
        &format!("    serve \"http://127.0.0.1:{port}\""),
        HEALTH,
    );
    let check = una(&dir, &["check", "."]);
    assert!(check.status.success(), "{}", stderr(&check));
    let mut command = Command::new(env!("CARGO_BIN_EXE_una"));
    command
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("NO_COLOR", "1")
        .env_remove("UNA_TEST_RUN_SERVER")
        .args(["run", "."])
        .current_dir(&dir);
    let (notice, status, code, message) = serve_once(command, port);
    assert_eq!(
        notice,
        format!("serving on 127.0.0.1:{port} (granted by the package)")
    );
    assert_eq!(status, "HTTP/1.1 200 OK", "{message}");
    assert_eq!(code, Some(0), "{message}");
    // The notice is the run's only line on stderr.
    assert_eq!(message, "");
    let build = plain_build(&dir);
    assert!(build.status.success(), "{}", stderr(&build));
    let mut command = Command::new(dir.join("app"));
    command
        .env("NO_COLOR", "1")
        .env_remove("UNA_TEST_RUN_SERVER")
        .current_dir(&dir);
    let (notice, status, code, message) = serve_once(command, port);
    assert_eq!(
        notice,
        format!("serving on 127.0.0.1:{port} (granted by the package)")
    );
    assert_eq!(status, "HTTP/1.1 200 OK", "{message}");
    assert_eq!(code, Some(0), "{message}");
    assert_eq!(message, "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn una_run_refuses_a_serve_section_with_within_before_it_serves() {
    // `spec/31 §7` rule 3 item 7: every `within` of a section is U4003 under the preview's
    // `L_srv`, so `una run` refuses the package before any listener is bound.
    let port = free_port();
    let dir = package(
        "within-run",
        &format!("    serve \"http://127.0.0.1:{port}\""),
        HEALTH,
    );
    let manifest = std::fs::read_to_string(dir.join("ita.una")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        manifest.replace("  module health\n", "  module health\n  within 5000\n"),
    )
    .unwrap();
    let run = una(&dir, &["run", "."]);
    assert_eq!(run.status.code(), Some(1), "{}", stderr(&run));
    assert!(stderr(&run).contains("U4003"), "{}", stderr(&run));
    assert!(
        std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
        "a listener is open"
    );
    let _ = std::fs::remove_dir_all(&dir);
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

/// `una build . --out app` of the package in `dir`, which builds the driver of a `serve` section
/// with no test flag (`spec/31 §1` rule 10).
fn plain_build(dir: &Path) -> Output {
    native_build::runtime();
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("NO_COLOR", "1")
        .args(["build", ".", "--out", "app"])
        .current_dir(dir)
        .output()
        .unwrap()
}

#[test]
fn an_executable_without_a_serve_grant_is_the_usage_error_of_rule_1() {
    // `spec/31 §7` rule 1: a run of a package with a `serve` section and no `serve:` grant, in
    // `run grants` or by `--grant`, is a usage error, exit code 2, before anything starts; the
    // executable that `una build` writes gives the error `una run` gives. The build itself
    // takes no grant: the executable's may come from its flags.
    let dir = package("ungranted-native", "    console", HEALTH);
    let built = plain_build(&dir);
    assert!(built.status.success(), "{}", stderr(&built));
    let ran = Command::new(dir.join("app"))
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(2), "{}", stderr(&ran));
    let interpreted = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("NO_COLOR", "1")
        .env_remove("UNA_TEST_RUN_SERVER")
        .args(["run", "."])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(
        interpreted.status.code(),
        Some(2),
        "{}",
        stderr(&interpreted)
    );
    assert!(
        stderr(&ran).contains("grants an address to serve on"),
        "{}",
        stderr(&ran)
    );
    assert_eq!(stderr(&ran), stderr(&interpreted));
    // With a `--grant` flag the executable serves, until a host stop ends it with exit code 0.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut served = Command::new(dir.join("app"))
        .env("NO_COLOR", "1")
        .arg("--grant")
        .arg(format!("serve:http://127.0.0.1:{port}"))
        .current_dir(&dir)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(60),
            "the executable never listened"
        );
        assert!(served.try_wait().unwrap().is_none(), "the executable ended");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let killed = Command::new("kill")
        .args(["-TERM", &served.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    let ended = served.wait_with_output().unwrap();
    assert_eq!(ended.status.code(), Some(0), "{}", stderr(&ended));
    // The executable names the address it served, as `una run` does (`spec/31 §6` rule 14).
    assert_eq!(
        stderr(&ended),
        format!("serving on 127.0.0.1:{port} (granted by a `--grant` flag)\n")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_serve_section_with_within_is_refused_by_the_build_and_the_executable() {
    // `spec/31 §7` rule 3 item 7: every `within` of a section is U4003 today, so the test build
    // never writes a driver that would run without the deadline it names.
    let dir = package("within-build", LOOPBACK, HEALTH);
    let manifest = std::fs::read_to_string(dir.join("ita.una")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        manifest.replace("  module health\n", "  module health\n  within 5000\n"),
    )
    .unwrap();
    let built = plain_build(&dir);
    assert_eq!(built.status.code(), Some(1), "{}", stderr(&built));
    assert!(stderr(&built).contains("U4003"), "{}", stderr(&built));
    assert!(!dir.join("app").exists(), "no executable is written");
    let _ = std::fs::remove_dir_all(&dir);
    // An executable whose description names a `within` is no description: the runtime refuses
    // to start it rather than run the driver without its deadline.
    let dir = package("within-description", LOOPBACK, HEALTH);
    let built = plain_build(&dir);
    assert!(built.status.success(), "{}", stderr(&built));
    let executable = std::fs::read(dir.join("app")).unwrap();
    let null = br#""within":null}"#;
    let at = executable
        .windows(null.len())
        .position(|window| window == null)
        .expect("the description names the section");
    let mut patched = executable.clone();
    patched[at..at + null.len()].copy_from_slice(br#""within":5000}"#);
    std::fs::write(dir.join("app"), patched).unwrap();
    // A changed executable needs its ad hoc signature again on macOS, or the kernel kills it.
    if cfg!(target_os = "macos") {
        let signed = Command::new("codesign")
            .args(["--force", "--sign", "-"])
            .arg(dir.join("app"))
            .output()
            .unwrap();
        assert!(signed.status.success(), "{}", stderr(&signed));
    }
    let ran = Command::new(dir.join("app"))
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(3), "{}", stderr(&ran));
    assert!(
        stderr(&ran).contains("embedded description is invalid"),
        "{}",
        stderr(&ran)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
