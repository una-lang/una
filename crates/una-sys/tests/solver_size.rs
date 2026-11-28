//! The size of the files behind the feature `solver` (ADR 0124 rule 35 and its amendments of
//! 2026-10-27, 2026-11-01 and 2026-11-28, ADR 0058's note of 2026-11-01). The amendment caps
//! those files at their measured physical count plus at most 5%, so that the trusted,
//! `unsafe`-bearing confinement code cannot drift upward unnoticed. Nothing else enforced that
//! count, so this test does: it fails if the files under `crates/una-sys/src/solver/` grow past
//! the cap. When a change needs more, raise [`CAP`] with a dated amendment that states the new
//! count, never silently.
// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

/// The cap: the measured count of 1,908 lines (ADR 0124's amendment of 2026-11-28, which counts
/// the end by `SIGXCPU` of #2268 and the reading of the kernel's kill of #2321) plus 5%. Before
/// it, 1,687 lines plus 5%, 1,768 (the amendment of 2026-11-01).
const CAP: usize = 2003;

#[test]
fn the_solver_files_stay_within_their_capped_size() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/solver");
    let mut total = 0;
    let mut files = Vec::new();
    visit(&dir, &mut total, &mut files);
    assert!(
        !files.is_empty(),
        "no Rust file found under {}",
        dir.display()
    );
    files.sort();
    assert!(
        total <= CAP,
        "the files behind `solver` are {total} physical lines, over the cap of {CAP} \
         (ADR 0124's amendment of 2026-11-28); raise the cap with a dated amendment stating \
         the new count, or split the code. Files: {files:?}"
    );
}

/// Adds the physical line count of every `.rs` file under `dir`, recursively, to `total`, and
/// records each file's path and count.
fn visit(dir: &Path, total: &mut usize, files: &mut Vec<(String, usize)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            visit(&path, total, files);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let lines = std::fs::read_to_string(&path).unwrap().lines().count();
            *total += lines;
            files.push((path.display().to_string(), lines));
        }
    }
}
