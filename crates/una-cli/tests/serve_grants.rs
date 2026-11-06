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
        if std::env::var_os("UNA_RT_PREBUILT").is_some() {
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

#[test]
fn a_run_or_a_build_whose_main_uses_http_server_fails_closed_until_its_runtime_lands() {
    // `spec/31 §1` rule 10: the program is checked in full, and neither engine runs it, before
    // anything starts: no listener is bound, no executable is written.
    let port = free_port();
    let dir = package(
        "pending",
        &format!("    serve \"http://127.0.0.1:{port}\"\n"),
    );
    std::fs::write(dir.join("src/main.una"), SERVER).unwrap();
    let checked = una(&dir, &["check"]);
    assert!(checked.status.success(), "{}", stderr(&checked));
    let wanted = "`main.main` uses `HttpServer`, whose operations do not run in this release yet";
    let ran = una(&dir, &["run"]);
    assert_eq!(ran.status.code(), Some(2), "{}", stderr(&ran));
    assert!(stderr(&ran).contains(wanted), "{}", stderr(&ran));
    assert_eq!(stdout(&ran), "");
    let out = dir.join("server");
    let built = una(&dir, &["build", "--out", out.to_str().unwrap()]);
    assert_eq!(built.status.code(), Some(2), "{}", stderr(&built));
    assert!(stderr(&built).contains(wanted), "{}", stderr(&built));
    assert!(!out.exists());
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
