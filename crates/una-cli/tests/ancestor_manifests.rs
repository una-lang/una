//! A manifest above the build or the checkout does not change the tests (#1560): an `ita.una`
//! of edition 0.4 planted above a temporary fixture changes the edition of an unsealed single
//! file, as the upward search of `spec/30` says, and changes nothing below a seal of
//! `common/hermetic.rs`, for the two failures the stray manifest caused: the U3014 example of
//! `explain_examples` and the verifier's corpus cross-check on
//! `verification/fn_type_join_not_refined.una`.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/hermetic.rs"]
mod hermetic;

use std::path::{Path, PathBuf};

use una_cli::Tier;
use una_diag::Code;

/// The stray manifest of #1560: an `ita.una` of edition 0.4.
const PLANTED: &str = "ita leak\n  version 0.1.0\n  edition 0.4\n";

/// A fresh directory under `CARGO_TARGET_TMPDIR` with [`PLANTED`] in it.
fn planted(name: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("ancestor-manifests")
        .join(name);
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("ita.una"), PLANTED).unwrap();
    base
}

/// The single `una` block under `heading` in the explain text of `code`.
fn example(code: Code, heading: &str) -> String {
    let section = una_explain::text(code)
        .split(heading)
        .nth(1)
        .unwrap()
        .split("\n## ")
        .next()
        .unwrap();
    let body = section.split("```").nth(1).unwrap();
    body.split_once('\n').unwrap().1.to_owned()
}

/// The codes of `una check` on `file` at tier 0.
fn codes(file: &Path) -> Vec<Code> {
    una_cli::check(file, Tier::Zero)
        .unwrap()
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

/// The sites where the checker records an obligation and those where tier 1 generates one, for
/// the single file at `path`, in edition 0.1's layout, as `verification.rs` compares them.
fn sites(path: &Path) -> (Vec<String>, Vec<String>) {
    let checked = una_cli::check(path, Tier::Zero).unwrap();
    assert_eq!(checked.diagnostics, [], "{}", path.display());
    let obligations = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn_scoped(scope, || {
                una_verify::obligations(
                    &checked.program,
                    &checked.sources,
                    una_verify::SourceLayout::Joined,
                )
            })
            .unwrap()
            .join()
            .unwrap()
    })
    .unwrap();
    let key = |kind: una_diag::ObligationKind, location: una_diag::Location| {
        format!(
            "{} {}:{}:{}",
            kind.as_str(),
            location.file,
            location.start.line,
            location.start.col
        )
    };
    let mut pending: Vec<String> = checked
        .pending
        .iter()
        .map(|p| key(p.kind, p.location.clone()))
        .collect();
    let mut generated: Vec<String> = Vec::new();
    for obligation in &obligations {
        let spans = if obligation.paths.is_empty() {
            vec![obligation.source]
        } else {
            obligation.paths.clone()
        };
        for span in spans {
            generated.push(key(obligation.kind, checked.sources.location(span)));
        }
    }
    pending.sort();
    pending.dedup();
    generated.sort();
    generated.dedup();
    (pending, generated)
}

#[test]
fn a_planted_manifest_reaches_an_unsealed_file_and_no_sealed_one() {
    let base = planted("explain");
    let failing = example(Code::U3014, "## Failing example");
    let fixed = example(Code::U3014, "## Fixed example");
    // Unsealed, the file takes the planted edition 0.4, where `example` is written `test`: the
    // failure of #1560, which shows that the plant is effective.
    let unsealed = base.join("unsealed").join("failing.una");
    std::fs::create_dir_all(unsealed.parent().unwrap()).unwrap();
    std::fs::write(&unsealed, &failing).unwrap();
    assert!(
        codes(&unsealed).contains(&Code::U0103),
        "{:?}",
        codes(&unsealed)
    );
    // Sealed, as `explain_examples` writes its examples, it keeps edition 0.1.
    let sealed = base.join("sealed");
    hermetic::seal(&sealed);
    let file = sealed.join("U3014").join("failing.una");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, &failing).unwrap();
    assert_eq!(codes(&file), [Code::U3014]);
    let file = sealed.join("U3014").join("fixed.una");
    std::fs::write(&file, &fixed).unwrap();
    assert_eq!(codes(&file), []);
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_sealed_copy_of_the_corpus_ignores_a_planted_manifest() {
    let base = planted("corpus");
    let case = "tests/conformance/verification/fn_type_join_not_refined.una";
    // Unsealed, the case takes edition 0.4, whose layout tier 1 does not use here: the checker
    // and tier 1 disagree on its sites, the failure of #1560.
    let unsealed = base.join("unsealed").join("fn_type_join_not_refined.una");
    std::fs::create_dir_all(unsealed.parent().unwrap()).unwrap();
    std::fs::copy(hermetic::repository().join(case), &unsealed).unwrap();
    let (pending, generated) = sites(&unsealed);
    assert_ne!(pending, generated);
    // Sealed, as the runner and `verification.rs` read the corpus, the two agree.
    let root = hermetic::checkout_under(&base.join("sealed"), &["tests/conformance"]);
    assert_eq!(
        hermetic::manifest_above(&root.join(case)),
        Some(base.join("sealed").join("una.toml").canonicalize().unwrap())
    );
    let (pending, generated) = sites(&root.join(case));
    assert!(!pending.is_empty());
    assert_eq!(pending, generated);
    std::fs::remove_dir_all(&base).unwrap();
}
