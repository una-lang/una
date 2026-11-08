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
