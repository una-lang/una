//! `install.sh` (#240): it installs only what the release's validation record vouches for, and
//! its pins follow the repository. The full installation, which downloads Z3, is tested by hand
//! in a clean container and a clean `HOME` (docs/releases/validation.md).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &str) -> String {
    std::fs::read_to_string(root().join(path)).unwrap()
}

/// The value of `NAME=value` at the start of a line of `install.sh`.
fn setting(script: &str, name: &str) -> String {
    script
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{name}=")))
        .unwrap()
        .to_owned()
}

#[test]
fn the_installer_pins_follow_the_repository() {
    let script = read("install.sh");
    let manifest = read("Cargo.toml");
    let version = manifest
        .lines()
        .find_map(|line| line.strip_prefix("version = \""))
        .unwrap()
        .trim_end_matches('"');
    assert_eq!(setting(&script, "DEFAULT_VERSION"), version);
    let lock = read("verification.lock");
    let solver = lock
        .lines()
        .find_map(|line| line.strip_prefix("solver_version"))
        .unwrap()
        .trim_start_matches([' ', '='])
        .trim_matches('"')
        .to_owned();
    assert_eq!(setting(&script, "Z3_VERSION"), solver);
    // Each Z3 asset and digest of the installer is the one scripts/install-z3.sh pins.
    let pins = read("scripts/install-z3.sh");
    let mut seen = 0;
    for line in script.lines() {
        for key in ["z3_asset=", "z3_sha256="] {
            if let Some(value) = line.trim().strip_prefix(key) {
                let value = value.trim_end_matches(" ;;");
                assert!(
                    pins.contains(value),
                    "{value} is not pinned in install-z3.sh"
                );
                seen += 1;
            }
        }
    }
    assert_eq!(seen, 6);
}

/// The archive's target on this machine, as the installer names it.
fn target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        other => panic!("no release for {other:?}"),
    }
}

fn sha256(path: &Path) -> String {
    let output = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .or_else(|_| Command::new("sha256sum").arg(path).output())
        .unwrap();
    String::from_utf8(output.stdout).unwrap()[..64].to_owned()
}

/// A release of `version` under `dir`: an archive holding a stub `una` and runtime, and a
/// validation record whose verdict is `verdict` and that lists `listed` as the archive's digest
/// (its own digest when `None`).
fn release(dir: &Path, version: &str, verdict: &str, listed: Option<&str>) {
    let _ = std::fs::remove_dir_all(dir);
    let name = format!("una-{version}-{}", target());
    let staging = dir.join("staging").join(&name);
    std::fs::create_dir_all(staging.join("lib")).unwrap();
    std::fs::write(staging.join("una"), "#!/bin/sh\necho una stub\n").unwrap();
    std::fs::write(staging.join("lib/libuna_rt.a"), "stub").unwrap();
    let releases = dir.join("releases").join(format!("v{version}"));
    std::fs::create_dir_all(&releases).unwrap();
    let archive = releases.join(format!("{name}.tar.gz"));
    let status = Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(dir.join("staging"))
        .arg(&name)
        .status()
        .unwrap();
    assert!(status.success());
    let digest = listed.map_or_else(|| sha256(&archive), str::to_owned);
    let records = dir
        .join("records")
        .join(format!("v{version}"))
        .join("docs/releases");
    std::fs::create_dir_all(&records).unwrap();
    std::fs::write(
        records.join(format!("{version}-validation.md")),
        format!(
            "# Validation record\n\n## Archives\n\n| Archive | sha256 |\n| --- | --- |\n| `{name}.tar.gz` | `{digest}` |\n\nVerdict: {verdict}\n"
        ),
    )
    .unwrap();
}

/// Runs the installer with `args` into `dir/home`, from the release under `dir`; no Z3 is
/// reachable, so a run that passes the checks of the archive stops at Z3.
fn install(dir: &Path, version: &str, args: &[&str]) -> Output {
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    Command::new("sh")
        .arg(root().join("install.sh"))
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("HOME", &home)
        .env("UNA_VERSION", version)
        .env(
            "UNA_RELEASES",
            format!("file://{}", dir.join("releases").display()),
        )
        .env(
            "UNA_RECORDS",
            format!("file://{}", dir.join("records").display()),
        )
        .env(
            "UNA_Z3_RELEASES",
            format!("file://{}", dir.join("no-z3").display()),
        )
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn the_installer_installs_nothing_that_the_validation_record_does_not_vouch_for() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install");
    let version = "9.9.9";
    let home = dir.join("home/.una");
    // A digest that is not the archive's: refused, and nothing installed.
    release(&dir, version, "PASS (every gate)", Some(&"0".repeat(64)));
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("SHA-256 mismatch"),
        "{}",
        stderr(&output)
    );
    assert!(!home.exists());
    // A record whose verdict is not PASS: refused.
    release(&dir, version, "FAIL (linux)", None);
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("is not a PASS of every gate"),
        "{}",
        stderr(&output)
    );
    assert!(!home.exists());
    // A record that vouches for the archive: the installer goes on, to Z3, which this test
    // does not serve, and still installs nothing.
    release(&dir, version, "PASS (every gate)", None);
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("cannot download z3-"),
        "{}",
        stderr(&output)
    );
    assert!(!home.exists());
    // Uninstall removes UNA_HOME, and nothing else.
    std::fs::create_dir_all(home.join("bin")).unwrap();
    let output = install(&dir, version, &["--uninstall"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(!home.exists() && dir.join("home").exists());
    assert_eq!(install(&dir, version, &["--bogus"]).status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&dir);
}
