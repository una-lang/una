//! The size of the base code of `una-sys`: every Rust file under `crates/una-sys/src/` but those
//! of the feature `solver` (ADR 0058, amendments of 2026-10-25, 2026-10-28 and 2026-11-19). The
//! amendments cap that code at its measured physical count plus at most 5%, so that the trusted,
//! `unsafe`-bearing code cannot drift upward unnoticed. Until the amendment of 2026-11-19 nothing
//! enforced the count, and it passed the cap of 1,594 lines unseen; this test fails if the base
//! code grows past the cap. When a change needs more, raise [`CAP`] with a dated amendment of
//! ADR 0058 that states the new count, and with the adversarial security review that item 3 of
//! its amendment of 2026-11-03 requires for a raised cap, never silently. The files of `solver`
//! have their own cap, in `solver_size.rs`.
//!
//! The count sees the Rust files under `src` only, so the other tests here refuse what would
//! bring code into the base without being counted: a `#[path]` attribute or an `include!` in a
//! counted file, a build script, and a `solver` module compiled without its feature.
// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

/// The cap: the measured count of 1,651 lines (ADR 0058's amendment of 2026-11-19) plus at most
/// 5%, rounded down.
const CAP: usize = 1733;

/// What a counted file must not hold: each brings in code or data from a file that the count
/// may not see (one outside `src`, or one without the extension `rs`).
const UNCOUNTED_SOURCES: [&str; 4] = ["#[path", "include!", "include_str!", "include_bytes!"];

#[test]
fn the_base_code_stays_within_its_capped_size() {
    let files = base_files();
    let total: usize = files.iter().map(|(_, lines)| lines).sum();
    assert!(
        total <= CAP,
        "the base code of `una-sys` is {total} physical lines, over the cap of {CAP} (ADR 0058's \
         amendment of 2026-11-19); raise the cap with a dated amendment stating the new count \
         and its adversarial security review, or remove code. Files: {files:?}"
    );
}

#[test]
fn no_counted_file_brings_in_a_file_the_count_does_not_see() {
    for (path, _) in base_files() {
        let text = std::fs::read_to_string(&path).unwrap();
        for (index, line) in text.lines().enumerate() {
            for source in UNCOUNTED_SOURCES {
                assert!(
                    !line.contains(source),
                    "{}:{}: `{source}` brings in a file that the count of the base code may not \
                     see (ADR 0058's amendment of 2026-11-19); write the code in a counted file",
                    path.display(),
                    index + 1
                );
            }
        }
    }
}

#[test]
fn the_crate_has_no_build_script() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(
        !root.join("build.rs").exists(),
        "`una-sys` has a build.rs: a build script could generate code that the count of the \
         base code does not see (ADR 0058's amendment of 2026-11-19)"
    );
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for line in manifest.lines() {
        let key = line.split('=').next().unwrap_or("").trim();
        assert!(
            key != "build",
            "the manifest of `una-sys` names a build script (`{line}`); a build script could \
             generate code that the count of the base code does not see"
        );
    }
}

#[test]
fn the_solver_module_is_compiled_only_with_its_feature() {
    let lib = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let text = std::fs::read_to_string(&lib).unwrap();
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut declarations = 0;
    for (index, line) in lines.iter().enumerate() {
        if line.starts_with("//") || !line.contains("mod solver") {
            continue;
        }
        declarations += 1;
        assert!(
            index > 0 && lines[index - 1] == "#[cfg(feature = \"solver\")]",
            "{}:{}: the module `solver` is declared without `#[cfg(feature = \"solver\")]` on \
             the line before it; its files would be base code that the base count leaves out",
            lib.display(),
            index + 1
        );
    }
    assert_eq!(declarations, 1, "lib.rs declares the module `solver` once");
}

/// The counted files with their physical line counts, sorted: every `.rs` file under `src`,
/// recursively, but those under `src/solver`.
fn base_files() -> Vec<(PathBuf, usize)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    visit(&src, &src.join("solver"), &mut files);
    assert!(
        !files.is_empty(),
        "no Rust file found under {}",
        src.display()
    );
    files.sort();
    files
}

/// Records the path and the physical line count of every `.rs` file under `dir`, recursively.
/// The directory `skipped` (the files of `solver`) is left out, as
/// `-not -path 'crates/una-sys/src/solver/*'` leaves it out of the ADR's command. A line is
/// counted as `wc -l` counts it: one per newline byte.
fn visit(dir: &Path, skipped: &Path, files: &mut Vec<(PathBuf, usize)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path != skipped {
                visit(&path, skipped, files);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            let bytes = std::fs::read(&path).unwrap();
            let lines = bytes.iter().filter(|&&b| b == b'\n').count();
            files.push((path, lines));
        }
    }
}
