//! The cost ADR 0094 states (Consequences, Complexity): one label comparison per operation of a
//! need at each call, so checking a chain of functions that each pass a labeled binding to the
//! next grows linearly with the chain. Only the check is timed, and a miss is measured twice
//! more (#341). The inference of labels is counted first (ADR 0077): the process-wide counter
//! `gives_steps` counts the calls to inferring functions taken from its worklist, which only
//! the second test's helpers are, so the first test never moves it.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/timing.rs"]
mod timing;

use std::path::Path;
use std::time::Instant;

use una_cli::Tier;

/// A package of edition 0.3 in `dir` with `source` as its one module, `m`, declaring the
/// category `Service`.
fn package(dir: &Path, source: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        "[package]\nname = \"scaling\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n\n[labels]\ncategories = [\"Service\"]\n",
    )
    .unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verification.lock"),
        dir.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/m.una"), source).unwrap();
}

/// A chain of `n` functions: each passes its labeled binding to the next by name, and the last
/// reads the labeled operation.
fn chain(n: usize) -> String {
    let mut source =
        String::from("module m\n\npub capability Files\n  op owner(file: Text) -> Text\n\n");
    for i in 0..n {
        let body = if i + 1 == n {
            "@files.owner(file)".to_owned()
        } else {
            format!("f{}(file)", i + 1)
        };
        if i > 0 {
            source.push('\n');
        }
        source.push_str(&format!(
            "pub f{i}(file: Text) -> internal[Service] Text\n  uses @files: Files[owner gives internal[Service]]\n=\n  {body}\n"
        ));
    }
    source
}

/// The seconds a tier-0 check of `source` takes, the best of three.
fn check_seconds(dir: &Path, source: &str) -> f64 {
    package(dir, source);
    (0..3)
        .map(|_| {
            let started = Instant::now();
            let checked = una_cli::check(dir, Tier::Zero).unwrap();
            assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
            started.elapsed().as_secs_f64()
        })
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn passing_a_labeled_binding_along_a_chain_costs_one_comparison_per_call() {
    // From 200 to 1,600 functions, eight times as many calls take about eight times as long; a
    // comparison against every function would take about 64 times. The bound, 24, leaves a
    // wide margin, as #329's does.
    let dir = std::env::temp_dir().join(format!("una-gives-chain-{}", std::process::id()));
    let fixed = check_seconds(&dir, &chain(2));
    timing::assert_growth("functions", &[200, 1_600], fixed, 1e-6, 24.0, |n| {
        check_seconds(&dir, &chain(n))
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// An entry point and a chain of `n` private helpers without `uses` clauses, the last of which
/// reads `owner`. With `labeled`, the entry point labels `owner` and each helper's labels are
/// inferred from the one before (ADR 0094 rule 3, #566 part 2); without, the same chain
/// infers only its `uses`. `reversed` writes the helpers last to first, so that the inference
/// cannot ride on source order.
fn inferred_chain(n: usize, labeled: bool, reversed: bool) -> String {
    let label = if labeled { "internal[Service] " } else { "" };
    let mut helpers: Vec<String> = (0..n)
        .map(|i| {
            let body = if i + 1 == n {
                "@files.owner(file)".to_owned()
            } else {
                format!("g{}(file)", i + 1)
            };
            format!("g{i}(file: Text) -> {label}Text = {body}\n")
        })
        .collect();
    if reversed {
        helpers.reverse();
    }
    let gives = if labeled {
        " gives internal[Service]"
    } else {
        ""
    };
    let mut source =
        String::from("module m\n\npub capability Files\n  op owner(file: Text) -> Text\n\n");
    source.push_str(&format!(
        "pub entry(file: Text) -> {label}Text\n  uses @files: Files[owner{gives}]\n=\n  g0(file)\n"
    ));
    for helper in helpers {
        source.push('\n');
        source.push_str(&helper);
    }
    source
}

/// The label inference steps of a tier-0 check of `source`, the same in each of three runs,
/// and its seconds, the best of three.
fn inferred_check(dir: &Path, source: &str) -> (usize, f64) {
    package(dir, source);
    let runs: Vec<(usize, f64)> = (0..3)
        .map(|_| {
            let steps = una_core::label_counters::gives_steps();
            let started = Instant::now();
            let checked = una_cli::check(dir, Tier::Zero).unwrap();
            let took = started.elapsed().as_secs_f64();
            assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
            (una_core::label_counters::gives_steps() - steps, took)
        })
        .collect();
    // The count is deterministic: the same in every run.
    assert!(runs.iter().all(|run| run.0 == runs[0].0), "{runs:?}");
    let best = runs.iter().map(|run| run.1).fold(f64::INFINITY, f64::min);
    (runs[0].0, best)
}

#[test]
fn inferring_labels_along_a_chain_of_helpers_adds_a_bounded_share() {
    // The labels flow from the entry point down the chain over a worklist of calls, linear in
    // the calls. Counted first: each call to a helper is taken once, and once more when its
    // caller's labels change, so the labeled chain takes at most about two steps per call in
    // either source order, and 8 times the helpers at most about 8 times the steps; a label
    // pass per helper over every call would take about `n` steps per call. The `uses`
    // inference the labels ride on has its own cost on such a chain, so the time, the
    // fallback, compares the labeled chain with the same chain unlabeled: the labels add at
    // most a constant factor, and a miss is measured twice more, keeping each one's best
    // (#341).
    let dir = std::env::temp_dir().join(format!("una-gives-inferred-{}", std::process::id()));
    for reversed in [false, true] {
        let mut counted = Vec::new();
        for n in [200, 1_600] {
            let (_, mut plain) = inferred_check(&dir, &inferred_chain(n, false, reversed));
            let (steps, mut labeled) = inferred_check(&dir, &inferred_chain(n, true, reversed));
            let calls = n;
            assert!(
                steps >= calls,
                "the counter counts: {steps} steps, {calls} calls"
            );
            assert!(
                steps <= 2 * calls + 2,
                "{n} helpers (reversed: {reversed}): {steps} label inference steps for {calls} calls"
            );
            counted.push(steps);
            if labeled > 2.0 * plain + 0.05 {
                for _ in 0..2 {
                    plain = plain.min(inferred_check(&dir, &inferred_chain(n, false, reversed)).1);
                    labeled =
                        labeled.min(inferred_check(&dir, &inferred_chain(n, true, reversed)).1);
                }
            }
            assert!(
                labeled <= 2.0 * plain + 0.05,
                "{n} helpers (reversed: {reversed}): labeled {labeled:.3}s against {plain:.3}s unlabeled, best of three"
            );
        }
        assert!(
            counted[1] as f64 <= 8.8 * counted[0] as f64,
            "reversed: {reversed}: 1,600 helpers took {} steps, 200 took {}",
            counted[1],
            counted[0]
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// One function that reads `n` labeled answers and sends `n` values of the answers' label to
/// an operation that accepts `public` data: `n` U4003, each naming an answer (ADR 0094 rule 7).
fn reports(n: usize) -> String {
    let mut source = String::from(
        "module m\n\npub capability Files\n  op owner(file: Text) -> Text\n\npub capability Mailer\n  op send(to: Text) -> Unit\n\npub leak(file: Text, pin: internal[Service] Text) -> Unit\n  uses @files: Files[owner gives internal[Service]]\n  uses @mailer: Mailer[send]\n=\n",
    );
    for i in 0..n {
        source.push_str(&format!("  let a{i} = @files.owner(file)\n"));
    }
    for _ in 0..n {
        source.push_str("  @mailer.send(to: pin)\n");
    }
    source.push_str("  unit\n");
    source
}

/// The seconds a tier-0 check of `source` takes, the best of three, with `n` U4003 reported.
fn report_seconds(dir: &Path, source: &str, n: usize) -> f64 {
    package(dir, source);
    (0..3)
        .map(|_| {
            let started = Instant::now();
            let checked = una_cli::check(dir, Tier::Zero).unwrap();
            let named = checked
                .diagnostics
                .iter()
                .filter(|d| {
                    d.code == una_diag::Code::U4003
                        && d.message.contains("the answer of `@files.owner`")
                })
                .count();
            assert_eq!(named, n, "{:?}", checked.diagnostics.first());
            started.elapsed().as_secs_f64()
        })
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn naming_the_answer_of_each_flow_report_stays_linear() {
    // ADR 0094 rule 7: each report names an answer by its label, and the checker keeps one
    // answer per distinct label, so `n` answers and `n` reports cost linear time. From 250 to
    // 4,000 of each, sixteen times as many take about sixteen times as long; scanning every
    // answer at every report (the first version, review of #1536) takes far more there, and
    // 256 times once the scan dominates. The bound, 40, leaves a wide margin.
    let dir = std::env::temp_dir().join(format!("una-gives-reports-{}", std::process::id()));
    let fixed = report_seconds(&dir, &reports(2), 2);
    timing::assert_growth(
        "answers and reports",
        &[250, 4_000],
        fixed,
        1e-6,
        40.0,
        |n| report_seconds(&dir, &reports(n), n),
    );
    let _ = std::fs::remove_dir_all(&dir);
}
