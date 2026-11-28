//! Reading a manifest is linear in its size, with the profile and the declared budget (ADR
//! 0109 rules 1 and 4, design note `docs/plan/0109-s1-design.md` §3 and §4): `ita.una` with `n`
//! `needs` lines, and `una.toml` with `n` label categories, each beside `profile systems`, a run
//! entry and `max_memory`. The profile and the budget add O(1); a quadratic pass over the lines
//! would take four times as long at twice the size, a linear one twice, so the bound, 3, lies
//! between the two. Each measurement is the shortest of five reads, and a miss is measured twice
//! more (#341).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/timing.rs"]
mod timing;

use una_project::ita::parse_ita_manifest;
use una_project::{Profile, parse_manifest};

/// `ita.una` of a `systems` package with a run entry, its budget, and `n` needs.
fn ita_text(n: usize) -> String {
    let mut text =
        String::from("ita scaling\n  version 0.1.0\n  edition 0.4\n  profile systems\n\nneeds\n");
    for i in 0..n {
        text.push_str(&format!("  dep{i} 1.0\n"));
    }
    text.push_str("\nrun\n  entry main\n  max_memory 64\n");
    text
}

/// `una.toml` of edition 0.3, a `systems` package with a run entry, its budget, and `n`
/// label categories.
fn toml_text(n: usize) -> String {
    let categories: Vec<String> = (0..n).map(|i| format!("\"Cat{i}\"")).collect();
    format!(
        "[package]\nname = \"scaling\"\nversion = \"0.1.0\"\nedition = \"0.3\"\nprofile = \"systems\"\n\n[labels]\ncategories = [{}]\n\n[run]\nentry = \"main\"\nmax_memory = 64\n",
        categories.join(", ")
    )
}

/// Reads `ita.una` text, keeping the result from the optimizer.
fn read_ita(text: &str) {
    drop(std::hint::black_box(parse_ita_manifest(
        una_syntax::ita::Text::Written(text),
    )));
}

/// Reads `una.toml` text, keeping the result from the optimizer.
fn read_toml(text: &str) {
    drop(std::hint::black_box(parse_manifest(text)));
}

/// The shortest of five calls of `read`, in seconds.
fn seconds(mut read: impl FnMut()) -> f64 {
    (0..5)
        .map(|_| timing::timed(&mut read).1)
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn ita_una_with_a_profile_and_a_budget_reads_in_linear_time() {
    let text = ita_text(4);
    let (manifest, _) = parse_ita_manifest(&text);
    assert_eq!(manifest.profile, Profile::Systems);
    assert_eq!(manifest.max_memory, Some(64));
    let fixed = timing::best_of_three(|| seconds(|| read_ita(&text)));
    let texts: Vec<(usize, String)> = [2_000, 4_000].map(|n| (n, ita_text(n))).into();
    timing::assert_growth("needs lines", &[2_000, 4_000], fixed, 1e-6, 3.0, |n| {
        let text = &texts.iter().find(|(size, _)| *size == n).unwrap().1;
        let (manifest, _) = parse_ita_manifest(text);
        assert_eq!(manifest.max_memory, Some(64));
        seconds(|| read_ita(text))
    });
}

#[test]
fn una_toml_with_a_profile_and_a_budget_reads_in_linear_time() {
    let text = toml_text(4);
    let (manifest, errors) = parse_manifest(&text);
    assert_eq!(errors, []);
    assert_eq!(manifest.profile, Profile::Systems);
    assert_eq!(manifest.max_memory, Some(64));
    let fixed = timing::best_of_three(|| seconds(|| read_toml(&text)));
    let texts: Vec<(usize, String)> = [2_000, 4_000].map(|n| (n, toml_text(n))).into();
    timing::assert_growth("label categories", &[2_000, 4_000], fixed, 1e-6, 3.0, |n| {
        let text = &texts.iter().find(|(size, _)| *size == n).unwrap().1;
        let (manifest, errors) = parse_manifest(text);
        assert_eq!(errors, []);
        assert_eq!(manifest.categories.len(), n);
        seconds(|| read_toml(text))
    });
}
