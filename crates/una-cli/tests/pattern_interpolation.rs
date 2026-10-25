//! S-28 (#801): a text literal with interpolation in a pattern decided a branch, or the `Bool` of
//! `is`, by the values it interpolates, which could be `secret`, with no diagnostic. A pattern's
//! literal is a constant now, and interpolating in it is U4008 (`spec/07 §7`
//! rule 3). The original leaking program must be refused: `una run` runs nothing.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const LOCK: &str = "encoding_version = \"una-smt-1\"\nsolver           = \"z3\"\nsolver_version   = \"5.1.0\"\nlogic            = \"QF_UFLIA\"\nstep_budget      = 5000000\nrandom_seed      = 0\n";

/// The program the finding reported: which line it prints depended on the `secret` text.
const LEAK: &str = "module leak\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let s: secret Text = \"abc\"\n  let g = \"abc\"\n  when g\n    is \"{s}\" -> @console.print(\"yes\")\n    else -> @console.print(\"no\")\n";

fn package(edition: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("pattern_interpolation")
        .join(edition.replace('.', "_"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        format!(
            "[package]\nname = \"leak\"\nversion = \"0.1.0\"\nedition = \"{edition}\"\n\n[run]\nentry = \"leak\"\ngrants = [\"console\"]\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("verification.lock"), LOCK).unwrap();
    std::fs::write(dir.join("src/leak.una"), LEAK).unwrap();
    dir
}

fn una(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .env("NO_COLOR", "1")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

#[test]
fn the_interpreter_refuses_a_pattern_that_interpolates_a_secret() {
    let edition = "0.1";
    let dir = package(edition);
    // The interpreter: nothing runs, so neither line is printed.
    let run = una(&dir, &["run", "."]);
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert_eq!(run.status.code(), Some(1), "{edition}: {stderr}");
    assert!(stderr.contains("U4008"), "{edition}: {stderr}");
    assert!(run.stdout.is_empty(), "{edition}: {run:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
