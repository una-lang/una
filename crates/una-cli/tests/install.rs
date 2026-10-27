//! `install.sh` (#240): it installs only what the release's validation summary vouches for, and
//! its pins follow the repository. It downloads nothing but the archive: the solver is built
//! into `una` (ADR 0124). The full installation is tested by hand in a clean container and a
//! clean `HOME` (docs/release-gates.md).

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
    // Nothing but the archive is downloaded: the installer names no Z3 asset, digest or
    // release (ADR 0124 rule 32).
    for forbidden in [
        "z3_asset",
        "z3_sha256",
        "Z3_VERSION",
        "UNA_Z3_RELEASES",
        "unzip",
    ] {
        assert!(
            !script.contains(forbidden),
            "install.sh still has {forbidden}"
        );
    }
}

/// The archive's target on this machine, as the installer names it, when it has releases.
fn target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        _ => None,
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

/// A release of `version` under `dir`: an archive holding a stub `una` and runtime, and, in the
/// tree of its tag, a validation summary (#717) whose verdict is `verdict` and that lists
/// `listed` as the archive's digest (its own digest when `None`).
fn release(dir: &Path, target: &str, version: &str, verdict: &str, listed: Option<&str>) {
    let _ = std::fs::remove_dir_all(dir);
    let name = format!("una-{version}-{target}");
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
        .join("docs/validation");
    std::fs::create_dir_all(&records).unwrap();
    std::fs::write(
        records.join(format!("{version}.md")),
        format!(
            "# Una v{version}: release validation\n\n## Archives\n\n| Archive | sha256 |\n| --- | --- |\n| `{name}.tar.gz` | `{digest}` |\n\nVerdict: {verdict}\n"
        ),
    )
    .unwrap();
}

/// Runs the installer with `args` into `dir/home`, from the release under `dir`.
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
fn the_installer_installs_nothing_that_the_validation_summary_does_not_vouch_for() {
    // The installer has no release for another platform.
    let Some(target) = target() else {
        return;
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install");
    let version = "9.9.9";
    let home = dir.join("home/.una");
    // A digest that is not the archive's: refused, and nothing installed.
    release(
        &dir,
        target,
        version,
        "PASS (every gate)",
        Some(&"0".repeat(64)),
    );
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("SHA-256 mismatch"),
        "{}",
        stderr(&output)
    );
    assert!(!home.exists());
    // A summary whose verdict is not PASS: refused.
    release(&dir, target, version, "FAIL (linux)", None);
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("is not a PASS of every gate"),
        "{}",
        stderr(&output)
    );
    assert!(!home.exists());
    // A summary that vouches for the archive: the installer installs it, and nothing else.
    release(&dir, target, version, "PASS (every gate)", None);
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(home.join("bin/una").exists() && home.join("lib/libuna_rt.a").exists());
    assert!(!home.join("z3").exists() && !home.join("bin/z3").exists());
    assert!(
        !std::fs::read_to_string(home.join(".una-install"))
            .unwrap()
            .contains("z3")
    );
    std::fs::remove_dir_all(&home).unwrap();
    // A release tagged before summaries were committed: its summary is read from main.
    let tag = dir.join("records").join(format!("v{version}"));
    let main = dir.join("records/main");
    std::fs::rename(&tag, &main).unwrap();
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(home.join("bin/una").exists());
    std::fs::remove_dir_all(&home).unwrap();
    // No summary anywhere, as for 0.1.x: refused on every platform, with a message that says
    // why and what to do instead.
    std::fs::remove_dir_all(&main).unwrap();
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("no validation summary for 9.9.9"),
        "{}",
        stderr(&output)
    );
    assert!(!home.exists());
    // Without the record of an installation, uninstall refuses and removes nothing, whatever
    // UNA_HOME holds.
    std::fs::create_dir_all(home.join("bin")).unwrap();
    std::fs::write(home.join("bin/tool"), "mine").unwrap();
    let output = install(&dir, version, &["--uninstall"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("no record of an installation"),
        "{}",
        stderr(&output)
    );
    assert!(home.join("bin/tool").exists());
    // With the record, it removes the files listed there, and the directories the
    // installation created when they are empty: `lib` and `z3`, not `bin`, which holds a file
    // of the user's, and not UNA_HOME, which existed before.
    for file in ["bin/una", "lib/libuna_rt.a", "z3/bin/z3", "README.md"] {
        std::fs::create_dir_all(home.join(file).parent().unwrap()).unwrap();
        std::fs::write(home.join(file), "una").unwrap();
    }
    std::fs::write(
        home.join(".una-install"),
        "# Una 9.9.9, installed by install.sh\nversion 9.9.9\ndir lib\ndir z3\ndir z3/bin\nfile bin/una\nfile bin/z3\nfile lib/libuna_rt.a\nfile z3/bin/z3\nfile README.md\n",
    )
    .unwrap();
    let output = install(&dir, version, &["--uninstall"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(home.join("bin/tool").exists());
    for gone in ["bin/una", "lib", "z3", "README.md", ".una-install"] {
        assert!(!home.join(gone).exists(), "{gone}");
    }
    // A record that names a path outside UNA_HOME is refused before anything is removed.
    std::fs::write(home.join("README.md"), "una").unwrap();
    std::fs::write(
        home.join(".una-install"),
        "version 9.9.9\nfile README.md\nfile ../escape\n",
    )
    .unwrap();
    std::fs::write(dir.join("home/escape"), "kept").unwrap();
    let output = install(&dir, version, &["--uninstall"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(dir.join("home/escape").exists() && home.join("README.md").exists());
    assert_eq!(install(&dir, version, &["--bogus"]).status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_repacked_archive_is_accepted_only_as_repacked_md_pairs_it() {
    let Some(target) = target() else {
        return;
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-repacked");
    let version = "9.9.8";
    let home = dir.join("home/.una");
    let name = format!("una-{version}-{target}.tar.gz");
    let gated = "1".repeat(64);
    // The summary lists the gated digest, which is not the published archive's.
    release(&dir, target, version, "PASS (every gate)", Some(&gated));
    let published = sha256(&dir.join("releases").join(format!("v{version}")).join(&name));
    let repacked = dir
        .join("records")
        .join(format!("v{version}"))
        .join("docs/validation/repacked.md");
    let table = |rows: &[(&str, &str, &str)]| {
        let mut text = String::from(
            "# Repacked\n\n| Archive | Gated sha256 | Published sha256 | Check |\n| --- | --- | --- | --- |\n",
        );
        for (archive, from, to) in rows {
            text.push_str(&format!(
                "| `{archive}` | `{from}` | `{to}` | identical |\n"
            ));
        }
        std::fs::write(&repacked, text).unwrap();
    };
    // A pair for this archive and this summary digest: accepted, and the archive installs.
    table(&[(&name, &gated, &published)]);
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(home.join("bin/una").exists() && home.join("lib/libuna_rt.a").exists());
    assert!(!home.join("z3").exists() && !home.join("bin/z3").exists());
    assert!(
        !std::fs::read_to_string(home.join(".una-install"))
            .unwrap()
            .contains("z3")
    );
    std::fs::remove_dir_all(&home).unwrap();
    // A pair whose published digest is not the download's: refused.
    table(&[(&name, &gated, &"2".repeat(64))]);
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("SHA-256 mismatch"),
        "{}",
        stderr(&output)
    );
    // A pair for another archive: refused.
    let other = format!("una-{version}-another-target.tar.gz");
    table(&[(&other, &gated, &published)]);
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("SHA-256 mismatch"),
        "{}",
        stderr(&output)
    );
    // A pair for another summary digest: refused.
    table(&[(&name, &"3".repeat(64), &published)]);
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("SHA-256 mismatch"),
        "{}",
        stderr(&output)
    );
    // A withdrawn archive, which the release does not publish: said so.
    std::fs::remove_file(dir.join("releases").join(format!("v{version}")).join(&name)).unwrap();
    std::fs::write(
        &repacked,
        format!("## Withdrawn\n\n| Archive |\n| --- |\n| `{name}` |\n"),
    )
    .unwrap();
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("was withdrawn"),
        "{}",
        stderr(&output)
    );
    assert!(!home.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
