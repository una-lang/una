//! `serve:` grants in `una run` and in executables (`spec/31 §2`, ADR 0123 rules 6 to 9): the
//! usage errors of `--grant`, the edition, overlaps across the manifest and the flags, the TLS
//! files checked on their open descriptors, and the listener bound before `main`, in both
//! engines. The repository holds no private key, and these tests make none: the parsing of real
//! keys is tested in `una-http`. Ports come from the kernel.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/hermetic.rs"]
mod hermetic;

use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Builds the runtime library once, which `una build` links.
fn runtime() {
    static BUILT: OnceLock<()> = OnceLock::new();
    BUILT.get_or_init(|| {
        // Set by the nextest setup script (.config/nextest.toml): the library is built, and a second build in each test
        // process would rewrite it under the tests that link it.
        #[allow(clippy::disallowed_methods)]
        let prebuilt = std::env::var_os("UNA_RT_PREBUILT").is_some();
        if prebuilt {
            return;
        }
        let status = Command::new(env!("CARGO"))
            .args(["build", "--quiet", "-p", "una-rt"])
            .current_dir(root())
            .status()
            .unwrap();
        assert!(status.success(), "cargo build -p una-rt failed");
    });
}

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

const MAIN: &str = "module main\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"started\")\n";

/// A package of edition 0.4 named `app`, whose `main` prints `started`, with `grants` lines
/// under `run grants` beside `console`.
fn package(name: &str, grants: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("una-serve-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita app\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry main\n  grants\n    console\n{grants}"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/types/float_edition_0_4/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/main.una"), MAIN).unwrap();
    dir
}

/// A package of edition 0.3, described by `una.toml`.
fn package_0_3(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("una-serve-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n\n[run]\nentry = \"main\"\ngrants = [\"console\"]\n",
    )
    .unwrap();
    std::fs::copy(
        root().join("verification.lock"),
        dir.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/main.una"), MAIN).unwrap();
    dir
}

/// A port the kernel had free a moment ago.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// A certificate file and a key file in `dir` that hold no PEM block, the key with `mode`:
/// enough for the checks this test makes, which refuse before a real key would be needed. The
/// parsing of real keys is tested in `una-http`, with keys made at test time.
fn tls_files(dir: &Path, mode: u32) -> (PathBuf, PathBuf) {
    let chain = dir.join("chain.pem");
    let key_file = dir.join("key.pem");
    std::fs::write(&chain, "not a certificate\n").unwrap();
    std::fs::write(&key_file, "not a key\n").unwrap();
    std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(mode)).unwrap();
    (chain, key_file)
}

#[test]
fn a_valid_serve_grant_binds_before_main_and_the_run_goes_on() {
    let dir = package("plain", "");
    let spec = format!("serve:http://127.0.0.1:{};connections=8", free_port());
    let output = una(&dir, &["run", "--grant", &spec]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "started\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn invalid_serve_flags_are_usage_errors() {
    let dir = package("invalid", "");
    for (spec, message) in [
        (
            "serve:http://localhost:8080",
            "error: `serve:http://localhost:8080` is not a valid serve grant: `localhost` is not an IPv4 address; a serve grant names an IP address, never a host name, and IPv6 goes in brackets\n",
        ),
        (
            "serve:http://127.0.0.1:8080;line=65;tls12",
            "error: `serve:http://127.0.0.1:8080;line=65;tls12` is not a valid serve grant: `line` is a number of KiB from 1 to 64, not `65`; `tls12` is for https grants only; this grant is http\n",
        ),
        (
            "serve:https://127.0.0.1:8443",
            "error: `serve:https://127.0.0.1:8443` is not a valid serve grant: an https grant names its `certificate` file; an https grant names its `key` file\n",
        ),
        (
            "serve:http://127.0.0.1:8080;10/second;10/minute",
            "error: `serve:http://127.0.0.1:8080;10/second;10/minute` is not a valid serve grant: `10/minute` is a second rate; a serve grant has one rate\n",
        ),
    ] {
        let output = una(&dir, &["run", "--grant", spec]);
        assert_eq!(output.status.code(), Some(2), "{spec}");
        assert_eq!(stderr(&output), message, "{spec}");
        assert_eq!(stdout(&output), "", "{spec}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn serve_flags_need_edition_0_4() {
    let dir = package_0_3("edition");
    let spec = format!("serve:http://127.0.0.1:{}", free_port());
    let output = una(&dir, &["run", "--grant", &spec]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert_eq!(
        stderr(&output),
        format!(
            "error: `{spec}` needs a package of edition 0.4, described by ita.una; this package is of edition 0.3\n"
        )
    );
    // A single file outside any package is of edition 0.1: a file inside a package's
    // directory is read as that package. The seal keeps it of edition 0.1 whatever lies above
    // the temporary directory (#1560).
    let alone = std::env::temp_dir().join(format!("una-serve-alone-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&alone);
    hermetic::seal(&alone);
    let file = alone.join("main.una");
    std::fs::write(&file, MAIN).unwrap();
    let output = una(
        &alone,
        &[
            "run",
            file.to_str().unwrap(),
            "--grant",
            "console",
            "--grant",
            &spec,
        ],
    );
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(stderr(&output).ends_with("this package is of edition 0.1\n"));
    let _ = std::fs::remove_dir_all(&alone);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn overlaps_across_the_manifest_and_the_flags_are_refused() {
    let port = free_port();
    let dir = package(
        "overlap",
        &format!("    serve \"http://127.0.0.1:{port}\"\n"),
    );
    let output = una(
        &dir,
        &["run", "--grant", &format!("serve:http://0.0.0.0:{port}")],
    );
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert_eq!(
        stderr(&output),
        format!(
            "error: the serve grant on `0.0.0.0:{port}` overlaps another serve grant of the run: each address and port is granted once, and a wildcard shares its port with no grant of its family\n"
        )
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_address_the_run_cannot_bind_is_a_usage_error() {
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let dir = package("taken", &format!("    serve \"http://127.0.0.1:{port}\"\n"));
    let output = una(&dir, &["run"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).starts_with(&format!(
            "error: cannot listen on `127.0.0.1:{port}` for its serve grant: "
        )),
        "{}",
        stderr(&output)
    );
    assert_eq!(stdout(&output), "", "main never starts");
    drop(taken);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_tls_files_are_checked_on_their_descriptors_before_main() {
    let dir = package("tls", "");
    let (chain, key) = tls_files(&dir, 0o644);
    let spec = format!(
        "serve:https://127.0.0.1:{};certificate={};key={}",
        free_port(),
        chain.display(),
        key.display()
    );
    let output = una(&dir, &["run", "--grant", &spec]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert_eq!(
        stderr(&output),
        format!(
            "error: the private key `{}` has mode 0644: a key file gives others no access and its group no write access, such as 0400, 0600 or 0440\n",
            key.display()
        )
    );
    assert_eq!(stdout(&output), "", "main never starts");
    // With its mode right, the key is read from the descriptor, and a file with no key is
    // refused; the message names the path, never the content.
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    let output = una(&dir, &["run", "--grant", &spec]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert_eq!(
        stderr(&output),
        format!(
            "error: the private key `{}` holds no PEM private key in PKCS#8, SEC1 or PKCS#1 form\n",
            key.display()
        )
    );
    // A missing file is refused before anything runs.
    let missing = dir.join("missing.pem");
    let spec = format!(
        "serve:https://127.0.0.1:{};certificate={};key={}",
        free_port(),
        chain.display(),
        missing.display()
    );
    let output = una(&dir, &["run", "--grant", &spec]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).starts_with(&format!(
            "error: cannot open the private key `{}`: ",
            missing.display()
        )),
        "{}",
        stderr(&output)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn executables_check_serve_grants_as_una_run_does() {
    runtime();
    let dir = package("native", "");
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let exe = dir.join("target/una/app");
    let run = |args: &[&str]| {
        Command::new(&exe)
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap()
    };
    let output = run(&["--grant", "serve:http://localhost:8080"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(
        stderr(&output)
            .starts_with("error: `serve:http://localhost:8080` is not a valid serve grant: ")
    );
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let output = run(&["--grant", &format!("serve:http://127.0.0.1:{port}")]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(stderr(&output).starts_with(&format!("error: cannot listen on `127.0.0.1:{port}`")));
    drop(taken);
    let output = run(&[
        "--grant",
        &format!("serve:http://127.0.0.1:{}", free_port()),
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(stdout(&output), "started\n");
    let _ = std::fs::remove_dir_all(&dir);

    // An executable of edition 0.3 refuses a `serve:` grant.
    let old = package_0_3("native-0-3");
    let built = una(&old, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let output = Command::new(old.join("target/una/app"))
        .args([
            "--grant",
            &format!("serve:http://127.0.0.1:{}", free_port()),
        ])
        .current_dir(&old)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(
        stderr(&output).ends_with(
            "needs a program of edition 0.4; this executable is of an earlier edition\n"
        )
    );
    let _ = std::fs::remove_dir_all(&old);
}

/// A server of the language form (`spec/31 §1` rule 6), whose `main` uses `HttpServer`.
const SERVER: &str = "module main\n\nimport std.host (Exchange, HttpServer, Reply)\n\nanswer(exchange: internal Exchange) -> Unit\n  uses @server: HttpServer[reply]\n=\n  @server.reply(to: exchange.right, reply: Reply(status: 204, headers: [], body: \"\"))\n    if error use unit\n\npub main() -> Unit\n  uses @server: HttpServer[next, reply, close]\n  diverges\n=\n  tasks\n    for ever\n      let next = attempt @server.next()\n      when next\n        is Ok(exchange) ->\n          start answer(exchange)\n        is Err(failure) -> unit\n    on stop\n      @server.close()\n";

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

/// One `GET /` to `command`, a run that serves on `port`, then SIGTERM: the reply and the run.
fn serve_once(mut command: Command, port: u16) -> (String, std::process::Output) {
    use std::io::{Read as _, Write as _};
    let mut served = Served(Some(
        command
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let mut pipe = served.child().stderr.take().unwrap();
    let errors = std::thread::spawn(move || {
        let mut errors = Vec::new();
        let _ = pipe.read_to_end(&mut errors);
        errors
    });
    let started = std::time::Instant::now();
    let mut stream = loop {
        if let Ok(stream) = std::net::TcpStream::connect(("127.0.0.1", port)) {
            break stream;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(60),
            "the run never listened"
        );
        assert!(
            served.child().try_wait().unwrap().is_none(),
            "the run ended"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(30)))
        .unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut reply = Vec::new();
    let _ = stream.read_to_end(&mut reply);
    let status = served.terminate();
    let ended = std::process::Output {
        status,
        stdout: Vec::new(),
        stderr: errors.join().unwrap(),
    };
    (String::from_utf8_lossy(&reply).into_owned(), ended)
}

#[test]
fn una_run_and_the_executable_una_build_makes_serve_a_program_whose_main_uses_http_server() {
    // `spec/31 §1` rule 10: `una run` of a program that serves runs it, and `una build` builds
    // it, without any test flag: a request gets the program's `204`, and a host stop ends the
    // run with exit code 0, in both engines.
    let port = free_port();
    let dir = package(
        "serving",
        &format!("    serve \"http://127.0.0.1:{port}\"\n"),
    );
    std::fs::write(dir.join("src/main.una"), SERVER).unwrap();
    let checked = una(&dir, &["check"]);
    assert!(checked.status.success(), "{}", stderr(&checked));
    runtime();
    let out = dir.join("server");
    let built = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("NO_COLOR", "1")
        .args(["build", "--out", out.to_str().unwrap()])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(built.status.success(), "{}", stderr(&built));
    let mut interpreted = Command::new(env!("CARGO_BIN_EXE_una"));
    interpreted
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("NO_COLOR", "1")
        .env_remove("UNA_TEST_RUN_SERVER")
        .arg("run")
        .current_dir(&dir);
    let mut native = Command::new(&out);
    native
        .env("NO_COLOR", "1")
        .env_remove("UNA_TEST_RUN_SERVER")
        .current_dir(&dir);
    for (engine, command) in [("interpreter", interpreted), ("native", native)] {
        let (reply, ended) = serve_once(command, port);
        assert!(
            reply.starts_with("HTTP/1.1 204"),
            "{engine}: {reply}{}",
            stderr(&ended)
        );
        assert_eq!(ended.status.code(), Some(0), "{engine}: {}", stderr(&ended));
        // The address it served, granted by `run grants` (`spec/31 §6` rule 14).
        assert_eq!(
            stderr(&ended),
            format!("serving on 127.0.0.1:{port} (granted by the package)\n"),
            "{engine}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The first `count` lines of stderr of `command`, a run that serves, then SIGTERM and its exit
/// code.
fn notices(mut command: Command, count: usize) -> (Vec<String>, Option<i32>) {
    use std::io::BufRead as _;
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
        for line in std::io::BufReader::new(pipe).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let seen: Vec<String> = (0..count)
        .map(|_| {
            lines
                .recv_timeout(std::time::Duration::from_secs(60))
                .expect("a notice")
        })
        .collect();
    (seen, served.terminate().code())
}

#[test]
fn a_run_that_serves_names_each_grant_in_order_in_both_engines() {
    // `spec/31 §6` rule 14: one line per `serve:` grant, the package's before the flags', the
    // same in `una run` and in an executable of the language form.
    runtime();
    let (own, flag) = (free_port(), free_port());
    let dir = package(
        "notices",
        &format!("    serve \"http://127.0.0.1:{own}\"\n"),
    );
    std::fs::write(dir.join("src/main.una"), SERVER).unwrap();
    let built = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .arg("build")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(built.status.success(), "{}", stderr(&built));
    let grant = format!("serve:http://127.0.0.1:{flag}");
    let mut interpreted = Command::new(env!("CARGO_BIN_EXE_una"));
    interpreted
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_TEST_RUN_SERVER")
        .args(["run", "--grant", &grant])
        .current_dir(&dir);
    let mut native = Command::new(dir.join("target/una/app"));
    native.args(["--grant", &grant]).current_dir(&dir);
    let wanted = [
        format!("serving on 127.0.0.1:{own} (granted by the package)"),
        format!("serving on 127.0.0.1:{flag} (granted by a `--grant` flag)"),
    ];
    for (engine, command) in [("interpreter", interpreted), ("native", native)] {
        let (seen, code) = notices(command, 2);
        assert_eq!(seen, wanted, "{engine}");
        assert_eq!(code, Some(0), "{engine}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A server of the language form whose `main` does not say `diverges`: it takes no host stop
/// (ADR 0116 rule 18), so the run keeps the default action of SIGTERM.
const BOUNDED_SERVER: &str = "module main\n\nimport std.host (HttpServer, Reply)\n\npub main() -> Unit\n  uses @server: HttpServer[next, reply]\n=\n  let next = attempt @server.next()\n  when next\n    is Ok(exchange) ->\n      @server.reply(to: exchange.right, reply: Reply(status: 204, headers: [], body: \"\"))\n        if error use unit\n    is Err(failure) -> unit\n";

#[test]
fn a_language_form_server_without_diverges_ends_by_the_default_action_of_sigterm() {
    // ADR 0116 rule 18, `spec/31 §6` rule 14: only a run that can take a host stop installs the
    // handlers of SIGINT and SIGTERM. A `main` without `diverges` takes none, so SIGTERM ends
    // the run at once with 143 (128 + 15), with no drain, while it waits in `next`: under
    // `una run` and in the executable that `una build` makes.
    use std::os::unix::process::ExitStatusExt as _;
    let port = free_port();
    let dir = package(
        "bounded",
        &format!("    serve \"http://127.0.0.1:{port}\"\n"),
    );
    std::fs::write(dir.join("src/main.una"), BOUNDED_SERVER).unwrap();
    let checked = una(&dir, &["check"]);
    assert!(checked.status.success(), "{}", stderr(&checked));
    runtime();
    let out = dir.join("server");
    let built = una(&dir, &["build", "--out", out.to_str().unwrap()]);
    assert!(built.status.success(), "{}", stderr(&built));
    let mut interpreted = Command::new(env!("CARGO_BIN_EXE_una"));
    interpreted
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_TEST_RUN_SERVER")
        .arg("run");
    for (engine, mut command) in [("interpreter", interpreted), ("native", Command::new(&out))] {
        let mut served = Served(Some(
            command
                .env("NO_COLOR", "1")
                .current_dir(&dir)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        ));
        let started = std::time::Instant::now();
        while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(60),
                "{engine}: the run never listened"
            );
            assert!(
                served.child().try_wait().unwrap().is_none(),
                "{engine}: the run ended"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let status = served.terminate();
        assert_eq!(status.code(), None, "{engine}");
        assert_eq!(status.signal(), Some(15), "{engine}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_executable_whose_address_is_taken_is_a_usage_error() {
    // `spec/31 §2` rule 7: an address the run cannot bind is a usage error before `main`, in the
    // executable that `una build` makes of a program that serves, as in `una run`, with no hang.
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let dir = package(
        "taken-native",
        &format!("    serve \"http://127.0.0.1:{port}\"\n"),
    );
    std::fs::write(dir.join("src/main.una"), SERVER).unwrap();
    runtime();
    let out = dir.join("server");
    let built = una(&dir, &["build", "--out", out.to_str().unwrap()]);
    assert!(built.status.success(), "{}", stderr(&built));
    let mut served = Served(Some(
        Command::new(&out)
            .env("NO_COLOR", "1")
            .current_dir(&dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while served.child().try_wait().unwrap().is_none() {
        assert!(std::time::Instant::now() < deadline, "the executable hung");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let ended = served.0.take().unwrap().wait_with_output().unwrap();
    assert_eq!(ended.status.code(), Some(2), "{}", stderr(&ended));
    assert!(
        stderr(&ended).starts_with(&format!("error: cannot listen on `127.0.0.1:{port}`")),
        "{}",
        stderr(&ended)
    );
    assert_eq!(stdout(&ended), "", "main never starts");
    drop(taken);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_run_of_a_program_that_serves_an_https_grant_fails_closed_in_both_engines() {
    // `spec/31 §6` rule 10: no TLS server is built while the TLS provider is undecided (#1059),
    // so a run that would serve an `https` grant is a usage error before `main`, with valid
    // files, and never serves plain bytes on the address: in `una run` with a flag, and with
    // the grant in `run grants` in `una run` and in the executable that `una build` makes.
    let port = free_port();
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["localhost".to_owned()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let tls = |dir: &Path| {
        let (chain, key_file) = (dir.join("chain.pem"), dir.join("key.pem"));
        std::fs::write(&chain, cert.pem()).unwrap();
        std::fs::write(&key_file, key.serialize_pem()).unwrap();
        std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600)).unwrap();
        (chain, key_file)
    };
    let refused = |engine: &str, output: Output| {
        assert_eq!(
            output.status.code(),
            Some(2),
            "{engine}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains("an https serve grant cannot be served yet"),
            "{engine}: {}",
            stderr(&output)
        );
        assert_eq!(stdout(&output), "", "{engine}: main never starts");
        assert!(
            std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
            "{engine}: a listener is open"
        );
    };
    let dir = package("https", "");
    std::fs::write(dir.join("src/main.una"), SERVER).unwrap();
    let (chain, key_file) = tls(&dir);
    let spec = format!(
        "serve:https://127.0.0.1:{port};certificate={};key={}",
        chain.display(),
        key_file.display()
    );
    refused("flag", una(&dir, &["run", "--grant", &spec]));
    let _ = std::fs::remove_dir_all(&dir);
    let dir = package(
        "https-built",
        &format!(
            "    serve \"https://127.0.0.1:{port}\"\n      certificate \"chain.pem\"\n      key \"key.pem\"\n"
        ),
    );
    std::fs::write(dir.join("src/main.una"), SERVER).unwrap();
    tls(&dir);
    runtime();
    let out = dir.join("server");
    let built = una(&dir, &["build", "--out", out.to_str().unwrap()]);
    assert!(built.status.success(), "{}", stderr(&built));
    refused("interpreter", una(&dir, &["run"]));
    let native = Command::new(&out)
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .output()
        .unwrap();
    refused("native", native);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_core_of_a_package_that_imports_no_server_name_names_nothing_of_the_server() {
    // ADR 0123 rule 0 (`spec/31 §1`): Core IR declares `HttpServer` and its kinds only for a
    // package that imports a name of the server, so the `una.core/1` document of every other
    // package of edition 0.4 is what it was before them, and refers to no kind it lacks.
    let dir = package("core", "");
    let plain = una(&dir, &["core", "--json"]);
    assert!(plain.status.success(), "{}", stderr(&plain));
    let text = stdout(&plain);
    for name in ["HttpServer", "Exchange", "ServeError", "reply_right"] {
        assert!(
            !text.contains(name),
            "{name} in the core of a package that serves nothing"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
    let dir = package(
        "core-serving",
        &format!("    serve \"http://127.0.0.1:{}\"\n", free_port()),
    );
    std::fs::write(dir.join("src/main.una"), SERVER).unwrap();
    let serving = una(&dir, &["core", "--json"]);
    assert!(serving.status.success(), "{}", stderr(&serving));
    let text = stdout(&serving);
    for name in ["HttpServer", "Exchange", "ServeError", "reply_right"] {
        assert!(
            text.contains(name),
            "{name} missing from the core of a server"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_script_of_connections_is_checked_before_main_in_both_engines() {
    // ADR 0123 rule 21, `spec/31 §8` rule 6: `--serve-script` needs `--virtual-time`,
    // `--serve-out` needs `--serve-script`, and after the key files the script is read and
    // checked in full, a regular file of at most 64 MiB; its connections do not run in this
    // release yet, so every run that gives it ends with a usage error, exit code 2, the same in
    // both engines, with no output, no trace file touched and no listener bound.
    runtime();
    let dir = package("script", "");
    let built = una(&dir, &["build"]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let exe = dir.join("target/una/app");
    // The test holds the grant's port, so a run that tried to listen on it would report that.
    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    let grant = format!("serve:http://127.0.0.1:{port}");
    let open = format!(
        "{{\"at\": 0, \"connection\": 1, \"grant\": \"127.0.0.1:{port}\", \"peer\": \"192.0.2.1\", \"reads\": \"paced\"}}\n"
    );
    let path = |name: &str| dir.join(name).to_str().unwrap().to_owned();
    let write = |name: &str, text: &str| {
        std::fs::write(dir.join(name), text).unwrap();
        path(name)
    };
    let valid = write(
        "valid.jsonl",
        &format!(
            "{open}{{\"at\": 3, \"connection\": 1, \"bytes\": \"R0VUIC8gSFRUUC8xLjENCkhvc3Q6IGxvY2FsaG9zdA0KDQo=\"}}\n{{\"at\": 3, \"connection\": 1, \"read\": 4096}}\n{{\"at\": 9, \"connection\": 1, \"close\": true}}\n"
        ),
    );
    let earlier = write(
        "earlier.jsonl",
        &format!(
            "{}{}",
            open.replace("\"at\": 0", "\"at\": 9"),
            open.replace("\"at\": 0", "\"at\": 5")
                .replace("\"connection\": 1", "\"connection\": 2")
        ),
    );
    let missing = path("missing.jsonl");
    // A FIFO would block a read, and a device never end: neither is read.
    let fifo = path("fifo.jsonl");
    let made = Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(made.success());
    std::fs::create_dir_all(dir.join("folder.jsonl")).unwrap();
    let folder = path("folder.jsonl");
    let large = path("large.jsonl");
    std::fs::File::create(&large)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    let bad_keys = write("bad-keys.toml", "x");
    let keys = format!("keys:{bad_keys}");
    let not_regular = "is not a regular file".to_owned();
    let cases: Vec<(Vec<&str>, String)> = vec![
        (
            vec!["--grant", &grant, "--serve-script", &valid],
            "`--serve-script` needs `--virtual-time`".to_owned(),
        ),
        // The flags of a script come before the values of the other flags.
        (
            vec!["--workers", "0", "--serve-script", &valid],
            "`--serve-script` needs `--virtual-time`".to_owned(),
        ),
        (
            vec!["--virtual-time", "--serve-out", "out.jsonl"],
            "`--serve-out` needs `--serve-script`".to_owned(),
        ),
        (
            vec!["--virtual-time", "--serve-script", &valid],
            "the run has none".to_owned(),
        ),
        (
            vec!["--virtual-time", "--serve-script", &missing],
            "the run has none".to_owned(),
        ),
        // The key files come before the script, in both engines.
        (
            vec![
                "--virtual-time",
                "--grant",
                &grant,
                "--grant",
                &keys,
                "--serve-script",
                &valid,
            ],
            format!("the key file `{bad_keys}` is invalid"),
        ),
        (
            vec![
                "--virtual-time",
                "--grant",
                &grant,
                "--serve-script",
                &missing,
            ],
            format!("cannot read the script `{missing}` of `--serve-script`"),
        ),
        (
            vec!["--virtual-time", "--grant", &grant, "--serve-script", &fifo],
            not_regular.clone(),
        ),
        (
            vec![
                "--virtual-time",
                "--grant",
                &grant,
                "--serve-script",
                "/dev/zero",
            ],
            not_regular.clone(),
        ),
        (
            vec![
                "--virtual-time",
                "--grant",
                &grant,
                "--serve-script",
                &folder,
            ],
            not_regular,
        ),
        (
            vec![
                "--virtual-time",
                "--grant",
                &grant,
                "--serve-script",
                &large,
            ],
            "is larger than 64 MiB".to_owned(),
        ),
        (
            vec![
                "--virtual-time",
                "--grant",
                &grant,
                "--serve-script",
                &earlier,
            ],
            "line 2: `at` is 5, before the line above it, at 9".to_owned(),
        ),
        (
            vec![
                "--virtual-time",
                "--grant",
                &grant,
                "--serve-script",
                &valid,
                "--serve-out",
                "out.jsonl",
                "--trace",
                "kept.json",
            ],
            "the scripted connections of `--serve-script` do not run in this release yet"
                .to_owned(),
        ),
        // Without a script, virtual time still takes no `serve:` grant.
        (
            vec!["--virtual-time", "--grant", &grant, "--trace", "kept.json"],
            "live peers send in real time".to_owned(),
        ),
    ];
    for (args, wanted) in cases {
        std::fs::write(dir.join("kept.json"), "kept\n").unwrap();
        let native = Command::new(&exe)
            .args(&args)
            .current_dir(&dir)
            .output()
            .unwrap();
        let native_kept = std::fs::read_to_string(dir.join("kept.json")).unwrap();
        let mut run = vec!["run"];
        run.extend_from_slice(&args);
        let interpreted = una(&dir, &run);
        let interpreted_kept = std::fs::read_to_string(dir.join("kept.json")).unwrap();
        for (engine, output, kept) in [
            ("native", &native, native_kept),
            ("interpreter", &interpreted, interpreted_kept),
        ] {
            assert_eq!(
                output.status.code(),
                Some(2),
                "{engine} {args:?}: {output:?}"
            );
            assert!(
                stderr(output).contains(&wanted),
                "{engine} {args:?}: {}",
                stderr(output)
            );
            assert_eq!(stdout(output), "", "{engine} {args:?}");
            assert_eq!(
                kept, "kept\n",
                "{engine} {args:?}: the trace file was touched"
            );
        }
        assert_eq!(stderr(&native), stderr(&interpreted), "{args:?}");
    }
    assert!(!dir.join("out.jsonl").exists());
    drop(held);
    // A program that serves runs under `una run` (`spec/31 §1` rule 10), but its scripted
    // connections still do not run outside the toolchain's own tests (`spec/31 §8` rule 6), and
    // without a script virtual time still takes no `serve:` grant: each a usage error before
    // anything is bound.
    std::fs::write(dir.join("src/main.una"), SERVER).unwrap();
    let cases: [(Vec<&str>, &str); 2] = [
        (
            vec![
                "run",
                "--virtual-time",
                "--grant",
                &grant,
                "--serve-script",
                &valid,
            ],
            "the scripted connections of `--serve-script` do not run in this release yet",
        ),
        (
            vec!["run", "--virtual-time", "--grant", &grant],
            "live peers send in real time",
        ),
    ];
    for (args, wanted) in cases {
        let ran = una(&dir, &args);
        assert_eq!(ran.status.code(), Some(2), "{args:?}: {ran:?}");
        assert!(stderr(&ran).contains(wanted), "{args:?}: {}", stderr(&ran));
        assert_eq!(stdout(&ran), "", "{args:?}");
        assert!(
            std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
            "{args:?}: a listener is open"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
