//! `install.sh` (#240): it installs only what the release's validation summary vouches for, and
//! its pins follow the repository. It downloads nothing but the archive: the solver is built
//! into `una` (ADR 0124). It offers exactly the shipped targets, and on Linux it refuses musl and
//! a glibc below 2.39 before anything is downloaded (#1904), reads the C library from the running
//! system, refuses a 32-bit userland, and as root writes only where only root can (#1938), shown
//! here on fake machines. The full
//! installation is tested by hand in a clean container and a clean `HOME` (docs/release-gates.md).

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

/// The archive's target on this machine, as the installer names it, when it has releases:
/// macOS on arm64 and Linux on arm64 (#1904); Linux on x86_64 is not shipped in this release.
fn target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        _ => None,
    }
}

/// The shipped targets, read from `scripts/release-targets.txt` by the one reader the release
/// scripts use (#1903).
fn shipped_targets() -> Vec<String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(r#". "$1/scripts/release-targets.sh" && release_targets "$1/scripts/release-targets.txt""#)
        .arg("sh")
        .arg(root())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The installer offers exactly the shipped targets of `scripts/release-targets.txt` (#1903):
/// none that no release builds, and every one that a release builds; and the target this test
/// installs on this machine is one of them.
#[test]
fn the_installer_offers_exactly_the_shipped_targets() {
    let mut shipped = shipped_targets();
    let script = read("install.sh");
    let mut offered: Vec<&str> = script
        .split_whitespace()
        .filter_map(|word| word.strip_prefix("target="))
        .collect();
    assert!(!offered.is_empty(), "install.sh names no target");
    shipped.sort();
    offered.sort();
    assert_eq!(
        offered, shipped,
        "install.sh offers {offered:?}, scripts/release-targets.txt lists {shipped:?}"
    );
    if let Some(target) = target() {
        assert!(shipped.iter().any(|t| t == target), "{target}");
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

/// A release of `version` under `dir`: an archive holding a stub `una`, with the runtime built
/// in (ADR 0124 step 2), and, in the tree of its tag, a validation summary (#717) whose verdict
/// is `verdict` and that lists `listed` as the archive's digest (its own digest when `None`).
fn release(dir: &Path, target: &str, version: &str, verdict: &str, listed: Option<&str>) {
    release_holding(dir, target, version, verdict, listed, &[]);
}

/// [`release`], with the archive also holding `extra` files, by path and content: the
/// `lib/libuna_rt.a` of a release before ADR 0124 step 2, or an entry the installer refuses.
/// A content `symlink:<target>` makes the entry a symbolic link to `<target>`.
fn release_holding(
    dir: &Path,
    target: &str,
    version: &str,
    verdict: &str,
    listed: Option<&str>,
    extra: &[(&str, &str)],
) {
    // The release's own trees; an installation under `dir/home` stays, so that a release can be
    // installed over an earlier one.
    for tree in ["staging", "releases", "records"] {
        let _ = std::fs::remove_dir_all(dir.join(tree));
    }
    let name = format!("una-{version}-{target}");
    let staging = dir.join("staging").join(&name);
    std::fs::create_dir_all(&staging).unwrap();
    for (path, text) in extra {
        let path = staging.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        if let Some(target) = text.strip_prefix("symlink:") {
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, &path).unwrap();
        } else {
            std::fs::write(path, text).unwrap();
        }
    }
    std::fs::write(staging.join("una"), "#!/bin/sh\necho una stub\n").unwrap();
    // Executable, as in a real archive: the installer checks it before installing.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(staging.join("una"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }
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
    run_installer(
        &root().join("install.sh"),
        dir,
        version,
        args,
        SYSTEM_PATH,
        &[],
    )
}

const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// Runs `script` with `args` and `PATH` `path` into `dir/home`, from the release under `dir`, with
/// `envs` set beside the installer's settings.
fn run_installer(
    script: &Path,
    dir: &Path,
    version: &str,
    args: &[&str],
    path: &str,
    envs: &[(&str, &str)],
) -> Output {
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    // In `dir`, so that a relative UNA_HOME stays there.
    Command::new("sh")
        .current_dir(dir)
        .arg(script)
        .args(args)
        .env_clear()
        .env("PATH", path)
        .env("HOME", &home)
        .envs(envs.iter().copied())
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
    let _ = std::fs::remove_dir_all(&dir);
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
    assert!(home.join("bin/una").exists() && !home.join("lib").exists());
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
fn an_archive_without_lib_installs_no_runtime_and_an_earlier_one_still_installs_its_own() {
    // ADR 0124 step 2: the runtime is built into `una`, so a fresh installation holds `bin/una`
    // and the documents and no `lib/`. An archive of an earlier release, whose `una` reads its
    // runtime from `lib/libuna_rt.a`, still installs it; an archive holding anything else at
    // its top level, or more under `lib/`, is refused before anything is installed.
    let Some(target) = target() else {
        return;
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-no-lib");
    let _ = std::fs::remove_dir_all(&dir);
    let version = "9.9.7";
    let home = dir.join("home/.una");
    let pass = "PASS (every gate)";
    let documents = [
        ("README.md", "readme"),
        ("LICENSE", "license"),
        ("docs/guide/getting-started.md", "guide"),
    ];
    // A fresh installation from an archive without `lib/`.
    release_holding(&dir, target, version, pass, None, &documents);
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let record = std::fs::read_to_string(home.join(".una-install")).unwrap();
    assert_eq!(
        record,
        format!(
            "# Una {version}, installed by install.sh; `sh install.sh --uninstall` removes what follows.\nversion {version}\ndir .\ndir bin\ndir docs\ndir docs/guide\nfile bin/una\nfile README.md\nfile LICENSE\nfile docs/guide/getting-started.md\n"
        )
    );
    let mut held: Vec<String> = std::fs::read_dir(&home)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    held.sort();
    assert_eq!(
        held,
        [".una-install", "LICENSE", "README.md", "bin", "docs"]
    );
    std::fs::remove_dir_all(&home).unwrap();
    // An earlier release, with its runtime in `lib/`: installed with it, as before.
    let earlier = [("lib/libuna_rt.a", "runtime"), ("README.md", "readme")];
    release_holding(&dir, target, version, pass, None, &earlier);
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        std::fs::read_to_string(home.join("lib/libuna_rt.a")).unwrap(),
        "runtime"
    );
    // A later release over it: the earlier runtime, which the new `una` never reads, is left
    // for the migration of step 5, and stays in the record, so that uninstall removes it.
    release_holding(&dir, target, version, pass, None, &documents);
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let record = std::fs::read_to_string(home.join(".una-install")).unwrap();
    assert!(record.contains("file lib/libuna_rt.a\n"), "{record}");
    let output = install(&dir, version, &["--uninstall"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(!dir.join("home/.una").exists());
    // Refused: another program beside `una`, or more than the runtime under `lib/`.
    for extra in [
        [("z3", "#!/bin/sh\n"), ("README.md", "readme")],
        [
            ("lib/libuna_rt.a", "runtime"),
            ("lib/libz3.dylib", "library"),
        ],
    ] {
        release_holding(&dir, target, version, pass, None, &extra);
        let output = install(&dir, version, &[]);
        assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
        assert!(stderr(&output).contains("refuses"), "{}", stderr(&output));
        assert!(!home.exists());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn an_archive_with_a_link_is_refused_before_anything_is_installed() {
    // #1359 (L2): a symbolic link in the archive, at its top level or among the guides, could
    // make the installer copy a file the archive does not hold, such as one of the machine's,
    // into UNA_HOME. Every link is refused, wherever it is and wherever it leads, before
    // anything is unpacked or installed.
    let Some(target) = target() else {
        return;
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-links");
    let _ = std::fs::remove_dir_all(&dir);
    let version = "9.9.6";
    let home = dir.join("home/.una");
    let pass = "PASS (every gate)";
    for extra in [
        [("README.md", "symlink:/etc/hosts"), ("LICENSE", "license")],
        [("README.md", "readme"), ("LICENSE", "symlink:README.md")],
        [
            ("README.md", "readme"),
            ("docs/guide/getting-started.md", "symlink:/etc/hosts"),
        ],
        [
            ("lib/libuna_rt.a", "symlink:/etc/hosts"),
            ("README.md", "readme"),
        ],
    ] {
        release_holding(&dir, target, version, pass, None, &extra);
        let output = install(&dir, version, &[]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{extra:?}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains("holds a link"),
            "{extra:?}: {}",
            stderr(&output)
        );
        assert!(!home.exists(), "{extra:?}");
    }
    // The same archive without the link installs.
    release_holding(
        &dir,
        target,
        version,
        pass,
        None,
        &[("README.md", "readme"), ("LICENSE", "license")],
    );
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_repacked_archive_is_accepted_only_as_repacked_md_pairs_it() {
    let Some(target) = target() else {
        return;
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-repacked");
    let _ = std::fs::remove_dir_all(&dir);
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
    assert!(home.join("bin/una").exists() && !home.join("lib").exists());
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

#[test]
fn the_installer_refuses_a_version_before_0_4_0_comparing_numbers_not_text() {
    // A release before 0.4.0 needs a Z3 on PATH that the installer no longer installs (#1819):
    // refused before anything is downloaded, so no release exists under `dir` and nothing is
    // fetched, from the network or elsewhere.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-versions");
    let _ = std::fs::remove_dir_all(&dir);
    let home = dir.join("home/.una");
    for version in ["0.3.16", "0.3.99", "0.1.0", "0.0.9"] {
        let output = install(&dir, version, &[]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{version}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains(&format!(
                "this installer installs 0.4.0 and later; {version} needs a separate Z3"
            )),
            "{version}: {}",
            stderr(&output)
        );
        assert_eq!(stderr(&output).lines().count(), 1, "{}", stderr(&output));
        assert!(!home.exists(), "{version}");
    }
    // Anything but MAJOR.MINOR.PATCH is refused as well.
    for version in [
        "0.4",
        "0.4.0.1",
        "0.4.0-rc1",
        "v0.4.0",
        "0..4",
        ".0.4",
        "a.b.c",
    ] {
        let output = install(&dir, version, &[]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{version}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains("is not a version"),
            "{version}: {}",
            stderr(&output)
        );
        assert!(!home.exists(), "{version}");
    }
    // 0.4.0 and 0.10.0 pass the check: 0.10 is after 0.4 as numbers, though not as text.
    for version in ["0.4.0", "0.10.0", "1.0.0"] {
        if let Some(target) = target() {
            release(&dir, target, version, "PASS (every gate)", None);
            let output = install(&dir, version, &[]);
            assert_eq!(
                output.status.code(),
                Some(0),
                "{version}: {}",
                stderr(&output)
            );
            assert!(home.join("bin/una").exists(), "{version}");
            std::fs::remove_dir_all(&home).unwrap();
        } else {
            // No release for this platform: refused for the platform, not the version.
            let output = install(&dir, version, &[]);
            assert_eq!(output.status.code(), Some(1));
            assert!(
                !stderr(&output).contains("0.4.0 and later"),
                "{}",
                stderr(&output)
            );
            if (std::env::consts::OS, std::env::consts::ARCH) == ("linux", "x86_64") {
                assert!(
                    stderr(&output).contains("Linux on x86_64 is not shipped in this release"),
                    "{}",
                    stderr(&output)
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A machine the installer runs on, whatever machine runs the test (#1904): commands on a
/// directory put first on `PATH`. `uname -s` and `uname -m` print `os` and `arch`;
/// `getconf GNU_LIBC_VERSION` prints `getconf`, or fails as musl's and macOS's do when it is
/// `None`; `getconf LONG_BIT` prints `long_bit`, or fails when it is `None`; `ldd --version`
/// prints `ldd` (on stderr with status 1 when it names musl, as musl's does), or is not found
/// when it is `None`; `id -u` prints `uid`. `curl` and `wget`, when the machine has them, record
/// every call in `dir/fetches` and then run the real ones, so that a test sees whether anything
/// was downloaded.
///
/// The machine's loaders (#1938) are in a directory of the test, at which a copy of `install.sh`
/// points its one `loaders=/lib` line: `ld-linux-aarch64.so.1 --version` prints `loader` (on
/// stderr with status 1 when it names musl, as musl's loader does, or after a `fails:` prefix),
/// or is missing when it is `None`; `ld-musl-aarch64.so.1` is there when `musl_loader` is set.
struct Host<'a> {
    os: &'a str,
    arch: &'a str,
    getconf: Option<&'a str>,
    long_bit: Option<&'a str>,
    ldd: Option<&'a str>,
    loader: Option<&'a str>,
    musl_loader: bool,
    uid: &'a str,
}

/// What glibc's aarch64 loader answers to `--version` on Ubuntu 24.04 (its first line).
const GLIBC_LOADER: &str = "ld.so (Ubuntu GLIBC 2.39-0ubuntu8.9) stable release version 2.39.";
/// What musl's loader answers to `--version`, on stderr with status 1.
const MUSL_LOADER: &str = "musl libc (aarch64)\nVersion 1.2.5\nDynamic Program Loader";

const GLIBC_239: Host<'static> = Host {
    os: "Linux",
    arch: "aarch64",
    getconf: Some("glibc 2.39"),
    long_bit: Some("64"),
    ldd: Some("ldd (Ubuntu GLIBC 2.39-0ubuntu8.6) 2.39"),
    loader: Some(GLIBC_LOADER),
    musl_loader: false,
    uid: "1000",
};

fn script(path: &Path, text: &str) {
    std::fs::write(path, format!("#!/bin/sh\n{text}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// Runs the installer, as [`install`], on `host`; returns its output and the downloads it made.
fn install_on(dir: &Path, version: &str, host: &Host) -> (Output, Vec<String>) {
    install_on_with(dir, version, host, &[], &[])
}

/// [`install_on`], with `args` and the settings `envs`.
fn install_on_with(
    dir: &Path,
    version: &str,
    host: &Host,
    args: &[&str],
    envs: &[(&str, &str)],
) -> (Output, Vec<String>) {
    let lib = dir.join("fake-lib");
    let _ = std::fs::remove_dir_all(&lib);
    std::fs::create_dir_all(&lib).unwrap();
    if let Some(text) = host.loader {
        script(
            &lib.join("ld-linux-aarch64.so.1"),
            &if text.contains("musl") {
                format!("printf '{text}\\n' >&2; exit 1")
            } else if let Some(text) = text.strip_prefix("fails:") {
                format!("echo '{text}'; exit 1")
            } else {
                format!("[ \"$1\" = --version ] && echo '{text}'")
            },
        );
    }
    if host.musl_loader {
        script(&lib.join("ld-musl-aarch64.so.1"), "exit 1");
    }
    // install.sh with its one `loaders=/lib` line pointing at the fake machine's loaders; every
    // other line is the repository's.
    let original = read("install.sh");
    assert_eq!(
        original
            .lines()
            .filter(|line| *line == "loaders=/lib")
            .count(),
        1,
        "install.sh has no single `loaders=/lib` line"
    );
    let patched = original.replace(
        "\nloaders=/lib\n",
        &format!("\nloaders='{}'\n", lib.display()),
    );
    let copy = dir.join("install-on-host.sh");
    std::fs::write(&copy, patched).unwrap();
    let bin = dir.join("fake-bin");
    let _ = std::fs::remove_dir_all(&bin);
    std::fs::create_dir_all(&bin).unwrap();
    let fetches = dir.join("fetches");
    let _ = std::fs::remove_file(&fetches);
    script(
        &bin.join("uname"),
        &format!(
            "case $1 in -s) echo '{}' ;; -m) echo '{}' ;; *) exit 1 ;; esac",
            host.os, host.arch
        ),
    );
    let answer = |value: Option<&str>| match value {
        Some(answer) => format!("echo '{answer}'"),
        None => "echo \"getconf: $1: invalid argument\" >&2; exit 1".to_owned(),
    };
    script(
        &bin.join("getconf"),
        &format!(
            "case $1 in GNU_LIBC_VERSION) {} ;; LONG_BIT) {} ;; *) exit 1 ;; esac",
            answer(host.getconf),
            answer(host.long_bit)
        ),
    );
    script(
        &bin.join("ldd"),
        &match host.ldd {
            Some(text) if text.contains("musl") => format!("echo '{text}' >&2; exit 1"),
            Some(text) => format!("echo '{text}'"),
            None => "echo 'ldd: not found' >&2; exit 127".to_owned(),
        },
    );
    script(
        &bin.join("id"),
        &format!(
            "[ \"$1\" = -u ] && echo '{}' || PATH={SYSTEM_PATH} exec id \"$@\"",
            host.uid
        ),
    );
    for fetcher in ["curl", "wget"] {
        let found = Command::new("sh")
            .arg("-c")
            .arg(format!("PATH={SYSTEM_PATH} command -v {fetcher}"))
            .output()
            .unwrap()
            .status
            .success();
        if found {
            script(
                &bin.join(fetcher),
                &format!(
                    "echo \"{fetcher} $*\" >> '{}'\nPATH={SYSTEM_PATH} exec {fetcher} \"$@\"",
                    fetches.display()
                ),
            );
        }
    }
    let output = run_installer(
        &copy,
        dir,
        version,
        args,
        &format!("{}:{SYSTEM_PATH}", bin.display()),
        envs,
    );
    let fetched = std::fs::read_to_string(&fetches)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    (output, fetched)
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The `uname -s` and `uname -m` of a machine each shipped target installs on; a target added to
/// `scripts/release-targets.txt` needs its row here and its case in `install.sh`.
fn uname_of(target: &str) -> Option<(&'static str, &'static str)> {
    match target {
        "aarch64-apple-darwin" => Some(("Darwin", "arm64")),
        "aarch64-unknown-linux-gnu" => Some(("Linux", "aarch64")),
        _ => None,
    }
}

const CC_NOTE: &str = "una build needs a C compiler on PATH as cc";
const ROOT_NOTE: &str = "refuse to run as root";

#[test]
fn every_shipped_target_installs_on_its_machine_with_its_notes() {
    // Each target of scripts/release-targets.txt is the archive the installer picks on its
    // machine; Linux on arm64 also as `uname -m` says `arm64`. On Linux the installer says that
    // `una build` needs `cc` and that the proving commands refuse root; on macOS, whose `una`
    // links by itself, neither, unless the user is root.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-targets");
    let _ = std::fs::remove_dir_all(&dir);
    let version = "9.9.5";
    let home = dir.join("home/.una");
    let mut machines: Vec<(String, &str, &str)> = shipped_targets()
        .into_iter()
        .map(|target| {
            let machine = uname_of(&target);
            assert!(
                machine.is_some(),
                "no machine for {target}: add it to uname_of and to install.sh"
            );
            let (os, arch) = machine.unwrap();
            (target, os, arch)
        })
        .collect();
    machines.push(("aarch64-unknown-linux-gnu".to_owned(), "Linux", "arm64"));
    for (target, os, arch) in &machines {
        release(&dir, target, version, "PASS (every gate)", None);
        let host = Host {
            os,
            arch,
            ..GLIBC_239
        };
        let (output, fetched) = install_on(&dir, version, &host);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{os} {arch}: {}",
            stderr(&output)
        );
        assert!(home.join("bin/una").exists(), "{os} {arch}");
        assert!(
            fetched
                .iter()
                .any(|f| f.contains(&format!("una-{version}-{target}.tar.gz"))),
            "{os} {arch}: {fetched:?}"
        );
        let linux = *os == "Linux";
        for note in [CC_NOTE, ROOT_NOTE, "link the system's glibc dynamically"] {
            assert_eq!(
                stdout(&output).contains(note),
                linux,
                "{os} {arch}: {note}: {}",
                stdout(&output)
            );
        }
        std::fs::remove_dir_all(&home).unwrap();
    }
    // Root on macOS: told that the proving commands refuse it.
    release(
        &dir,
        "aarch64-apple-darwin",
        version,
        "PASS (every gate)",
        None,
    );
    let host = Host {
        os: "Darwin",
        arch: "arm64",
        uid: "0",
        ..GLIBC_239
    };
    let (output, _) = install_on(&dir, version, &host);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).contains(ROOT_NOTE), "{}", stdout(&output));
    assert!(!stdout(&output).contains(CC_NOTE), "{}", stdout(&output));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn linux_without_a_glibc_at_the_floor_is_refused_before_anything_is_downloaded() {
    // The Linux `una` is a dynamic glibc executable built on glibc 2.39 (#1904): musl, an older
    // glibc and a glibc whose version cannot be read are refused, and nothing is downloaded,
    // though a valid release for the machine is there to download. Each case runs as `uname -m`
    // says `aarch64` and as it says `arm64` (#1938).
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-glibc");
    let _ = std::fs::remove_dir_all(&dir);
    let version = "9.9.4";
    let home = dir.join("home/.una");
    let target = "aarch64-unknown-linux-gnu";
    release(&dir, target, version, "PASS (every gate)", None);
    let musl = "musl libc (aarch64)\nVersion 1.2.5\nDynamic Program Loader";
    let refused = [
        // Alpine: musl's ldd, and a getconf that does not know GNU_LIBC_VERSION.
        (None, Some(musl), "this system's C library is musl"),
        // musl with a compatibility getconf that answers like glibc's.
        (
            Some("glibc 2.39"),
            Some(musl),
            "this system's C library is musl",
        ),
        // No answer at all: never assumed to be glibc.
        (None, None, "cannot read this system's glibc version"),
        (
            None,
            Some("ldd (GNU libc) 2.39"),
            "cannot read this system's glibc version",
        ),
        (Some(""), None, "cannot read this system's glibc version"),
        (
            Some("glibc 2"),
            None,
            "cannot read this system's glibc version",
        ),
        (
            Some("glibc 2.x"),
            None,
            "cannot read this system's glibc version",
        ),
        (
            Some("glibc 2.39-rc"),
            None,
            "cannot read this system's glibc version",
        ),
        (
            Some("musl 1.2.5"),
            None,
            "cannot read this system's glibc version",
        ),
        // Below the floor, compared as numbers: 2.4 is before 2.39, though not as text.
        (Some("glibc 2.38"), None, "this system has glibc 2.38"),
        (Some("glibc 2.35"), None, "this system has glibc 2.35"),
        (Some("glibc 2.4"), None, "this system has glibc 2.4"),
        (Some("glibc 1.99"), None, "this system has glibc 1.99"),
    ];
    for ((getconf, ldd, why), arch) in refused
        .into_iter()
        .flat_map(|case| [(case, "aarch64"), (case, "arm64")])
    {
        let host = Host {
            arch,
            getconf,
            ldd,
            ..GLIBC_239
        };
        let (output, fetched) = install_on(&dir, version, &host);
        let case = format!("{arch} {getconf:?} {ldd:?}");
        assert_eq!(output.status.code(), Some(1), "{case}: {}", stderr(&output));
        assert!(stderr(&output).contains(why), "{case}: {}", stderr(&output));
        assert!(
            stderr(&output).contains(
                "needs glibc 2.39 or later; Alpine and other musl systems are not supported"
            ),
            "{case}: {}",
            stderr(&output)
        );
        assert!(fetched.is_empty(), "{case}: {fetched:?}");
        assert!(!home.exists(), "{case}");
    }
    // At the floor and above, compared as numbers: installed.
    for (getconf, arch) in [
        "glibc 2.39",
        "glibc 2.40",
        "glibc 2.100",
        "glibc 2.40.9000",
        "glibc 3.0",
    ]
    .into_iter()
    .flat_map(|getconf| [(getconf, "aarch64"), (getconf, "arm64")])
    {
        let host = Host {
            arch,
            getconf: Some(getconf),
            ..GLIBC_239
        };
        let (output, fetched) = install_on(&dir, version, &host);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{arch} {getconf}: {}",
            stderr(&output)
        );
        assert!(!fetched.is_empty(), "{arch} {getconf}");
        std::fs::remove_dir_all(&home).unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_machine_without_a_release_is_refused_before_anything_is_downloaded() {
    // Linux on x86_64 is not shipped in this release (#1904), and no other system or processor
    // has a release: refused with the reason, and nothing downloaded, though an archive under
    // the name of each target is there.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-machines");
    let _ = std::fs::remove_dir_all(&dir);
    let version = "9.9.3";
    let home = dir.join("home/.una");
    let shipped = "the releases are for macOS on arm64 and Linux on arm64 (aarch64)";
    for (os, arch, why, target) in [
        (
            "Linux",
            "x86_64",
            "Linux on x86_64 is not shipped in this release",
            "x86_64-unknown-linux-gnu",
        ),
        (
            "Linux",
            "amd64",
            "Linux on x86_64 is not shipped in this release",
            "x86_64-unknown-linux-gnu",
        ),
        (
            "Linux",
            "armv7l",
            "no Una release for Linux armv7l",
            "armv7-unknown-linux-gnueabihf",
        ),
        (
            "Linux",
            "riscv64",
            "no Una release for Linux riscv64",
            "riscv64gc-unknown-linux-gnu",
        ),
        (
            "Darwin",
            "x86_64",
            "no Una release for Darwin x86_64",
            "x86_64-apple-darwin",
        ),
        (
            "FreeBSD",
            "arm64",
            "no Una release for FreeBSD arm64",
            "aarch64-unknown-freebsd",
        ),
    ] {
        release(&dir, target, version, "PASS (every gate)", None);
        let host = Host {
            os,
            arch,
            ..GLIBC_239
        };
        let (output, fetched) = install_on(&dir, version, &host);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{os} {arch}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains(why),
            "{os} {arch}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains(shipped),
            "{os} {arch}: {}",
            stderr(&output)
        );
        assert!(fetched.is_empty(), "{os} {arch}: {fetched:?}");
        assert!(!home.exists(), "{os} {arch}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Asserts that `output` is a refusal for `why` with exit code 1, that nothing was downloaded and
/// that `home` was not created.
fn assert_refused(case: &str, output: &Output, fetched: &[String], why: &str, home: &Path) {
    assert_eq!(output.status.code(), Some(1), "{case}: {}", stderr(output));
    assert!(stderr(output).contains(why), "{case}: {}", stderr(output));
    assert!(fetched.is_empty(), "{case}: {fetched:?}");
    assert!(!home.exists(), "{case}");
}

#[test]
fn the_c_library_is_the_one_the_system_runs_not_a_file_beside_it() {
    // #1938: a glibc system that also has musl's loader installed (Debian's `musl` package) is
    // glibc, and installs. musl is still refused: by its `ldd`, by musl's loader when getconf
    // shows no glibc, and when the aarch64 loader `una` needs is missing, is musl's (gcompat) or
    // does not answer as glibc's. Each case runs as `uname -m` says `aarch64` and `arm64`.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-loader");
    let _ = std::fs::remove_dir_all(&dir);
    let version = "9.9.2";
    let home = dir.join("home/.una");
    release(
        &dir,
        "aarch64-unknown-linux-gnu",
        version,
        "PASS (every gate)",
        None,
    );
    let needs = "needs glibc 2.39 or later; Alpine and other musl systems are not supported";
    for arch in ["aarch64", "arm64"] {
        let host = Host {
            arch,
            musl_loader: true,
            ..GLIBC_239
        };
        let (output, fetched) = install_on(&dir, version, &host);
        assert_eq!(output.status.code(), Some(0), "{arch}: {}", stderr(&output));
        assert!(
            fetched
                .iter()
                .any(|f| f.contains("aarch64-unknown-linux-gnu.tar.gz")),
            "{arch}: {fetched:?}"
        );
        std::fs::remove_dir_all(&home).unwrap();
        let refused = [
            (
                "musl's loader, no getconf answer",
                Host {
                    arch,
                    getconf: None,
                    ldd: None,
                    musl_loader: true,
                    ..GLIBC_239
                },
                "this system's C library is musl: getconf GNU_LIBC_VERSION shows no glibc, and musl's loader is ",
            ),
            (
                "gcompat: musl's loader at glibc's path",
                Host {
                    arch,
                    ldd: None,
                    loader: Some(MUSL_LOADER),
                    musl_loader: true,
                    ..GLIBC_239
                },
                "/ld-linux-aarch64.so.1 is musl's loader",
            ),
            (
                "no aarch64 loader",
                Host {
                    arch,
                    loader: None,
                    ..GLIBC_239
                },
                "does not answer as glibc's",
            ),
            (
                "glibc's answer, but a failure",
                Host {
                    arch,
                    loader: Some("fails:ld.so (GNU libc) stable release version 2.39."),
                    ..GLIBC_239
                },
                "does not answer as glibc's",
            ),
            (
                "another loader",
                Host {
                    arch,
                    loader: Some("ld.so (another libc) release 1.0"),
                    ..GLIBC_239
                },
                "does not answer as glibc's",
            ),
        ];
        for (case, host, why) in refused {
            let (output, fetched) = install_on(&dir, version, &host);
            let case = format!("{arch} {case}");
            assert_refused(&case, &output, &fetched, why, &home);
            assert!(
                stderr(&output).contains(needs),
                "{case}: {}",
                stderr(&output)
            );
        }
        // Without musl's loader, the same machine without a getconf answer is not named musl.
        let host = Host {
            arch,
            getconf: None,
            ldd: None,
            ..GLIBC_239
        };
        let (output, fetched) = install_on(&dir, version, &host);
        assert_refused(
            arch,
            &output,
            &fetched,
            "cannot read this system's glibc version",
            &home,
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_32_bit_userland_is_refused_before_anything_is_downloaded() {
    // #1938: a 32-bit Arm userland on a 64-bit kernel reports aarch64, but cannot start the
    // aarch64 `una`: `getconf LONG_BIT` must say 64, and an answer that is not exactly 64, or
    // none, is refused, though the glibc and its loader are fine.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-userland");
    let _ = std::fs::remove_dir_all(&dir);
    let version = "9.9.1";
    let home = dir.join("home/.una");
    release(
        &dir,
        "aarch64-unknown-linux-gnu",
        version,
        "PASS (every gate)",
        None,
    );
    for arch in ["aarch64", "arm64"] {
        for long_bit in [
            Some("32"),
            None,
            Some(""),
            Some("64 "),
            Some("064"),
            Some("128"),
        ] {
            let host = Host {
                arch,
                long_bit,
                ..GLIBC_239
            };
            let (output, fetched) = install_on(&dir, version, &host);
            assert_refused(
                &format!("{arch} {long_bit:?}"),
                &output,
                &fetched,
                "this system's userland is not 64-bit (getconf LONG_BIT",
                &home,
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A directory owned by neither root nor this test's user and writable by neither its group nor
/// others, when the machine has one, for the owner check as root.
fn a_directory_of_another_user() -> Option<PathBuf> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(
            "find /private/var/ /var/ /usr/ /opt/ -maxdepth 2 -type d ! -user 0 ! -user \"$(id -u)\" \
             ! -perm -0020 ! -perm -0002 2> /dev/null | head -n 1",
        )
        .output()
        .unwrap();
    let found = String::from_utf8(output.stdout).unwrap();
    let found = found.trim_end_matches('\n');
    (!found.is_empty()).then(|| PathBuf::from(found))
}

#[cfg(unix)]
#[test]
fn as_root_the_installer_writes_only_where_only_root_can_write() {
    // #1938: as root, a link or a directory that another user can change, in UNA_HOME or above
    // it, could redirect root's writes or removals outside UNA_HOME. The installer refuses each
    // before anything is downloaded or removed, follows no link, and never runs the installed
    // `una`. `id -u` says 0 here, and the owner check takes this process's own user as root's
    // stand-in ([ -O ]), as it is root's own when the installer really runs as root.
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-root");
    let _ = std::fs::remove_dir_all(&dir);
    let version = "9.9.0";
    let home = dir.join("home/.una");
    let elsewhere = dir.join("elsewhere");
    let target = "aarch64-unknown-linux-gnu";
    release(&dir, target, version, "PASS (every gate)", None);
    let root = Host {
        uid: "0",
        ..GLIBC_239
    };
    let mode = |path: &Path, mode: u32| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    };
    let reset = || {
        for path in [dir.join("home"), elsewhere.clone()] {
            if let Ok(meta) = std::fs::symlink_metadata(&path) {
                if meta.file_type().is_symlink() {
                    std::fs::remove_file(&path).unwrap();
                } else {
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                        .unwrap();
                    std::fs::remove_dir_all(&path).unwrap();
                }
            }
        }
        std::fs::create_dir_all(dir.join("home")).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
    };
    // An ordinary user sees `una --version`; root installs, but does not run it.
    reset();
    let (output, _) = install_on(&dir, version, &GLIBC_239);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).contains("una stub"), "{}", stdout(&output));
    reset();
    let (output, fetched) = install_on(&dir, version, &root);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(home.join("bin/una").exists() && !fetched.is_empty());
    assert!(!stdout(&output).contains("una stub"), "{}", stdout(&output));
    assert!(
        stdout(&output).contains("as root, the installer does not run the installed una"),
        "{}",
        stdout(&output)
    );
    assert!(stdout(&output).contains(ROOT_NOTE), "{}", stdout(&output));
    let link = "does not follow the symbolic link";
    let writable = "must be writable only by its owner, root";
    type Setup<'a> = Box<dyn Fn() + 'a>;
    let cases: Vec<(&str, Setup, &str)> = vec![
        (
            "bin is a link",
            Box::new(|| {
                std::fs::create_dir_all(&home).unwrap();
                symlink(&elsewhere, home.join("bin")).unwrap();
            }),
            link,
        ),
        (
            "UNA_HOME is a link",
            Box::new(|| symlink(&elsewhere, &home).unwrap()),
            link,
        ),
        (
            "bin/una is a link",
            Box::new(|| {
                std::fs::create_dir_all(home.join("bin")).unwrap();
                symlink(elsewhere.join("una"), home.join("bin/una")).unwrap();
            }),
            link,
        ),
        (
            "the record is a link",
            Box::new(|| {
                std::fs::create_dir_all(&home).unwrap();
                symlink(elsewhere.join("record"), home.join(".una-install")).unwrap();
            }),
            link,
        ),
        (
            "a guide is a link",
            Box::new(|| {
                std::fs::create_dir_all(home.join("docs/guide")).unwrap();
                symlink(
                    elsewhere.join("g.md"),
                    home.join("docs/guide/getting-started.md"),
                )
                .unwrap();
            }),
            "docs/guide holds a symbolic link",
        ),
        (
            "UNA_HOME writable by its group",
            Box::new(|| {
                std::fs::create_dir_all(&home).unwrap();
                mode(&home, 0o775);
            }),
            writable,
        ),
        (
            "bin writable by others",
            Box::new(|| {
                std::fs::create_dir_all(home.join("bin")).unwrap();
                mode(&home.join("bin"), 0o777);
            }),
            writable,
        ),
        (
            "the directory UNA_HOME is created in is writable by others, though sticky",
            Box::new(|| mode(&dir.join("home"), 0o1777)),
            writable,
        ),
        (
            "a directory above writable by others, not sticky",
            Box::new(|| {
                std::fs::create_dir_all(home.join("bin")).unwrap();
                mode(&dir.join("home"), 0o777);
            }),
            writable,
        ),
    ];
    for (case, setup, why) in &cases {
        reset();
        setup();
        let (output, fetched) = install_on(&dir, version, &root);
        assert_eq!(output.status.code(), Some(1), "{case}: {}", stderr(&output));
        assert!(stderr(&output).contains(why), "{case}: {}", stderr(&output));
        assert!(fetched.is_empty(), "{case}: {fetched:?}");
        assert!(!home.join("bin/una").exists(), "{case}");
        assert_eq!(
            std::fs::read_dir(&elsewhere).unwrap().count(),
            0,
            "{case}: written outside UNA_HOME"
        );
    }
    // A sticky directory above the one UNA_HOME is created in, as /tmp is: installed.
    reset();
    let shared = dir.join("home/shared");
    std::fs::create_dir_all(shared.join("inner")).unwrap();
    mode(&shared, 0o1777);
    let inner = shared.join("inner/.una");
    let inner_home = inner.to_str().unwrap();
    let (output, _) = install_on_with(&dir, version, &root, &[], &[("UNA_HOME", inner_home)]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(inner.join("bin/una").exists());
    // Not sticky: refused.
    std::fs::remove_dir_all(&inner).unwrap();
    mode(&shared, 0o777);
    let (output, fetched) = install_on_with(&dir, version, &root, &[], &[("UNA_HOME", inner_home)]);
    assert_refused("shared, not sticky", &output, &fetched, writable, &inner);
    // UNA_HOME relative, or with a `..` component.
    reset();
    let dotted = format!("{}/home/../home/.una", dir.display());
    for (una_home, why) in [
        ("una-home", "UNA_HOME must be an absolute path"),
        (
            dotted.as_str(),
            "UNA_HOME must have no '.' or '..' component",
        ),
    ] {
        let (output, fetched) =
            install_on_with(&dir, version, &root, &[], &[("UNA_HOME", una_home)]);
        assert_refused(una_home, &output, &fetched, why, &home);
        assert!(!dir.join("una-home").exists(), "{una_home}");
    }
    // A directory of another user, when this machine has one: refused for its owner.
    if let Some(theirs) = a_directory_of_another_user() {
        let una_home = theirs.join(".una");
        let (output, fetched) = install_on_with(
            &dir,
            version,
            &root,
            &[],
            &[("UNA_HOME", una_home.to_str().unwrap())],
        );
        assert_refused(
            &una_home.display().to_string(),
            &output,
            &fetched,
            "must be owned by root, not by uid",
            &una_home,
        );
    }
    // Uninstall as root: a recorded directory that is a link is refused before anything is
    // removed, and what it leads to is kept.
    reset();
    let (output, _) = install_on(&dir, version, &GLIBC_239);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    std::fs::create_dir_all(elsewhere.join("z3/bin")).unwrap();
    std::fs::write(elsewhere.join("z3/bin/z3"), "kept").unwrap();
    symlink(elsewhere.join("z3"), home.join("z3")).unwrap();
    let record = home.join(".una-install");
    let text = std::fs::read_to_string(&record).unwrap();
    // Recorded as a file only, and as directories with the file.
    for extra in ["file z3/bin/z3\n", "dir z3\ndir z3/bin\nfile z3/bin/z3\n"] {
        std::fs::write(&record, format!("{text}{extra}")).unwrap();
        let (output, _) = install_on_with(&dir, version, &root, &["--uninstall"], &[]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{extra}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains(link),
            "{extra}: {}",
            stderr(&output)
        );
        assert!(elsewhere.join("z3/bin/z3").exists() && home.join("bin/una").exists());
    }
    // Without the link, root uninstalls.
    std::fs::remove_file(home.join("z3")).unwrap();
    std::fs::write(&record, text).unwrap();
    let (output, _) = install_on_with(&dir, version, &root, &["--uninstall"], &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(!home.exists());
    reset();
    let _ = std::fs::remove_dir_all(&dir);
}
