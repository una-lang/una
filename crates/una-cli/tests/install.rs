//! `install.sh` (#240): it installs only what the release's validation summary vouches for, and
//! its pins follow the repository. It downloads nothing but the archive: the solver is built
//! into `una` (ADR 0124). It offers exactly the shipped targets, and on Linux it refuses musl and
//! a glibc below the floor (2.28, and 2.39 for the releases 0.4.0 to 0.5.1, #2105) before anything
//! is downloaded (#1904), reads the C library from the running
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
    // The public install command reads `install.sh` from `main`, where the release pull request
    // merges hours before its tag exists (#2101): the default is the workspace version, or the
    // release before it until the pull request that follows the tag moves it.
    let changelog = read("CHANGELOG.md");
    let before = changelog
        .lines()
        .filter_map(|line| line.strip_prefix("## v"))
        .find(|released| *released != version)
        .unwrap();
    let default = setting(&script, "DEFAULT_VERSION");
    assert!(
        default == version || default == before,
        "install.sh installs {default} by default: neither {version} nor {before}"
    );
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

/// The names in `envs` that set up a test rather than the installer, and are not passed on:
/// `UMASK`, and, for [`install_on_with`], `DEFAULT_TMP`, the directory that stands for `/tmp`,
/// and `FETCH_HOOK`; so are the names `HOOK_<tool>`.
const TEST_SEAMS: [&str; 3] = ["UMASK", "DEFAULT_TMP", "FETCH_HOOK"];

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
    // In `dir`, so that a relative UNA_HOME stays there, under the umask that `envs` names as
    // `UMASK` (022 by default), which is not passed on as a setting.
    let umask = envs
        .iter()
        .find(|(name, _)| *name == "UMASK")
        .map_or("022", |(_, value)| value);
    Command::new("sh")
        .current_dir(dir)
        .arg("-c")
        .arg(r#"umask "$1" && shift && exec sh "$@""#)
        .arg("sh")
        .arg(umask)
        .arg(script)
        .args(args)
        .env_clear()
        .env("PATH", path)
        .env("HOME", &home)
        .envs(
            envs.iter()
                .copied()
                .filter(|(name, _)| !TEST_SEAMS.contains(name) && !name.starts_with("HOOK_")),
        )
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
fn the_installer_installs_a_release_that_the_owner_accepted_in_its_summary() {
    // The summary is read from the tag's tree, so the release cannot replace it: a verdict line
    // `ACCEPTED BY THE OWNER` is the owner's recorded decision for a release whose gates did not
    // all pass on its commit. The archive's digest is still the one the summary lists.
    let Some(target) = target() else {
        return;
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-accepted");
    let _ = std::fs::remove_dir_all(&dir);
    let version = "9.9.8";
    let home = dir.join("home/.una");
    // Accepted, but with a digest that is not the archive's: refused, nothing installed.
    release(
        &dir,
        target,
        version,
        "ACCEPTED BY THE OWNER",
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
    // A verdict that only starts like the accepted one: refused.
    release(&dir, target, version, "ACCEPTED BY THE OWNERS", None);
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(!home.exists());
    // Accepted, with the archive's digest: installed.
    release(&dir, target, version, "ACCEPTED BY THE OWNER", None);
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(home.join("bin/una").exists());
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
    // No summary anywhere, as for a version not released: refused on every platform, with a
    // message that says why and what to check.
    std::fs::remove_dir_all(&main).unwrap();
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("no validation summary for 9.9.9 could be read")
            && stderr(&output).contains("check that 9.9.9 is released")
            && !stderr(&output).contains("0.1.x"),
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
    // A later release over it: the earlier runtime, which the new `una` never reads, is removed
    // with the `lib/` it was in (ADR 0124 rule 32, #1992), and neither stays in the record.
    release_holding(&dir, target, version, pass, None, &documents);
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(!home.join("lib").exists(), "{}", stdout(&output));
    let record = std::fs::read_to_string(home.join(".una-install")).unwrap();
    assert!(!record.contains("lib"), "{record}");
    assert!(
        stdout(&output).contains(&format!(
            "removed what the earlier installation held and {version} does not: lib/libuna_rt.a\n"
        )),
        "{}",
        stdout(&output)
    );
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

/// An installation by the installer of v0.3.16, as it leaves `dir/home/.una` (the evidence of
/// #1882): `bin/una`, `bin/z3` as a link to `../z3/bin/z3`, the `z3` it names, the runtime in
/// `lib/`, the documents and its record.
#[cfg(unix)]
fn installed_by_v0_3_16(dir: &Path) -> PathBuf {
    let home = dir.join("home/.una");
    for (file, text) in [
        ("bin/una", "una 0.3.16"),
        ("z3/bin/z3", "z3 5.1.0"),
        ("lib/libuna_rt.a", "runtime"),
        ("README.md", "readme"),
        ("LICENSE", "license"),
        ("docs/guide/getting-started.md", "guide"),
        (
            "docs/guide/dropped.md",
            "a guide the new release does not ship",
        ),
    ] {
        std::fs::create_dir_all(home.join(file).parent().unwrap()).unwrap();
        std::fs::write(home.join(file), text).unwrap();
    }
    std::os::unix::fs::symlink("../z3/bin/z3", home.join("bin/z3")).unwrap();
    std::fs::write(
        home.join(".una-install"),
        "# Una 0.3.16, installed by install.sh; `sh install.sh --uninstall` removes what follows.\n\
         version 0.3.16\ndir .\ndir bin\ndir docs\ndir docs/guide\ndir lib\ndir z3\ndir z3/bin\n\
         file bin/una\nfile bin/z3\nfile lib/libuna_rt.a\nfile z3/bin/z3\nfile README.md\n\
         file LICENSE\nfile docs/guide/getting-started.md\nfile docs/guide/dropped.md\n",
    )
    .unwrap();
    home
}

#[cfg(unix)]
#[test]
fn an_upgrade_removes_what_the_new_release_does_not_hold_and_uninstall_then_leaves_nothing() {
    // #1992, ADR 0124 rule 32: installed over v0.3.16, the installer removes the files of the
    // earlier record that the new installation does not hold (`bin/z3`, a link, and the `z3` it
    // names; the runtime in `lib/`; a guide the new release dropped), then the directories of
    // that record left empty, and says so. Nothing is left that no record lists, so uninstall,
    // after `una` made the proof cache's key, leaves nothing at all.
    let Some(target) = target() else {
        return;
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-upgrade");
    let _ = std::fs::remove_dir_all(&dir);
    let version = "9.9.3";
    let pass = "PASS (every gate)";
    let documents = [
        ("README.md", "readme"),
        ("LICENSE", "license"),
        ("docs/guide/getting-started.md", "guide"),
    ];
    let home = installed_by_v0_3_16(&dir);
    // A recorded `bin/z3` that is a link to a file outside UNA_HOME: the link is removed, and
    // the file it names is kept.
    let elsewhere = dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("z3"), "a z3 of the user's").unwrap();
    std::fs::remove_file(home.join("bin/z3")).unwrap();
    std::os::unix::fs::symlink(elsewhere.join("z3"), home.join("bin/z3")).unwrap();
    release_holding(&dir, target, version, pass, None, &documents);
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let said = stdout(&output);
    for gone in ["bin/z3", "z3", "lib", "docs/guide/dropped.md"] {
        assert!(
            std::fs::symlink_metadata(home.join(gone)).is_err(),
            "{gone}: {said}"
        );
    }
    for kept in [
        "bin/una",
        "README.md",
        "LICENSE",
        "docs/guide/getting-started.md",
    ] {
        assert!(home.join(kept).is_file(), "{kept}: {said}");
    }
    assert_eq!(
        std::fs::read_to_string(home.join("bin/una")).unwrap(),
        "#!/bin/sh\necho una stub\n"
    );
    assert_eq!(
        std::fs::read_to_string(elsewhere.join("z3")).unwrap(),
        "a z3 of the user's"
    );
    assert!(
        said.contains(&format!(
            "removed what the earlier installation held and {version} does not: bin/z3 lib/libuna_rt.a z3/bin/z3 docs/guide/dropped.md\n"
        )),
        "{said}"
    );
    assert!(
        said.contains("removed z3, which una no longer needs"),
        "{said}"
    );
    let record = std::fs::read_to_string(home.join(".una-install")).unwrap();
    assert_eq!(
        record,
        format!(
            "# Una {version}, installed by install.sh; `sh install.sh --uninstall` removes what follows.\nversion {version}\ndir .\ndir bin\ndir docs\ndir docs/guide\nfile bin/una\nfile README.md\nfile LICENSE\nfile docs/guide/getting-started.md\n"
        )
    );
    // The same release again: nothing more to remove, and nothing said of it.
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(!stdout(&output).contains("removed"), "{}", stdout(&output));
    assert_eq!(
        std::fs::read_to_string(home.join(".una-install")).unwrap(),
        record
    );
    // `una check` made the proof cache's key (ADR 0064): uninstall removes it, names its path,
    // and leaves nothing of `~/.una`.
    std::fs::write(home.join("proof-cache.key"), [7u8; 32]).unwrap();
    let output = install(&dir, version, &["--uninstall"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(
        std::fs::symlink_metadata(&home).is_err(),
        "left after uninstall: {said}"
    );
    assert!(
        said.contains(&format!(
            "removed the proof cache's key {}",
            home.join("proof-cache.key").display()
        )),
        "{said}"
    );
    assert!(
        said.contains(&format!("removed {}\n", home.display())),
        "{said}"
    );

    // A file of the user's in a directory of the earlier record: the directory stays, with the
    // file, and stays recorded, so that uninstall removes it once it is empty.
    let home = installed_by_v0_3_16(&dir);
    std::fs::write(home.join("z3/notes.txt"), "mine").unwrap();
    // A recorded file that is a directory now is not a file of the installation: kept.
    std::fs::remove_file(home.join("lib/libuna_rt.a")).unwrap();
    std::fs::create_dir_all(home.join("lib/libuna_rt.a")).unwrap();
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(!home.join("z3/bin").exists() && home.join("z3/notes.txt").exists());
    assert!(home.join("lib/libuna_rt.a").is_dir());
    let record = std::fs::read_to_string(home.join(".una-install")).unwrap();
    assert!(
        record.contains("dir lib\n") && record.contains("dir z3\n") && !record.contains("z3/bin"),
        "{record}"
    );
    assert!(
        !record.contains("file lib") && !record.contains("file z3"),
        "{record}"
    );
    let _ = std::fs::remove_dir_all(&home);

    // An earlier record that names a path outside UNA_HOME: refused before anything is
    // downloaded or removed.
    let home = installed_by_v0_3_16(&dir);
    std::fs::write(dir.join("home/escape"), "kept").unwrap();
    let record = home.join(".una-install");
    let text = std::fs::read_to_string(&record).unwrap();
    // So is one that spells a path with a `.` or empty component, which the comparison with the
    // new installation's files would miss: `file ./bin/una` would remove the new `una`.
    let outside = "which is not inside";
    let spelled = "which has a '.' or empty component";
    for (bad, why) in [
        ("file ../escape\n", outside),
        ("file /etc/hosts\n", outside),
        ("dir ../home\n", outside),
        ("file ./bin/una\n", spelled),
        ("file bin//una\n", spelled),
        ("file bin/una/.\n", spelled),
        ("file bin/\n", spelled),
        ("file .\n", spelled),
        ("dir ./bin\n", spelled),
    ] {
        std::fs::write(&record, format!("{text}{bad}")).unwrap();
        let output = install(&dir, version, &[]);
        assert_eq!(output.status.code(), Some(1), "{bad}: {}", stderr(&output));
        assert!(stderr(&output).contains(why), "{bad}: {}", stderr(&output));
        assert_eq!(
            std::fs::read_to_string(home.join("bin/una")).unwrap(),
            "una 0.3.16",
            "{bad}"
        );
        assert!(
            home.join("bin/z3").exists() && dir.join("home/escape").exists(),
            "{bad}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_recorded_path_through_a_link_is_refused_for_every_user_before_anything_is_downloaded() {
    // ADR 0124 rule 32 (review 1 of #2199): a record path with a component that is a symbolic
    // link is refused for every user, not only for root, before anything is downloaded or
    // removed; otherwise `rm` and `rmdir` would follow it outside UNA_HOME, as an ordinary
    // user's `~/.una/z3` that links to their own Z3 would.
    use std::os::unix::fs::symlink;
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-record-links");
    let _ = std::fs::remove_dir_all(&dir);
    let version = "9.9.2";
    let home = dir.join("home/.una");
    let elsewhere = dir.join("elsewhere");
    release(
        &dir,
        "aarch64-unknown-linux-gnu",
        version,
        "PASS (every gate)",
        None,
    );
    let (output, _) = install_on(&dir, version, &GLIBC_239);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    std::fs::create_dir_all(elsewhere.join("z3/bin")).unwrap();
    std::fs::write(elsewhere.join("z3/bin/z3"), "the user's z3").unwrap();
    symlink(elsewhere.join("z3"), home.join("z3")).unwrap();
    let record = home.join(".una-install");
    let text = std::fs::read_to_string(&record).unwrap();
    // Recorded as a file only, as a directory only, and as directories with the file; by an
    // installation and by uninstall.
    for extra in [
        "file z3/bin/z3\n",
        "dir z3\n",
        "dir z3\ndir z3/bin\nfile z3/bin/z3\n",
    ] {
        for args in [&[][..], &["--uninstall"][..]] {
            std::fs::write(&record, format!("{text}{extra}")).unwrap();
            let (output, fetched) = install_on_with(&dir, version, &GLIBC_239, args, &[]);
            let case = format!("{extra:?} {args:?}");
            assert_eq!(output.status.code(), Some(1), "{case}: {}", stderr(&output));
            assert!(
                stderr(&output).contains("which goes through the symbolic link"),
                "{case}: {}",
                stderr(&output)
            );
            assert!(fetched.is_empty(), "{case}: {fetched:?}");
            assert_eq!(
                std::fs::read_to_string(elsewhere.join("z3/bin/z3")).unwrap(),
                "the user's z3",
                "{case}"
            );
            assert!(home.join("bin/una").exists() && record.exists(), "{case}");
        }
    }
    // The last component of a recorded file may be a link: the link goes, by an installation
    // and by uninstall, and the file it names outside stays.
    std::fs::remove_file(home.join("z3")).unwrap();
    for args in [&[][..], &["--uninstall"][..]] {
        if !home.join("bin/una").exists() {
            let (output, _) = install_on(&dir, version, &GLIBC_239);
            assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        }
        let text = std::fs::read_to_string(&record).unwrap();
        symlink(elsewhere.join("z3/bin/z3"), home.join("bin/z3")).unwrap();
        std::fs::write(&record, format!("{text}file bin/z3\n")).unwrap();
        let (output, _) = install_on_with(&dir, version, &GLIBC_239, args, &[]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{args:?}: {}",
            stderr(&output)
        );
        assert!(
            std::fs::symlink_metadata(home.join("bin/z3")).is_err(),
            "{args:?}"
        );
        assert_eq!(
            std::fs::read_to_string(elsewhere.join("z3/bin/z3")).unwrap(),
            "the user's z3",
            "{args:?}"
        );
    }
    assert!(!home.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_recorded_name_that_is_a_new_file_spelled_otherwise_is_never_removed() {
    // #2199 review 2: on a file system that ignores case, as macOS's default does, `BIN/una` and
    // `README.MD` name the new `bin/una` and `README.md`. A recorded name that is the same file
    // or directory as one the new installation holds is not removed, however it is spelled.
    let Some(target) = target() else {
        return;
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-case");
    let _ = std::fs::remove_dir_all(&dir);
    let version = "9.9.1";
    let home = dir.join("home/.una");
    release_holding(
        &dir,
        target,
        version,
        "PASS (every gate)",
        None,
        &[
            ("README.md", "readme"),
            ("docs/guide/getting-started.md", "guide"),
        ],
    );
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let record = home.join(".una-install");
    let text = std::fs::read_to_string(&record).unwrap();
    let ignores_case = home.join("BIN/una").exists();
    std::fs::write(
        &record,
        format!("{text}file BIN/una\nfile Bin/UNA\nfile README.MD\nfile DOCS/guide/Getting-Started.md\ndir BIN\ndir Docs/Guide\n"),
    )
    .unwrap();
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    for kept in ["bin/una", "README.md", "docs/guide/getting-started.md"] {
        assert!(
            home.join(kept).is_file(),
            "{kept} (ignores case: {ignores_case})"
        );
    }
    assert!(
        !stdout(&output).contains("removed"),
        "ignores case: {ignores_case}: {}",
        stdout(&output)
    );
    // The new record lists the new files as the installer spells them, and no other file.
    let record = std::fs::read_to_string(&record).unwrap();
    let recorded: Vec<&str> = record.lines().filter(|l| l.starts_with("file ")).collect();
    let expected: Vec<&str> = text.lines().filter(|l| l.starts_with("file ")).collect();
    assert_eq!(recorded, expected, "ignores case: {ignores_case}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_directory_swapped_for_a_link_during_the_download_is_not_followed() {
    // #2199 review 2: the record is checked before the download, and checked again just before
    // each removal, so a recorded directory replaced by a link while the archive downloads is
    // refused then, and nothing outside UNA_HOME is removed.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-swap");
    let _ = std::fs::remove_dir_all(&dir);
    let version = "9.9.0";
    let home = dir.join("home/.una");
    let elsewhere = dir.join("elsewhere");
    release(
        &dir,
        "aarch64-unknown-linux-gnu",
        version,
        "PASS (every gate)",
        None,
    );
    let (output, _) = install_on(&dir, version, &GLIBC_239);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    std::fs::create_dir_all(home.join("z3/bin")).unwrap();
    std::fs::write(home.join("z3/bin/z3"), "the installed z3").unwrap();
    std::fs::create_dir_all(elsewhere.join("z3/bin")).unwrap();
    std::fs::write(elsewhere.join("z3/bin/z3"), "the user's z3").unwrap();
    let record = home.join(".una-install");
    let text = std::fs::read_to_string(&record).unwrap();
    std::fs::write(
        &record,
        format!("{text}dir z3\ndir z3/bin\nfile z3/bin/z3\n"),
    )
    .unwrap();
    let z3 = home.join("z3");
    let hook = format!(
        "[ -L '{z3}' ] || {{ mv '{z3}' '{z3}.real' && ln -s '{outside}' '{z3}'; }}",
        z3 = z3.display(),
        outside = elsewhere.join("z3").display()
    );
    let (output, fetched) = install_on_with(
        &dir,
        version,
        &GLIBC_239,
        &[],
        &[("FETCH_HOOK", hook.as_str())],
    );
    assert!(!fetched.is_empty());
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("which goes through the symbolic link"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read_to_string(elsewhere.join("z3/bin/z3")).unwrap(),
        "the user's z3"
    );

    // Uninstall checks again before each `rm` and each `rmdir` too, and a refusal keeps the
    // record, so that uninstall can be run again. `z3` is swapped for the link when `rm` first
    // runs (for `bin/una`, before `z3/bin/z3`), and, in the second leg, when `sort` orders the
    // directories, after every file is gone.
    let reinstall = |extra: &str| {
        let _ = std::fs::remove_file(&z3);
        let _ = std::fs::remove_dir_all(home.join("z3.real"));
        let _ = std::fs::remove_dir_all(&home);
        let (output, _) = install_on(&dir, version, &GLIBC_239);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        std::fs::create_dir_all(home.join("z3/bin")).unwrap();
        let text = std::fs::read_to_string(&record).unwrap();
        std::fs::write(&record, format!("{text}{extra}")).unwrap();
    };
    for (tool, extra, outside) in [
        (
            "HOOK_rm",
            "dir z3\ndir z3/bin\nfile z3/bin/z3\n",
            "z3/bin/z3",
        ),
        ("HOOK_sort", "dir z3\ndir z3/bin\n", "z3/bin"),
    ] {
        reinstall(extra);
        if extra.contains("file z3/bin/z3") {
            std::fs::write(home.join("z3/bin/z3"), "the installed z3").unwrap();
        }
        let _ = std::fs::remove_dir_all(elsewhere.join("z3"));
        std::fs::create_dir_all(elsewhere.join("z3/bin")).unwrap();
        if outside.ends_with("/z3") {
            std::fs::write(elsewhere.join(outside), "the user's z3").unwrap();
        }
        let (output, _) = install_on_with(
            &dir,
            version,
            &GLIBC_239,
            &["--uninstall"],
            &[(tool, hook.as_str())],
        );
        assert_eq!(output.status.code(), Some(1), "{tool}: {}", stderr(&output));
        assert!(
            stderr(&output).contains("which goes through the symbolic link"),
            "{tool}: {}",
            stderr(&output)
        );
        assert!(
            elsewhere.join(outside).exists(),
            "{tool}: followed the link"
        );
        assert!(record.exists(), "{tool}: the record is gone");
        // With the link gone, uninstall runs again and finishes.
        std::fs::remove_file(&z3).unwrap();
        std::fs::rename(home.join("z3.real"), &z3).unwrap();
        let (output, _) = install_on_with(&dir, version, &GLIBC_239, &["--uninstall"], &[]);
        assert_eq!(output.status.code(), Some(0), "{tool}: {}", stderr(&output));
        assert!(!home.exists(), "{tool}");
        assert!(elsewhere.join(outside).exists(), "{tool}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_recorded_link_to_a_new_file_is_removed_and_the_new_file_kept() {
    // A recorded link is never a file this installation wrote, even when it names one: it is
    // removed as itself (`held_new` compares names that are not links), and the new file stays.
    let Some(target) = target() else {
        return;
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-link-to-new");
    let _ = std::fs::remove_dir_all(&dir);
    let version = "9.8.7";
    let home = dir.join("home/.una");
    release_holding(
        &dir,
        target,
        version,
        "PASS (every gate)",
        None,
        &[("README.md", "readme")],
    );
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    std::os::unix::fs::symlink("una", home.join("bin/z3")).unwrap();
    std::os::unix::fs::symlink("README.md", home.join("README.old")).unwrap();
    let record = home.join(".una-install");
    let text = std::fs::read_to_string(&record).unwrap();
    std::fs::write(&record, format!("{text}file bin/z3\nfile README.old\n")).unwrap();
    let output = install(&dir, version, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    for link in ["bin/z3", "README.old"] {
        assert!(
            std::fs::symlink_metadata(home.join(link)).is_err(),
            "{link}: {}",
            stdout(&output)
        );
    }
    assert_eq!(
        std::fs::read_to_string(home.join("bin/una")).unwrap(),
        "#!/bin/sh\necho una stub\n"
    );
    assert_eq!(
        std::fs::read_to_string(home.join("README.md")).unwrap(),
        "readme"
    );
    assert!(
        stdout(&output).contains(&format!(
            "removed what the earlier installation held and {version} does not: bin/z3 README.old\n"
        )),
        "{}",
        stdout(&output)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A fresh installation of `version` into `dir/home/.una` (or where `envs` says), for the
/// uninstall tests of the proof cache's key (#2097); `None` on a platform without a release.
fn installed_for_uninstall(name: &str, version: &str, envs: &[(&str, &str)]) -> Option<PathBuf> {
    let target = target()?;
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    release(&dir, target, version, "PASS (every gate)", None);
    let output = run_installer(
        &root().join("install.sh"),
        &dir,
        version,
        &[],
        SYSTEM_PATH,
        envs,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    Some(dir)
}

/// Runs `--uninstall` for the installation of [`installed_for_uninstall`], and returns what it
/// printed after checking that it exits with 0.
fn uninstall(dir: &Path, version: &str, envs: &[(&str, &str)]) -> String {
    let output = run_installer(
        &root().join("install.sh"),
        dir,
        version,
        &["--uninstall"],
        SYSTEM_PATH,
        envs,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn uninstall_removes_the_proof_cache_key_and_then_the_empty_directory() {
    // #2097: `una` makes `$HOME/.una/proof-cache.key` at run time (ADR 0064), so the record of
    // the installation does not list it. Uninstall removes it too, says so, and then removes
    // `~/.una`, which holds nothing else.
    let version = "9.9.6";
    let Some(dir) = installed_for_uninstall("uninstall-key", version, &[]) else {
        return;
    };
    let home = dir.join("home/.una");
    std::fs::write(home.join("proof-cache.key"), [7u8; 32]).unwrap();
    let said = uninstall(&dir, version, &[]);
    assert!(!home.exists(), "{said}");
    assert!(
        said.contains(&format!(
            "removed the proof cache's key {}",
            home.join("proof-cache.key").display()
        )),
        "{said}"
    );
    assert!(
        said.contains(&format!("removed {}", home.display())),
        "{said}"
    );
    assert!(!said.contains("kept"), "{said}");
    // Without a key, nothing is said of one.
    let Some(dir) = installed_for_uninstall("uninstall-key", version, &[]) else {
        return;
    };
    let said = uninstall(&dir, version, &[]);
    assert!(!home.exists() && !said.contains("proof cache"), "{said}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn uninstall_keeps_and_names_what_the_product_did_not_make() {
    // The key goes; a file and a directory of the user's stay, with `~/.una`, and the message
    // names them. A staged key file of another name is not the key and stays as well.
    let version = "9.9.5";
    let Some(dir) = installed_for_uninstall("uninstall-foreign", version, &[]) else {
        return;
    };
    let home = dir.join("home/.una");
    std::fs::write(home.join("proof-cache.key"), [7u8; 32]).unwrap();
    std::fs::write(home.join("notes.txt"), "mine").unwrap();
    std::fs::create_dir_all(home.join("keys")).unwrap();
    std::fs::write(home.join("keys/owner.key"), "mine").unwrap();
    std::fs::write(home.join("proof-cache.key.bak"), "mine").unwrap();
    let said = uninstall(&dir, version, &[]);
    assert!(!home.join("proof-cache.key").exists(), "{said}");
    assert!(!home.join("bin").exists() && !home.join(".una-install").exists());
    assert_eq!(
        std::fs::read_to_string(home.join("notes.txt")).unwrap(),
        "mine"
    );
    assert_eq!(
        std::fs::read_to_string(home.join("keys/owner.key")).unwrap(),
        "mine"
    );
    assert!(home.join("proof-cache.key.bak").exists());
    assert!(
        said.contains(&format!(
            "removed Una from {}; kept there, since the installer did not install them: keys, notes.txt, proof-cache.key.bak\n",
            home.display()
        )),
        "{said}"
    );
    assert!(said.contains("removed the proof cache's key"), "{said}");

    // A directory at the key's name is not the key: it is kept, with what it holds, and named.
    let Some(dir) = installed_for_uninstall("uninstall-foreign", version, &[]) else {
        return;
    };
    std::fs::create_dir_all(home.join("proof-cache.key")).unwrap();
    std::fs::write(home.join("proof-cache.key/inside"), "mine").unwrap();
    let said = uninstall(&dir, version, &[]);
    assert!(home.join("proof-cache.key/inside").exists(), "{said}");
    assert!(
        said.contains(&format!(
            "kept {}, which is not a file",
            home.join("proof-cache.key").display()
        )),
        "{said}"
    );

    // With UNA_HOME elsewhere, the key is still the one of `$HOME/.una`: it is removed, and
    // that directory after it when it is empty, or kept and named when it holds something.
    for foreign in [false, true] {
        let custom = dir.join("home/custom");
        let custom_text = custom.to_str().unwrap().to_owned();
        let envs = [("UNA_HOME", custom_text.as_str())];
        let Some(dir) = installed_for_uninstall("uninstall-foreign", version, &envs) else {
            return;
        };
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("proof-cache.key"), [7u8; 32]).unwrap();
        if foreign {
            std::fs::write(home.join("notes.txt"), "mine").unwrap();
        }
        let said = uninstall(&dir, version, &envs);
        assert!(!custom.exists(), "{said}");
        assert!(!home.join("proof-cache.key").exists(), "{said}");
        assert_eq!(home.exists(), foreign, "{said}");
        assert_eq!(
            said.contains(&format!(
                "kept {}, which holds: notes.txt\n",
                home.display()
            )),
            foreign,
            "{said}"
        );
    }

    // A key file that UNA_PROOF_KEY names is the user's choice of file: kept, and named.
    let named = dir.join("home/my.key");
    let named_text = named.to_str().unwrap().to_owned();
    let envs = [("UNA_PROOF_KEY", named_text.as_str())];
    let Some(dir) = installed_for_uninstall("uninstall-foreign", version, &envs) else {
        return;
    };
    std::fs::write(&named, [9u8; 32]).unwrap();
    std::fs::write(home.join("proof-cache.key"), [7u8; 32]).unwrap();
    let said = uninstall(&dir, version, &envs);
    assert!(named.exists() && !home.exists(), "{said}");
    assert!(
        said.contains(&format!(
            "kept the proof cache's key {named_text}, which UNA_PROOF_KEY names"
        )),
        "{said}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn uninstall_removes_a_key_that_is_a_link_and_never_what_it_names() {
    // The key's name is removed, never followed: a symbolic link there goes, and the file it
    // named, outside `~/.una`, stays as it was. So does the directory a link names.
    use std::os::unix::fs::symlink;
    let version = "9.9.4";
    let Some(dir) = installed_for_uninstall("uninstall-link", version, &[]) else {
        return;
    };
    let home = dir.join("home/.una");
    let elsewhere = dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("real.key"), "the user's file").unwrap();
    symlink(elsewhere.join("real.key"), home.join("proof-cache.key")).unwrap();
    let said = uninstall(&dir, version, &[]);
    assert!(!home.exists(), "{said}");
    assert_eq!(
        std::fs::read_to_string(elsewhere.join("real.key")).unwrap(),
        "the user's file"
    );
    assert!(
        said.contains(&format!(
            "removed the proof cache's key {}, a symbolic link; the file it named is kept",
            home.join("proof-cache.key").display()
        )),
        "{said}"
    );
    // A link to a directory, and a link to nothing.
    for to in ["elsewhere", "missing"] {
        let Some(dir) = installed_for_uninstall("uninstall-link", version, &[]) else {
            return;
        };
        let elsewhere = dir.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("real.key"), "the user's file").unwrap();
        symlink(dir.join(to), home.join("proof-cache.key")).unwrap();
        let said = uninstall(&dir, version, &[]);
        assert!(!home.exists(), "{to}: {said}");
        assert!(elsewhere.join("real.key").exists(), "{to}: {said}");
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
            // On a Linux host below glibc 2.39, 0.4.0 passes the version check and is then
            // refused by the glibc floor of the releases 0.4.0 to 0.5.1 (#2105), as the Linux
            // legs' AlmaLinux 8 image (glibc 2.28) shows; the floor's own test is
            // the_releases_built_on_glibc_2_39_keep_that_floor.
            if version == "0.4.0"
                && output.status.code() == Some(1)
                && stderr(&output).contains("the una of 0.4.0 needs glibc 2.39 or later")
            {
                assert!(
                    !stderr(&output).contains("0.4.0 and later"),
                    "{}",
                    stderr(&output)
                );
                assert!(!home.exists(), "{version}");
                continue;
            }
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
/// when it is `None`; `id -u` prints `uid`, or fails when it is empty. `curl` and `wget`, when the machine has them, record
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
    let mut patched = original.replace(
        "\nloaders=/lib\n",
        &format!("\nloaders='{}'\n", lib.display()),
    );
    // And, when `envs` names `DEFAULT_TMP`, its one `default_tmp=/tmp` line pointing there.
    if let Some((_, tmp)) = envs.iter().find(|(name, _)| *name == "DEFAULT_TMP") {
        assert_eq!(
            original
                .lines()
                .filter(|line| *line == "default_tmp=/tmp")
                .count(),
            1,
            "install.sh has no single `default_tmp=/tmp` line"
        );
        patched = patched.replace("\ndefault_tmp=/tmp\n", &format!("\ndefault_tmp='{tmp}'\n"));
    }
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
        &if host.uid.is_empty() {
            "echo 'id: cannot find name for user ID' >&2; exit 1".to_owned()
        } else {
            format!(
                "[ \"$1\" = -u ] && echo '{}' || PATH={SYSTEM_PATH} exec id \"$@\"",
                host.uid
            )
        },
    );
    // `mktemp` records its arguments in `dir/mktemps`, so that a test sees where the work
    // directory was made.
    let _ = std::fs::remove_file(dir.join("mktemps"));
    script(
        &bin.join("mktemp"),
        &format!(
            "printf '%s\\n' \"$*\" >> '{}'\nPATH={SYSTEM_PATH} exec mktemp \"$@\"",
            dir.join("mktemps").display()
        ),
    );
    // `FETCH_HOOK` in `envs`, a shell command that each download runs first: a test changes
    // UNA_HOME while the installer downloads, between its checks and its removals.
    let hook = envs
        .iter()
        .find(|(name, _)| *name == "FETCH_HOOK")
        .map_or("", |(_, hook)| hook);
    // `HOOK_<tool>` in `envs`, a shell command that each run of `<tool>` (such as `rm` or
    // `sort`) runs first: a test changes UNA_HOME at a known step of uninstall, which downloads
    // nothing.
    for (name, hook) in envs {
        if let Some(tool) = name.strip_prefix("HOOK_") {
            script(
                &bin.join(tool),
                &format!("{hook}\nPATH={SYSTEM_PATH} exec {tool} \"$@\""),
            );
        }
    }
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
                    "echo \"{fetcher} $*\" >> '{}'\n{hook}\nPATH={SYSTEM_PATH} exec {fetcher} \"$@\"",
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
    // The Linux `una` is a dynamic glibc executable built on glibc 2.28 (#1904, #2105): musl, an
    // older glibc and a glibc whose version cannot be read are refused, and nothing is downloaded,
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
        // Below the floor, compared as numbers: 2.4 is before 2.28, though not as text.
        (Some("glibc 2.27"), None, "this system has glibc 2.27"),
        (Some("glibc 2.17"), None, "this system has glibc 2.17"),
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
                "the una of 9.9.4 needs glibc 2.28 or later; Alpine and other musl systems are not supported"
            ),
            "{case}: {}",
            stderr(&output)
        );
        assert!(fetched.is_empty(), "{case}: {fetched:?}");
        assert!(!home.exists(), "{case}");
    }
    // At the floor and above, compared as numbers: installed.
    for (getconf, arch) in [
        "glibc 2.28",
        "glibc 2.31",
        "glibc 2.35",
        "glibc 2.36",
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
fn the_releases_built_on_glibc_2_39_keep_that_floor() {
    // 0.4.0 to 0.5.1 were built on glibc 2.39 (Ubuntu 24.04); the releases after 0.5.1 on 2.28
    // (#2105). UNA_VERSION naming one of the earlier ones is refused on a glibc below 2.39,
    // where its `una` could not start, and nothing is downloaded; the versions are compared as
    // numbers, so 0.5.10 and 0.10.0 are after 0.5.1.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-glibc-239");
    let _ = std::fs::remove_dir_all(&dir);
    let home = dir.join("home/.una");
    let target = "aarch64-unknown-linux-gnu";
    for (version, floor) in [
        ("0.4.0", "2.39"),
        ("0.4.9", "2.39"),
        ("0.5.0", "2.39"),
        ("0.5.1", "2.39"),
        ("0.5.2", "2.28"),
        ("0.5.10", "2.28"),
        ("0.6.0", "2.28"),
        ("0.10.0", "2.28"),
        ("1.0.0", "2.28"),
    ] {
        release(&dir, target, version, "PASS (every gate)", None);
        for arch in ["aarch64", "arm64"] {
            let host = Host {
                arch,
                getconf: Some("glibc 2.35"),
                ..GLIBC_239
            };
            let (output, fetched) = install_on(&dir, version, &host);
            let case = format!("{version} {arch}");
            if floor == "2.39" {
                assert_eq!(output.status.code(), Some(1), "{case}: {}", stderr(&output));
                assert!(
                    stderr(&output).contains(&format!(
                        "this system has glibc 2.35; the una of {version} needs glibc 2.39 or later"
                    )),
                    "{case}: {}",
                    stderr(&output)
                );
                assert!(fetched.is_empty(), "{case}: {fetched:?}");
                assert!(!home.exists(), "{case}");
            } else {
                assert_eq!(output.status.code(), Some(0), "{case}: {}", stderr(&output));
                assert!(!fetched.is_empty(), "{case}");
                std::fs::remove_dir_all(&home).unwrap();
            }
            // At 2.39 every one of them installs.
            let host = Host { arch, ..GLIBC_239 };
            let (output, fetched) = install_on(&dir, version, &host);
            assert_eq!(output.status.code(), Some(0), "{case}: {}", stderr(&output));
            assert!(!fetched.is_empty(), "{case}");
            std::fs::remove_dir_all(&home).unwrap();
        }
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
    let needs = "needs glibc 2.28 or later; Alpine and other musl systems are not supported";
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

#[cfg(unix)]
#[test]
fn an_existing_file_is_replaced_and_never_written_through_a_hard_link() {
    // #1966: a target that is a hard link to another file, which as root could be outside
    // UNA_HOME, passes the owner and mode checks; `cp` and `>` would then write into that
    // other file. Each target's name is removed first, so the other file keeps its contents,
    // as root and as an ordinary user.
    use std::os::unix::fs::MetadataExt;
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-hard-links");
    let _ = std::fs::remove_dir_all(&dir);
    let version = "9.8.9";
    let home = dir.join("home/.una");
    let elsewhere = dir.join("elsewhere");
    let target = "aarch64-unknown-linux-gnu";
    let pass = "PASS (every gate)";
    let targets = [
        "bin/una.new",
        "lib/libuna_rt.a",
        "README.md",
        "LICENSE",
        "docs/guide/getting-started.md",
        ".una-install",
    ];
    release_holding(
        &dir,
        target,
        version,
        pass,
        None,
        &[
            ("lib/libuna_rt.a", "runtime"),
            ("README.md", "readme"),
            ("LICENSE", "license"),
            ("docs/guide/getting-started.md", "guide"),
        ],
    );
    let root = Host {
        uid: "0",
        ..GLIBC_239
    };
    for host in [&root, &GLIBC_239] {
        for name in targets {
            let _ = std::fs::remove_dir_all(dir.join("home"));
            let _ = std::fs::remove_dir_all(&elsewhere);
            std::fs::create_dir_all(&elsewhere).unwrap();
            let victim = elsewhere.join("victim");
            std::fs::write(&victim, "not una's").unwrap();
            let path = home.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::hard_link(&victim, &path).unwrap();
            let (output, _) = install_on(&dir, version, host);
            let case = format!("{name} as uid {}", host.uid);
            assert_eq!(output.status.code(), Some(0), "{case}: {}", stderr(&output));
            assert_eq!(
                std::fs::read_to_string(&victim).unwrap(),
                "not una's",
                "{case}: written through"
            );
            assert_eq!(std::fs::metadata(&victim).unwrap().nlink(), 1, "{case}");
            if name != "bin/una.new" {
                assert_eq!(std::fs::metadata(&path).unwrap().nlink(), 1, "{case}");
                assert_ne!(
                    std::fs::read_to_string(&path).unwrap(),
                    "not una's",
                    "{case}"
                );
            }
            assert_eq!(
                std::fs::read_to_string(home.join("bin/una")).unwrap(),
                "#!/bin/sh\necho una stub\n",
                "{case}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn what_the_installer_may_not_replace_at_a_target_is_refused_before_anything_is_written() {
    // #2216: a directory, a FIFO or a link to a directory where the installer writes a file, or
    // a file where it writes in a directory, made `rm -f`, `mkdir -p` or `mv` stop the
    // installation halfway, with a raw error after `bin/una` was replaced. Each is refused with
    // the installer's own one-line message, for every user: at a fixed name before anything is
    // downloaded, at a guide's name once the archive is unpacked, and in both cases before
    // anything is written or removed. As root, the root checks refuse most of them first.
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-fixed-targets");
    let _ = std::fs::remove_dir_all(&dir);
    let version = "9.8.7";
    let home = dir.join("home/.una");
    let elsewhere = dir.join("elsewhere");
    let target = "aarch64-unknown-linux-gnu";
    release_holding(
        &dir,
        target,
        version,
        "PASS (every gate)",
        None,
        &[
            ("lib/libuna_rt.a", "runtime"),
            ("README.md", "readme"),
            ("LICENSE", "license"),
            ("docs/guide/getting-started.md", "guide"),
        ],
    );
    let root = Host {
        uid: "0",
        ..GLIBC_239
    };
    let not_a_file = "is neither a regular file nor a symbolic link";
    let not_a_directory = "is not a directory";
    let to_a_directory = "is a symbolic link to a directory";
    let directory = |rel: &str| std::fs::create_dir_all(home.join(rel)).unwrap();
    let file = |rel: &str| {
        let path = home.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "theirs").unwrap();
    };
    type Setup<'a> = Box<dyn Fn() + 'a>;
    // The case, the target it sets up, how, why it is refused, and whether the target is one of
    // every installation (checked before any download) or one that only some archives hold, the
    // runtime and the guides (checked once the archive is unpacked).
    let cases: Vec<(&str, Setup, &str, bool)> = vec![
        (
            "README.md",
            Box::new(|| directory("README.md")),
            not_a_file,
            true,
        ),
        (
            "LICENSE",
            Box::new(|| directory("LICENSE")),
            not_a_file,
            true,
        ),
        (
            "bin/una",
            Box::new(|| directory("bin/una")),
            not_a_file,
            true,
        ),
        (
            "bin/una.new",
            Box::new(|| directory("bin/una.new")),
            not_a_file,
            true,
        ),
        (
            "lib/libuna_rt.a",
            Box::new(|| directory("lib/libuna_rt.a")),
            not_a_file,
            false,
        ),
        (
            ".una-install",
            Box::new(|| directory(".una-install")),
            not_a_file,
            true,
        ),
        (
            "README.md",
            Box::new(|| {
                std::fs::create_dir_all(&home).unwrap();
                let made = Command::new("mkfifo")
                    .arg(home.join("README.md"))
                    .status()
                    .unwrap();
                assert!(made.success());
            }),
            not_a_file,
            true,
        ),
        (
            "bin/una",
            Box::new(|| {
                directory("bin");
                symlink(&elsewhere, home.join("bin/una")).unwrap();
            }),
            to_a_directory,
            true,
        ),
        ("bin", Box::new(|| file("bin")), not_a_directory, true),
        ("lib", Box::new(|| file("lib")), not_a_directory, false),
        ("docs", Box::new(|| file("docs")), not_a_directory, false),
        (
            "docs/guide",
            Box::new(|| file("docs/guide")),
            not_a_directory,
            false,
        ),
        (
            "docs/guide/getting-started.md",
            Box::new(|| directory("docs/guide/getting-started.md")),
            not_a_file,
            false,
        ),
    ];
    for host in [&GLIBC_239, &root] {
        for (rel, setup, why, fixed) in &cases {
            let _ = std::fs::remove_dir_all(dir.join("home"));
            let _ = std::fs::remove_dir_all(&elsewhere);
            std::fs::create_dir_all(&elsewhere).unwrap();
            // The `bin/una` of an earlier installation, which a refusal keeps.
            let old = !rel.starts_with("bin");
            if old {
                file("bin/una");
            }
            setup();
            let (output, fetched) = install_on(&dir, version, host);
            let case = format!("{rel} ({why}) as uid {}", host.uid);
            assert_eq!(output.status.code(), Some(1), "{case}: {}", stderr(&output));
            // As root, the root checks refuse every case but a guide's name first, before any
            // download, with their own message.
            let before_download = if host.uid == "0" {
                *rel != "docs/guide/getting-started.md"
            } else {
                *fixed
            };
            if !before_download || host.uid != "0" {
                let message = format!("una install: error: {}/{rel} {why}", home.display());
                assert!(
                    stderr(&output).starts_with(&message),
                    "{case}: {}",
                    stderr(&output)
                );
                assert_eq!(
                    stderr(&output).lines().count(),
                    1,
                    "{case}: {}",
                    stderr(&output)
                );
            }
            assert_eq!(fetched.is_empty(), before_download, "{case}: {fetched:?}");
            if old {
                assert_eq!(
                    std::fs::read_to_string(home.join("bin/una")).unwrap(),
                    "theirs",
                    "{case}: bin/una replaced"
                );
            }
            for written in ["bin/una.new", "README.md", "LICENSE", "lib/libuna_rt.a"] {
                let path = home.join(written);
                assert!(
                    *rel == written || std::fs::symlink_metadata(&path).is_err(),
                    "{case}: {written} written"
                );
            }
            assert_eq!(
                std::fs::read_dir(&elsewhere).unwrap().count(),
                0,
                "{case}: written outside UNA_HOME"
            );
        }
    }
    // #2224 review 1: as an ordinary user, an existing directory the installer writes in that
    // the user cannot write in made `rm -f` stop the installation halfway in the same way. It is
    // refused before anything is written: `bin/` before any download, `docs/guide/` once the
    // archive is unpacked. As root, the owner and mode checks stand unchanged.
    let read_only = "is a directory this user cannot write in";
    for (rel, fixed) in [("bin", true), ("docs/guide", false), ("lib", false)] {
        let _ = std::fs::remove_dir_all(dir.join("home"));
        file("bin/una");
        file("README.md");
        directory(rel);
        std::fs::set_permissions(home.join(rel), std::fs::Permissions::from_mode(0o555)).unwrap();
        let (output, fetched) = install_on(&dir, version, &GLIBC_239);
        std::fs::set_permissions(home.join(rel), std::fs::Permissions::from_mode(0o755)).unwrap();
        let case = format!("{rel} of mode 555");
        assert_eq!(output.status.code(), Some(1), "{case}: {}", stderr(&output));
        let message = format!("una install: error: {}/{rel} {read_only}", home.display());
        assert!(
            stderr(&output).starts_with(&message),
            "{case}: {}",
            stderr(&output)
        );
        assert_eq!(
            stderr(&output).lines().count(),
            1,
            "{case}: {}",
            stderr(&output)
        );
        assert_eq!(fetched.is_empty(), fixed, "{case}: {fetched:?}");
        for kept in ["bin/una", "README.md"] {
            assert_eq!(
                std::fs::read_to_string(home.join(kept)).unwrap(),
                "theirs",
                "{case}: {kept} replaced"
            );
        }
    }
    // UNA_HOME itself a file: refused before any download, the file kept.
    let _ = std::fs::remove_dir_all(dir.join("home"));
    std::fs::create_dir_all(dir.join("home")).unwrap();
    std::fs::write(&home, "theirs").unwrap();
    let (output, fetched) = install_on(&dir, version, &GLIBC_239);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains(not_a_directory),
        "{}",
        stderr(&output)
    );
    assert!(fetched.is_empty(), "{fetched:?}");
    assert_eq!(std::fs::read_to_string(&home).unwrap(), "theirs");
    // What the installer may replace still installs, for an ordinary user: a symbolic link at
    // README.md to the user's own file is removed as itself, and the user's file is kept.
    let _ = std::fs::remove_dir_all(dir.join("home"));
    let _ = std::fs::remove_dir_all(&elsewhere);
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(elsewhere.join("mine"), "the user's").unwrap();
    symlink(elsewhere.join("mine"), home.join("README.md")).unwrap();
    let (output, _) = install_on(&dir, version, &GLIBC_239);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        std::fs::read_to_string(elsewhere.join("mine")).unwrap(),
        "the user's"
    );
    let readme = std::fs::symlink_metadata(home.join("README.md")).unwrap();
    assert!(readme.file_type().is_file());
    assert_eq!(
        std::fs::read_to_string(home.join("README.md")).unwrap(),
        "readme"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_work_directory_whose_name_holds_a_newline_still_matches_the_digest() {
    // The digest is read from the archive's bytes on stdin, never with its file name, which
    // `sha256sum` would escape, prefixing the digest with `\`.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("install-newline-tmp");
    let _ = std::fs::remove_dir_all(&dir);
    let version = "9.8.8";
    release(
        &dir,
        "aarch64-unknown-linux-gnu",
        version,
        "PASS (every gate)",
        None,
    );
    let tmp = dir.join("tmp\nline");
    std::fs::create_dir_all(&tmp).unwrap();
    let (output, _) = install_on_with(
        &dir,
        version,
        &GLIBC_239,
        &[],
        &[("TMPDIR", tmp.to_str().unwrap())],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(dir.join("home/.una/bin/una").exists());
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
            "bin/una.new is a link",
            Box::new(|| {
                std::fs::create_dir_all(home.join("bin")).unwrap();
                symlink(elsewhere.join("una.new"), home.join("bin/una.new")).unwrap();
            }),
            link,
        ),
        (
            "a leftover bin/una.new writable by others",
            Box::new(|| {
                std::fs::create_dir_all(home.join("bin")).unwrap();
                std::fs::write(home.join("bin/una.new"), "theirs").unwrap();
                mode(&home.join("bin/una.new"), 0o666);
            }),
            writable,
        ),
        (
            "LICENSE writable by its group",
            Box::new(|| {
                std::fs::create_dir_all(&home).unwrap();
                std::fs::write(home.join("LICENSE"), "theirs").unwrap();
                mode(&home.join("LICENSE"), 0o664);
            }),
            writable,
        ),
        (
            "a guide writable by others",
            Box::new(|| {
                std::fs::create_dir_all(home.join("docs/guide")).unwrap();
                std::fs::write(home.join("docs/guide/getting-started.md"), "theirs").unwrap();
                mode(&home.join("docs/guide/getting-started.md"), 0o666);
            }),
            writable,
        ),
        (
            "LICENSE is a directory",
            Box::new(|| std::fs::create_dir_all(home.join("LICENSE")).unwrap()),
            "must be a regular file",
        ),
        (
            "LICENSE is a FIFO",
            Box::new(|| {
                std::fs::create_dir_all(&home).unwrap();
                let made = Command::new("mkfifo")
                    .arg(home.join("LICENSE"))
                    .status()
                    .unwrap();
                assert!(made.success());
            }),
            "must be a regular file",
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
    // TMPDIR, where the archive is checked and unpacked: writable by others and not sticky,
    // another user could replace the archive after its check, so it is refused before any
    // download; sticky, as /tmp is, it is accepted.
    reset();
    let tmp = dir.join("home/tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    mode(&tmp, 0o777);
    let tmpdir = tmp.to_str().unwrap();
    let (output, fetched) = install_on_with(&dir, version, &root, &[], &[("TMPDIR", tmpdir)]);
    assert_refused("TMPDIR not sticky", &output, &fetched, writable, &home);
    assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 0);
    mode(&tmp, 0o1777);
    let (output, _) = install_on_with(&dir, version, &root, &[], &[("TMPDIR", tmpdir)]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(home.join("bin/una").exists());
    // A TMPDIR only root can write, below a directory that others can write and is not sticky:
    // refused, as another user could replace it.
    reset();
    let open = dir.join("home/open");
    std::fs::create_dir_all(open.join("tmp")).unwrap();
    mode(&open, 0o777);
    let below = open.join("tmp");
    let (output, fetched) = install_on_with(
        &dir,
        version,
        &root,
        &[],
        &[("TMPDIR", below.to_str().unwrap())],
    );
    assert_refused(
        "TMPDIR below an open directory",
        &output,
        &fetched,
        writable,
        &home,
    );
    // TMPDIR unset: the default, /tmp, is checked the same way (here a directory of the test
    // stands for it): writable by others and not sticky, refused; sticky, installed.
    reset();
    let default = dir.join("home/default-tmp");
    std::fs::create_dir_all(&default).unwrap();
    mode(&default, 0o777);
    let seam = [("DEFAULT_TMP", default.to_str().unwrap())];
    let (output, fetched) = install_on_with(&dir, version, &root, &[], &seam);
    assert_refused("default TMPDIR", &output, &fetched, writable, &home);
    mode(&default, 0o1777);
    let (output, _) = install_on_with(&dir, version, &root, &[], &seam);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let made = std::fs::read_to_string(dir.join("mktemps")).unwrap();
    assert_eq!(
        made,
        format!(
            "-d {}/una-install.XXXXXX\n",
            std::fs::canonicalize(&default).unwrap().display()
        )
    );
    // A TMPDIR that is a link in a directory that others can write, to a directory only root
    // can write: the work directory is made in the physical path that was checked, never
    // through the link, which another user could point elsewhere after the check.
    reset();
    let open = dir.join("home/open");
    let only = dir.join("home/only-root");
    std::fs::create_dir_all(&open).unwrap();
    std::fs::create_dir_all(&only).unwrap();
    symlink(&only, open.join("tmp")).unwrap();
    mode(&open, 0o777);
    let through = open.join("tmp");
    let (output, _) = install_on_with(
        &dir,
        version,
        &root,
        &[],
        &[("TMPDIR", through.to_str().unwrap())],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let made = std::fs::read_to_string(dir.join("mktemps")).unwrap();
    assert_eq!(
        made,
        format!(
            "-d {}/una-install.XXXXXX\n",
            std::fs::canonicalize(&only).unwrap().display()
        )
    );
    // Uninstall makes no work directory: as root it does not check TMPDIR.
    mode(&open, 0o755);
    std::fs::remove_file(open.join("tmp")).unwrap();
    mode(&open, 0o777);
    let (output, _) = install_on_with(
        &dir,
        version,
        &root,
        &["--uninstall"],
        &[("TMPDIR", open.to_str().unwrap())],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(!home.exists());
    // An `id -u` that prints root's id with leading zeros is root's.
    reset();
    let zeros = Host {
        uid: "00",
        ..GLIBC_239
    };
    let (output, _) = install_on(&dir, version, &zeros);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("as root, the installer does not run the installed una"),
        "{}",
        stdout(&output)
    );
    // Under a umask of 000, root's installation is still writable only by root.
    reset();
    let (output, _) = install_on_with(&dir, version, &root, &[], &[("UMASK", "000")]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    for path in [
        &home,
        &home.join("bin"),
        &home.join("bin/una"),
        &home.join(".una-install"),
    ] {
        let bits = std::fs::metadata(path).unwrap().permissions().mode();
        assert_eq!(bits & 0o022, 0, "{} has mode {bits:o}", path.display());
    }
    // The umask seam is live: an ordinary user's installation under 000 has a writable bin.
    reset();
    let (output, _) = install_on_with(&dir, version, &GLIBC_239, &[], &[("UMASK", "000")]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let bits = std::fs::metadata(home.join("bin"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(bits & 0o022, 0o022, "bin has mode {bits:o}");
    // `id -u` that fails, or prints something else than a number: refused, never taken as an
    // ordinary user, whose installation skips the root checks.
    reset();
    for uid in ["", "root", "0 "] {
        let host = Host { uid, ..GLIBC_239 };
        let (output, fetched) = install_on(&dir, version, &host);
        assert_refused(
            &format!("id -u {uid:?}"),
            &output,
            &fetched,
            "cannot read this process's user id",
            &home,
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
    // Recorded as a file only, as a directory only, and as directories with the file.
    for extra in [
        "file z3/bin/z3\n",
        "dir z3\n",
        "dir z3\ndir z3/bin\nfile z3/bin/z3\n",
    ] {
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
    // Installing over it as root is refused the same way, before anything is downloaded or
    // removed, since the installation removes what the earlier record names (#1992).
    for extra in ["file z3/bin/z3\n", "dir z3\ndir z3/bin\nfile z3/bin/z3\n"] {
        std::fs::write(&record, format!("{text}{extra}")).unwrap();
        let (output, fetched) = install_on(&dir, version, &root);
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
        assert!(fetched.is_empty(), "{extra}: {fetched:?}");
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
