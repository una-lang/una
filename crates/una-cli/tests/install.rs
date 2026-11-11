//! `install.sh` (#240): it installs only what the release's validation summary vouches for, and
//! its pins follow the repository. It downloads nothing but the archive: the solver is built
//! into `una` (ADR 0124). It offers exactly the shipped targets, and on Linux it refuses musl and
//! a glibc below 2.39 before anything is downloaded (#1904), shown here on fake machines. The full
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
    install_with_path(dir, version, args, SYSTEM_PATH)
}

const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

fn install_with_path(dir: &Path, version: &str, args: &[&str], path: &str) -> Output {
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    Command::new("sh")
        .arg(root().join("install.sh"))
        .args(args)
        .env_clear()
        .env("PATH", path)
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
/// `None`; `ldd --version` prints `ldd` (on stderr with status 1 when it names musl, as musl's
/// does), or is not found when it is `None`; `id -u` prints `uid`. `curl` and `wget`, when the
/// machine has them, record every call in `dir/fetches` and then run the real ones, so that a
/// test sees whether anything was downloaded.
struct Host<'a> {
    os: &'a str,
    arch: &'a str,
    getconf: Option<&'a str>,
    ldd: Option<&'a str>,
    uid: &'a str,
}

const GLIBC_239: Host<'static> = Host {
    os: "Linux",
    arch: "aarch64",
    getconf: Some("glibc 2.39"),
    ldd: Some("ldd (Ubuntu GLIBC 2.39-0ubuntu8.6) 2.39"),
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
    script(
        &bin.join("getconf"),
        &match host.getconf {
            Some(answer) => format!("[ \"$1\" = GNU_LIBC_VERSION ] && echo '{answer}'"),
            None => "echo \"getconf: $1: invalid argument\" >&2; exit 1".to_owned(),
        },
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
    let output = install_with_path(
        dir,
        version,
        &[],
        &format!("{}:{SYSTEM_PATH}", bin.display()),
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
    // though a valid release for the machine is there to download.
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
    for (getconf, ldd, why) in refused {
        let host = Host {
            getconf,
            ldd,
            ..GLIBC_239
        };
        let (output, fetched) = install_on(&dir, version, &host);
        let case = format!("{getconf:?} {ldd:?}");
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
    for getconf in [
        "glibc 2.39",
        "glibc 2.40",
        "glibc 2.100",
        "glibc 2.40.9000",
        "glibc 3.0",
    ] {
        let host = Host {
            getconf: Some(getconf),
            ..GLIBC_239
        };
        let (output, fetched) = install_on(&dir, version, &host);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{getconf}: {}",
            stderr(&output)
        );
        assert!(!fetched.is_empty(), "{getconf}");
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
