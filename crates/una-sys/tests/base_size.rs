//! The size of the base code of `una-sys`: every Rust file under `crates/una-sys/src/` but those
//! of the feature `solver` (ADR 0058, amendments of 2026-10-25, 2026-10-28 and 2026-11-19). The
//! amendments cap that code at its measured physical count plus at most 5%, so that the trusted,
//! `unsafe`-bearing code cannot drift upward unnoticed. Until the amendment of 2026-11-19 nothing
//! enforced the count, and it passed the cap of 1,594 lines unseen; this test fails if the base
//! code grows past the cap. When a change needs more, raise [`CAP`] with a dated amendment of
//! ADR 0058 that states the new count, and with the adversarial security review that item 3 of
//! its amendment of 2026-11-03 requires for a raised cap, never silently. The files of `solver`
//! have their own cap, in `solver_size.rs`.
// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

/// The cap: the measured count of 1,668 lines (ADR 0058's amendment of 2026-11-19) plus at most
/// 5%, rounded down.
const CAP: usize = 1751;

#[test]
fn the_base_code_stays_within_its_capped_size() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut total = 0;
    let mut files = Vec::new();
    visit(&src, &src.join("solver"), &mut total, &mut files);
    assert!(
        !files.is_empty(),
        "no Rust file found under {}",
        src.display()
    );
    files.sort();
    assert!(
        total <= CAP,
        "the base code of `una-sys` is {total} physical lines, over the cap of {CAP} (ADR 0058's \
         amendment of 2026-11-19); raise the cap with a dated amendment stating the new count \
         and its adversarial security review, or remove code. Files: {files:?}"
    );
}

/// Adds the physical line count of every `.rs` file under `dir`, recursively, to `total`, and
/// records each file's path and count. The directory `skipped` (the files of `solver`) is left
/// out, as `-not -path 'crates/una-sys/src/solver/*'` leaves it out of the ADR's command. A
/// line is counted as `wc -l` counts it: one per newline byte.
fn visit(dir: &Path, skipped: &Path, total: &mut usize, files: &mut Vec<(String, usize)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path != skipped {
                visit(&path, skipped, total, files);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            let bytes = std::fs::read(&path).unwrap();
            let lines = bytes.iter().filter(|&&b| b == b'\n').count();
            *total += lines;
            files.push((path.display().to_string(), lines));
        }
    }
}
