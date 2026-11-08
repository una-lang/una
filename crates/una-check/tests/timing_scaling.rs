//! Scaling of the timing label of the sites (ADR 0116 amendment of 2026-10-25, E and I; slice 7
//! of #1087): `τ⁰` of each site comes from per-node summaries closed once per package
//! (`timing::Summaries`), not from a search of the call graph per site, which made `n` sites that
//! reach `n` functions O(n²). Three shapes, each at four sizes:
//! - a chain of `n` functions, each with a `together` whose branch calls the next under a
//!   `within`, so every site reaches the rest of the chain;
//! - one function with `n` `within`s beside one construct;
//! - `n` statements that a `limiter` scopes over, each reaching the rest of a chain of calls.
//!
//! Counted first (ADR 0077): the timing and pass steps, which the process-wide counters count,
//! must at most double with each doubling, with a margin of 10 %. Timed as a fallback, over a
//! span of 8 times the size, by the shared helper of `una-cli/tests/common/timing.rs` (#1197):
//! less than 24 times as long. The nested shape of section I (slice 8b) is counted only: each
//! doubling of its depth must raise the count by less than 3 times. This file holds
//! one test, so no other test of its binary moves the counters while it reads them.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "../../una-cli/tests/common/timing.rs"]
mod timing;

use una_check::{ModuleInput, PackageInput, check_names, check_types};
use una_project::parse_manifest;
use una_syntax::{Interner, KeywordSet, SourceDb, lex_in, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.4\"\n";

thread_local! {
    /// The frame steps of the last [`checked`] on this thread.
    static FRAMES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

const HEAD: &str = "module app\n\nimport std.time (millisecond)\n\nstep(n: Int) -> Int = n + 1\n";

/// A chain of `n` functions, each with a `together` whose first branch calls the next under a
/// `within`: that `within` reaches the rest of the chain.
fn chain(n: usize) -> String {
    let mut text = String::from(HEAD);
    for i in 0..n {
        let call = if i + 1 == n {
            "step(n)".to_owned()
        } else {
            format!("f{}(n)", i + 1)
        };
        text.push_str(&format!(
            "\npub f{i}(n: Int) -> Int\n=\n  together\n    let a = {call} within 10 * millisecond\n      if error use 0\n    let b = step(n)\n  a + b\n"
        ));
    }
    text
}

/// One function with `n` `within`s after one construct whose branches call `g`, which `run`
/// reaches from a part.
fn many(n: usize) -> String {
    let mut text = String::from(HEAD);
    text.push_str("\npub run(n: Int) -> Int\n=\n");
    for i in 0..n {
        text.push_str(&format!(
            "  let w{i} = step(n) within 10 * millisecond\n    if error use 0\n"
        ));
    }
    text.push_str("  n\n\npub outer(n: Int) -> Int\n=\n  together\n    let a = run(n)\n    let b = step(n)\n  a + b\n");
    text
}

/// `n` statements that a `limiter` scopes over, each calling `h<i>`, where `h<i>` calls
/// `h<i+1>`: each statement reaches the rest of the chain, so a search per statement takes
/// O(n²) in all, and the summaries O(n).
fn limiters(n: usize) -> String {
    let mut text = String::from(
        "module app\n\nimport std.limit (Limit, limiter)\nimport std.time (millisecond)\n",
    );
    for i in 0..n {
        let next = if i + 1 == n {
            "n".to_owned()
        } else {
            format!("h{}(n) + 1", i + 1)
        };
        text.push_str(&format!("\nh{i}(n: Int) -> Int = {next}\n"));
    }
    text.push_str("\npub run(n: Int) -> Int\n=\n");
    for i in 0..n {
        text.push_str(&format!(
            "  let v{i} = h{i}(n) with @x = limiter(at_most: 1, per: 1000 * millisecond, waiting: 0)\n"
        ));
    }
    text.push_str("  n\n");
    text
}

/// 400 functions, each with `tasks` statements nested `d` deep, a `let` at each level and, at
/// the innermost, a read of every outer binding (amendment I): the frames are `400 × d` and the
/// reads `400 × d`, each read free in the frames inside the one that binds it. Recording every
/// item in every open frame, or rebuilding every frame's seed from its subtree, costs
/// `400 × d²/2`; the labels bottom up and the sweep of the reads cost O(400 × d log d).
fn nested(d: usize) -> String {
    let mut text = String::from("module app\n");
    for f in 0..400 {
        text.push_str(&format!("\npub f{f}(n: Int) -> Unit\n=\n"));
        for level in 0..d {
            let pad = "  ".repeat(level + 1);
            text.push_str(&format!("{pad}let a{level} = n + {level}\n{pad}tasks\n"));
        }
        let pad = "  ".repeat(d + 1);
        let sum: Vec<String> = (0..d).map(|level| format!("a{level}")).collect();
        text.push_str(&format!("{pad}let z = {}\n{pad}unit\n", sum.join(" + ")));
        for level in (0..d).rev() {
            let pad = "  ".repeat(level + 1);
            text.push_str(&format!("{pad}unit\n"));
        }
    }
    text
}

/// [`nested`] with a `within` at each level, reading the level's binding: every round of the
/// probe of pass 2 computes the labels of every frame from the summaries (`Summaries::frames`)
/// for these sites, whose rebuilding per frame would cost `400 × d²/2`.
fn nested_within(d: usize) -> String {
    let mut text = String::from(
        "module app\n\nimport std.time (millisecond)\n\nstep(n: Int) -> Int = n + 1\n",
    );
    for f in 0..400 {
        text.push_str(&format!("\npub f{f}(n: Int) -> Unit\n=\n"));
        for level in 0..d {
            let pad = "  ".repeat(level + 1);
            text.push_str(&format!(
                "{pad}let a{level} = n + {level}\n{pad}let w{level} = step(a{level}) within 10 * millisecond\n{pad}  if error use 0\n{pad}tasks\n"
            ));
        }
        let pad = "  ".repeat(d + 1);
        let sum: Vec<String> = (0..d).map(|level| format!("a{level}")).collect();
        text.push_str(&format!("{pad}let z = {}\n{pad}unit\n", sum.join(" + ")));
        for level in (0..d).rev() {
            let pad = "  ".repeat(level + 1);
            text.push_str(&format!("{pad}unit\n"));
        }
    }
    text
}

/// The codes of the package `text`, and the timing, pass and frame steps its type check took.
fn checked(text: &str) -> (Vec<String>, usize) {
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let manifest = db.add("una.toml", MANIFEST.as_bytes());
    // Edition 0.4 is described by `ita.una`; the settings `una.toml` gives are the same, and
    // the project error that names `ita.una` does not matter here, as in `tests/dense.rs`.
    let settings = parse_manifest(MANIFEST).0;
    let id = db.add("src/app.una", text.as_bytes());
    let lexed = lex_in(db.file(id), id, &mut interner, KeywordSet::V0_4);
    let parsed = parse(&lexed, id, &interner);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    let input = PackageInput {
        sources: &db,
        interner: &interner,
        single: false,
        modules: vec![ModuleInput {
            id,
            tree: &parsed.file,
            broken: false,
        }],
        manifest: Some(manifest),
        settings: &settings,
        run: None,
        earlier_errors: false,
        locks: &[],
    };
    assert!(check_names(&input).is_empty());
    let steps = || {
        una_core::label_counters::timing_steps()
            + una_core::label_counters::pass_steps()
            + una_core::label_counters::frame_steps()
    };
    let before = steps();
    let frames_before = una_core::label_counters::frame_steps();
    let check = check_types(&input);
    let steps = steps() - before;
    FRAMES.with(|slot| slot.set(una_core::label_counters::frame_steps() - frames_before));
    (
        check
            .diagnostics
            .iter()
            .map(|d| format!("{} {}", d.code.as_str(), d.message))
            .collect(),
        steps,
    )
}

#[test]
fn the_timing_label_of_the_sites_scales_linearly() {
    let sizes = [250, 500, 1_000, 2_000];
    for (shape, make) in [
        ("a chain", chain as fn(usize) -> String),
        ("many sites", many),
        ("`limiter` statements", limiters),
    ] {
        let counts: Vec<usize> = sizes
            .iter()
            .map(|&n| {
                let (codes, steps) = checked(&make(n));
                assert_eq!(codes, Vec::<String>::new(), "{shape} of {n}");
                steps
            })
            .collect();
        timing::assert_count_growth(shape, &sizes, &counts, 1.1);
        let span = [sizes[0], sizes[3]];
        timing::assert_growth(shape, &span, 0.0, timing::FLOOR, 24.0, |n| {
            let (text, at) = (make(n), sizes.iter().position(|&size| size == n).unwrap());
            let ((codes, steps), took) = timing::timed(|| checked(&text));
            assert_eq!(codes, Vec::<String>::new(), "{shape} of {n}");
            // The count is deterministic: the same in every run.
            assert_eq!(
                steps, counts[at],
                "{shape} of {n}: the steps of a second run"
            );
            took
        });
    }
    // Nested sites, counted (amendment I): each doubling of `d`, from `d` of 8, raises the count
    // by less than 3 times; `d²` growth reaches at least 3 at some doubling of these sizes. The
    // parser's cap on nesting allows the depth, which the test checks first.
    let depths = [8, 16, 32, 64];
    for (shape, make) in [
        ("nested sites", nested as fn(usize) -> String),
        ("nested sites with `within`s", nested_within),
    ] {
        let counts: Vec<usize> = depths
            .iter()
            .map(|&d| {
                let text = make(d);
                let (codes, steps) = std::thread::Builder::new()
                    .stack_size(256 << 20)
                    .spawn(move || {
                        let (codes, _) = checked(&text);
                        // The frame steps alone: the labels of the frames, the sweep of the
                        // reads, the search for a read's outermost frame and every rebuild of
                        // a seed, which the other counters would drown at these sizes.
                        (codes, FRAMES.with(std::cell::Cell::get))
                    })
                    .unwrap()
                    .join()
                    .unwrap();
                assert_eq!(codes, Vec::<String>::new(), "{shape} {d} deep");
                steps
            })
            .collect();
        for (pair, sizes) in counts.windows(2).zip(depths.windows(2)) {
            assert!(
                pair[1] < 3 * pair[0],
                "{shape}: from {} to {} deep the count went from {} to {}, 3 times or more (counts {counts:?})",
                sizes[0],
                sizes[1],
                pair[0],
                pair[1]
            );
        }
        // Sharper, as the copying of `d²` items is a small part of these counts at these
        // depths: per frame step of the bound O(F log F), for the `F = 400 × 2d` frames, the
        // count stays within 25 % from `d` of 8 to `d` of 64, where a rebuild of every frame's
        // seed, or a linear search for a read's outermost frame, rises by about half.
        let per: Vec<f64> = counts
            .iter()
            .zip(depths)
            .map(|(&count, d)| {
                // `ilog2`, an integer logarithm: no float function of the platform's libm.
                let frames = 400 * 2 * d;
                count as f64 / (frames as f64 * f64::from(frames.ilog2()))
            })
            .collect();
        let (low, high) = per.iter().fold((f64::MAX, 0.0f64), |(low, high), &x| {
            (low.min(x), high.max(x))
        });
        assert!(
            high < 1.25 * low,
            "{shape}: the count per F log F of the frames went from {low:.1} to {high:.1}, past 1.25 times (counts {counts:?})"
        );
    }
}
