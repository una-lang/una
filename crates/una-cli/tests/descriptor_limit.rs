//! A program that installs the stop handlers runs under a low soft limit of open files, in
//! both engines (#1757): the wake pipe moves as high as the limit allows, and a limit too
//! small for the runtime is a usage error, exit code 2, never the internal error U8001. Each
//! run is a child of `sh`, whose `ulimit -n` sets the soft limit for it alone.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/native_build.rs"]
mod native_build;

use std::path::Path;
use std::process::{Command, Output};

use native_build::{root, within_package};

/// The lines that `for_ever_on_stop` prints.
const PRINTED: &str = "alive 0\nalive 1\nalive 2\nstopped at 3\nafter 3\n";

/// Runs `program` with `args` in `dir` under `ulimit -n limit`.
fn limited(dir: &Path, limit: u64, program: &Path, args: &[&str]) -> Output {
    Command::new("sh")
        .arg("-c")
        .arg(format!("ulimit -n {limit} && exec \"$0\" \"$@\""))
        .arg(program)
        .args(args)
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("NO_COLOR", "1")
        .current_dir(dir)
        .output()
        .unwrap()
}

#[test]
fn a_stop_handling_program_runs_under_low_limits_in_both_engines() {
    let source = root().join("tests/conformance/runtime/for_ever_on_stop/src/alive.una");
    let program = std::fs::read_to_string(source).unwrap();
    let dir = within_package("fdlimit", "alive", &program);
    let una = Path::new(env!("CARGO_BIN_EXE_una"));
    let native = dir.join("target/una/fdlimit");
    for limit in [64, 256, 512] {
        let interpreted = limited(&dir, limit, una, &["run", "."]);
        assert_eq!(
            interpreted.status.code(),
            Some(0),
            "una run at {limit}: {interpreted:?}"
        );
        assert_eq!(
            String::from_utf8_lossy(&interpreted.stdout),
            PRINTED,
            "una run at {limit}"
        );
        let built = limited(&dir, limit, &native, &[]);
        assert_eq!(built.status.code(), Some(0), "native at {limit}: {built:?}");
        assert_eq!(
            String::from_utf8_lossy(&built.stdout),
            PRINTED,
            "native at {limit}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_limit_too_small_for_the_runtime_is_a_usage_error_in_both_engines() {
    let source = root().join("tests/conformance/runtime/for_ever_on_stop/src/alive.una");
    let program = std::fs::read_to_string(source).unwrap();
    let dir = within_package("fdtiny", "alive", &program);
    let una = Path::new(env!("CARGO_BIN_EXE_una"));
    let native = dir.join("target/una/fdtiny");
    for (engine, program, args) in [
        ("una run", una, &["run", "."][..]),
        ("native", &*native, &[][..]),
    ] {
        let refused = limited(&dir, 12, program, args);
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert_eq!(refused.status.code(), Some(2), "{engine}: {refused:?}");
        assert!(
            stderr.contains("`ulimit -n`) is 12") && !stderr.contains("U8001"),
            "{engine}: {stderr}"
        );
        assert!(refused.stdout.is_empty(), "{engine}: {refused:?}");
        // The minimum itself is enough.
        let ran = limited(&dir, una_sys::signals::MIN_DESCRIPTORS, program, args);
        assert_eq!(
            ran.status.code(),
            Some(0),
            "{engine} at the minimum: {ran:?}"
        );
        assert_eq!(
            String::from_utf8_lossy(&ran.stdout),
            PRINTED,
            "{engine} at the minimum"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
