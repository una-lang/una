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
///
/// `cargo nextest` runs each test in a process of its own and names its run in
/// `NEXTEST_RUN_ID`: the first processes of a run build the runtime, and the others find the
/// run's stamp, written after a build succeeded, and skip the build, so that the hundreds of
/// tests of one run do not each wait for Cargo's lock (#1652). A run builds once, as one
/// process of `cargo test` does; without the variable, every process builds.
pub fn runtime() {
    static BUILT: OnceLock<()> = OnceLock::new();
    BUILT.get_or_init(|| {
        let profile = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        #[allow(clippy::disallowed_methods)]
        let run = std::env::var("NEXTEST_RUN_ID").ok();
        let stamp = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("una-rt-{profile}.run"));
        if let Some(run) = &run
            && std::fs::read_to_string(&stamp).is_ok_and(|built| &built == run)
        {
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
        if let Some(run) = run {
            // Written whole under another name and renamed into place, so that no process
            // reads half a stamp.
            let staged = stamp.with_extension(format!("run.{}", std::process::id()));
            std::fs::write(&staged, run).unwrap();
            std::fs::rename(&staged, &stamp).unwrap();
        }
    });
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

/// A package of edition `0.4` named `name` in a fresh temporary directory, whose entry module
/// `module` is `program`, granted `console`, built with `una build`.
pub fn within_package(name: &str, module: &str, program: &str) -> PathBuf {
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
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    dir
}
