//! ADR 0109 rule 8: the `overflow` obligations of a `systems` package are linear in its `Int`
//! operations, each a quantifier-free query of the size of a fixed-width one. A module with
//! 10, 100 and 1,000 `Int` operations, each proven, verifies in time linear in their number: the
//! cost per operation stays within a factor of 3 from one size to the next, ten times larger.
//! Only the verification is timed, and a miss is measured twice more (#341).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/timing.rs"]
mod timing;

use std::path::Path;
use std::time::Instant;

use una_cli::Tier;

/// A `systems` package of edition 0.4 in `dir` with `source` as its one module, `main`,
/// described by `ita.una` and locked by `ita.lock`, the lock of a conformance case of edition
/// 0.4. It has no `run` section, so it declares no budget.
fn package(dir: &Path, source: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        "ita scaling\n  version 0.1.0\n  edition 0.4\n  profile systems\n",
    )
    .unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/conformance/verification/until_measure/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/main.una"), source).unwrap();
}

/// A module whose `n` functions each add two bounded `Int` values: one `overflow` obligation
/// each, proven from the refinement of the operands.
fn additions(n: usize) -> String {
    let mut out = String::from("module main\n\npub kind Small = Int where it >= 0 and it <= 1000\n");
    for i in 0..n {
        out.push_str(&format!("\npub f{i}(a: Small, b: Small) -> Int = a + b\n"));
    }
    out
}

#[test]
fn int_overflow_obligations_verify_in_linear_time() {
    let dir = std::env::temp_dir().join(format!("una-systems-scaling-{}", std::process::id()));
    let verify = |n: usize| {
        package(&dir, &additions(n));
        let started = Instant::now();
        let checked = una_cli::check(&dir, Tier::One).unwrap();
        let seconds = started.elapsed().as_secs_f64();
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
        // Every operation carries its obligation, and every one is proven.
        let overflow: Vec<_> = checked
            .verification
            .iter()
            .flat_map(|verification| verification.obligations.iter().zip(&verification.outcomes))
            .filter(|(obligation, _)| obligation.kind == una_diag::ObligationKind::Overflow)
            .collect();
        assert_eq!(overflow.len(), n);
        assert!(
            overflow
                .iter()
                .all(|(_, outcome)| outcome.status == una_diag::ObligationStatus::Proven)
        );
        seconds
    };
    let fixed = timing::best_of_three(|| verify(1));
    // Ten times as many operations take about ten times as long past the fixed cost: within
    // 30, a cost per operation that grows by less than 3; quadratic growth would take 100.
    timing::assert_growth(
        "`Int` overflow obligations",
        &[10, 100, 1_000],
        fixed,
        1e-3,
        30.0,
        verify,
    );
    let _ = std::fs::remove_dir_all(&dir);
}
