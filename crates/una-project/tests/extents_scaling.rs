//! Scaling of the extents of `[boundaries]` (ADR 0093 rule 9): finding the boundary that holds
//! a module is one lookup per segment of its path, so checking a manifest's trusted modules
//! against its boundaries, and mapping modules to boundaries, grow with the modules and not with
//! modules times boundaries (#565). Timed by the shared helper of
//! `una-cli/tests/common/timing.rs`, so this file holds one test.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::time::{Duration, Instant};

#[path = "../../una-cli/tests/common/timing.rs"]
mod timing;

use una_project::parse_manifest;

/// A manifest of `n` boundaries and `n` trusted modules outside them, each four segments deep.
fn manifest(n: usize) -> String {
    let mut text = String::from(
        "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n\n[boundaries]\n",
    );
    for i in 0..n {
        text.push_str(&format!("\"b{i}\" = []\n"));
    }
    text.push_str("\n[trusted]\nmodules = [");
    for i in 0..n {
        text.push_str(&format!("\"t{i}.a.b.c\", "));
    }
    text.push_str("]\n");
    text
}

/// The time to parse [`manifest`]`(n)`, which checks each trusted module against the
/// boundaries, and to find the boundary of `n` modules below them.
fn timed(n: usize) -> Duration {
    let text = manifest(n);
    let started = Instant::now();
    let (manifest, errors) = parse_manifest(&text);
    assert!(errors.is_empty(), "{errors:?}");
    let extents = manifest.extents();
    for i in 0..n {
        let index = extents.of(&format!("b{i}.x.y.z")).unwrap();
        assert_eq!(manifest.boundaries[index].module.value, format!("b{i}"));
        assert_eq!(extents.of(&format!("t{i}.a.b.c")), None);
    }
    started.elapsed()
}

/// A doubling of the modules and the boundaries together takes less than 2.5 times as long: a
/// scan of the boundaries per module would take four.
#[test]
fn extents_scale_with_modules_not_modules_times_boundaries() {
    timing::assert_growth(
        "extents",
        &[1_000, 2_000, 4_000, 8_000],
        0.0,
        timing::FLOOR,
        2.5,
        |n| timed(n).as_secs_f64(),
    );
}
