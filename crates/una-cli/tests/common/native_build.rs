//! Building and running native programs in the tests of `una-cli`: the repository root, the
//! runtime library that `una build` links, and `una` itself. Shared by `native.rs` and
//! `timing_channels.rs` (#1145).

// Each test file uses its own subset of these helpers.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

/// The repository root.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Builds the runtime library next to the `una` binary, once: `cargo test` builds `una-rt`'s
/// static library only when asked. Under `cargo test --release`, where the `una` binary is the
/// release one, it builds the release runtime beside it, so the executables link it: the
/// timing oracles run that way (#1145), since debug timings are not those of a release.
pub fn runtime() {
    static BUILT: OnceLock<()> = OnceLock::new();
    BUILT.get_or_init(|| {
        // Set by the nextest setup script (.config/nextest.toml): the library is built, and a second build in each test
        // process would rewrite it under the tests that link it.
        #[allow(clippy::disallowed_methods)]
        let prebuilt = std::env::var_os("UNA_RT_PREBUILT").is_some();
        if prebuilt {
            return;
        }
        let mut args = vec!["build", "--quiet", "--locked", "-p", "una-rt"];
        if !cfg!(debug_assertions) {
            args.push("--release");
        }
        let status = Command::new(env!("CARGO"))
            .args(&args)
            .current_dir(root())
            .status()
            .unwrap();
        assert!(status.success(), "cargo build -p una-rt failed");
    });
}

/// The runtime library built with its `test-hooks` feature, once, in a target directory of its
/// own, so that the shipped-shape library beside `una` stays without them: its path, for
/// `UNA_RT_LIB`. As `runtime`, in the profile of the test build (the placement probe of
/// `serve_owner_spread`, #1660, as the scripted engine of `serve_runs.rs`).
pub fn hooked_runtime() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let (profile_args, profile): (&[&str], &str) = if cfg!(debug_assertions) {
                (&[], "debug")
            } else {
                (&["--release"], "release")
            };
            let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("serve-hooks");
            let status = Command::new(env!("CARGO"))
                .args(["build", "--quiet", "--locked", "-p", "una-rt"])
                .args(profile_args)
                .args(["--features", "una-rt/test-hooks", "--target-dir"])
                .arg(&target)
                .current_dir(root())
                .status()
                .unwrap();
            assert!(
                status.success(),
                "cargo build -p una-rt --features una-rt/test-hooks failed"
            );
            target.join(profile).join("libuna_rt.a")
        })
        .clone()
}

/// The markers that only the test hooks of `una-cli` and of `una-http` compile into `una`, the
/// ones `scripts/check-release.sh` refuses in a shipped `una`: the solver's stand-in and the
/// scripted serve engine of `--serve-script` (`spec/31 §8` rule 6) are in a binary that holds
/// both.
pub const TEST_HOOK_MARKERS: [&str; 2] =
    ["UNA_CLI_TEST_HOOKS_MARKER", "UNA_HTTP_TEST_HOOKS_MARKER"];

/// The test build of `una`, held for this test process (#2201): a hard link, made at the first
/// call, to the file that `CARGO_BIN_EXE_una` names, checked to hold the test hooks.
///
/// `CARGO_BIN_EXE_una` is `target/<profile>/una`, the one path where cargo puts a copy of
/// whichever `una` it built last. The tests' build has the feature `test-hooks` (the
/// dev-dependency of `una-cli` on itself); a plain build of `una-cli` (`cargo build`, `cargo
/// run`, any build without the tests) does not, and both write that path: each cargo command
/// puts its own build there as a new file. So another cargo command in the same target
/// directory, while a test runs, swaps the binary under it, and every later `una` the test
/// starts is a shipped-shape one, which refuses `--serve-script`. The hard link keeps the file
/// that was there, whatever replaces the path afterwards; a binary already swapped when the test
/// starts fails here, with the reason, never at some later case.
#[cfg(unix)]
pub fn hooked_una() -> PathBuf {
    static HELD: OnceLock<PathBuf> = OnceLock::new();
    HELD.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("hooked-una");
        let held = hold_test_build(Path::new(env!("CARGO_BIN_EXE_una")), &dir);
        assert!(
            held.is_ok(),
            "{}",
            held.as_ref().err().map_or("", String::as_str)
        );
        held.unwrap()
    })
    .clone()
}

/// How long a held build of another key stays: longer than any test run (the tests gate stops a
/// run at 5,400 s), so no running test loses the build it holds.
#[cfg(unix)]
const HELD_FOR: std::time::Duration = std::time::Duration::from_secs(6 * 3600);

/// Holds the file at `source` by a hard link in `dir` and checks that it holds
/// [`TEST_HOOK_MARKERS`]: the link's path, or why the file is not a test build of `una`.
///
/// The link is named by the file's size and modification time, which cargo's copy of one build
/// keeps (each cargo command puts a new copy at `source`, with a new inode): every test process
/// that holds the same build shares one name, and a later copy of it takes the name in one
/// rename, so the links do not grow with the runs. A link of another key, unchanged for longer
/// than [`HELD_FOR`], is the build of an earlier change and is removed.
#[cfg(unix)]
pub fn hold_test_build(source: &Path, dir: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::MetadataExt;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let pending = dir.join(format!("pending-{}", std::process::id()));
    let _ = std::fs::remove_file(&pending);
    std::fs::hard_link(source, &pending).map_err(|e| {
        format!(
            "cannot hold the test build of `una` at {}: {e}",
            source.display()
        )
    })?;
    // The key is read from the link, not from `source`, which may already name another file.
    let file = std::fs::metadata(&pending).map_err(|e| format!("{}: {e}", pending.display()))?;
    let held = dir.join(format!(
        "una-{}-{}.{:09}",
        file.len(),
        file.mtime(),
        file.mtime_nsec()
    ));
    // A rename onto a link to the same file changes nothing and leaves `pending`: remove it.
    std::fs::rename(&pending, &held).map_err(|e| format!("{}: {e}", held.display()))?;
    let _ = std::fs::remove_file(&pending);
    let now = std::time::SystemTime::now();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let stale = path != held
                && entry.file_name().to_string_lossy().starts_with("una-")
                && std::fs::symlink_metadata(&path).is_ok_and(|m| {
                    let changed = std::time::UNIX_EPOCH
                        + std::time::Duration::from_secs(u64::try_from(m.ctime()).unwrap_or(0));
                    now.duration_since(changed).is_ok_and(|age| age > HELD_FOR)
                });
            if stale {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    for marker in TEST_HOOK_MARKERS {
        let found = Command::new("grep")
            .env("LC_ALL", "C")
            .args(["-a", "-q", "-F", "-e", marker])
            .arg(&held)
            .status()
            .map_err(|e| format!("grep: {e}"))?;
        match found.code() {
            Some(0) => {}
            Some(1) => {
                // Nothing that is not a test build stays held.
                let _ = std::fs::remove_file(&held);
                return Err(format!(
                    "{} was built without the test hooks (it holds no {marker}): it is not the \
                     test build of `una` that these tests need. A cargo command that builds `una` \
                     without its tests (`cargo build`, `cargo run`, `-p una-cli`) wrote that path \
                     in this target directory; give each cargo command its own CARGO_TARGET_DIR, \
                     rebuild the tests (`cargo nextest run --no-run`) and run them again (#2201)",
                    source.display()
                ));
            }
            _ => return Err(format!("grep {marker} {}: {found}", held.display())),
        }
    }
    Ok(held)
}

/// Runs `una` in `dir`, once the runtime library is built: `una build` links it, and cargo
/// builds the test binaries before the staticlib on a cold target directory.
pub fn una(dir: &Path, args: &[&str]) -> Output {
    runtime();
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

/// `una check .` in `dir`, with no runtime library built first, since checking links nothing:
/// the check of every timing oracle's program (#1947).
pub fn una_check(dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(["check", "."])
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

/// A package of edition `0.4` named `name` in a fresh temporary directory, whose entry module
/// `module` is `program`, granted `console`, built with `una build`.
pub fn within_package(name: &str, module: &str, program: &str) -> PathBuf {
    let dir = write_within_package(name, module, program);
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    dir
}

/// The package of [`within_package`], written and not built.
pub fn write_within_package(name: &str, module: &str, program: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("una-timing-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita {name}\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry {module}\n  grants\n    console\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/within_runs/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join(format!("src/{module}.una")), program).unwrap();
    dir
}
