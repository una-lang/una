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

/// Runs `una` in `dir`, once the runtime library is built: `una build` links it, and cargo
/// builds the test binaries before the staticlib on a cold target directory.
pub fn una(dir: &Path, args: &[&str]) -> Output {
    runtime();
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}
