//! Scaling of `L_clk` (ADR 0116 amendment of 2026-11-09, #898, C1, C5): the label of each
//! reading of `Clock.now` covers the bodies of every ancestor of its function, computed by the
//! two passes of strongly connected components that `L_lim` uses, never by a search per
//! reading, inside the probe of pass 2, whose rounds stop once no label rises. A chain of `n`
//! functions, each reading the clock and calling the next, under a `top` that works on a
//! `secret` first, has `n` readings with up to `n` ancestors each, so a search per reading
//! would take quadratic time. The process-wide counter `pass_steps` must grow linearly; this
//! file holds one test, so no other test of its binary moves the counter while it reads it.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "../../una-cli/tests/common/timing.rs"]
mod timing;

use una_check::{ModuleInput, PackageInput, check_names, check_types};
use una_project::parse_manifest;
use una_syntax::{Interner, KeywordSet, SourceDb, lex_in, parse};

/// Edition 0.4. It is described by `ita.una`; the settings `una.toml` gives are the same, and
/// the project error that names `ita.una` does not matter here, as in `tests/limit_scaling.rs`.
const MANIFEST_0_4: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.4\"\n";

/// `n` functions, each reading the clock and calling the next, under a `top` that works on a
/// `secret` before it calls the first: an ancestor of every reading.
fn app(n: usize) -> String {
    let mut text = String::from("module app\n");
    for i in 0..n {
        let next = if i + 1 == n {
            "unit".to_owned()
        } else {
            format!("f{}()", i + 1)
        };
        text.push_str(&format!(
            "\nf{i}() -> Unit\n  uses @clock: Clock[now]\n=\n  let now = @clock.now()\n  {next}\n"
        ));
    }
    text.push_str(
        "\npub top(key: secret Int) -> Unit\n  uses @clock: Clock[now]\n=\n  let doubled = key * 2\n  f0()\n",
    );
    text
}

/// The codes, the `Clock.now` entries of the report at `secret`, and the pass steps of the
/// type check of the chain of `n`.
fn checked(n: usize) -> (Vec<String>, usize, usize) {
    let text = app(n);
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let manifest = db.add("una.toml", MANIFEST_0_4.as_bytes());
    let (settings, errors) = parse_manifest(MANIFEST_0_4);
    assert!(errors.len() <= 1, "{errors:?}");
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
    let steps = una_core::label_counters::pass_steps();
    let check = check_types(&input);
    let steps = una_core::label_counters::pass_steps() - steps;
    let secret = una_core::Label {
        level: una_syntax::ast::Level::Secret,
        ..una_core::Label::public()
    };
    let readings = check
        .timing
        .iter()
        .filter(|site| site.construct == "Clock.now" && site.label == secret)
        .count();
    (
        check
            .diagnostics
            .iter()
            .map(|d| d.code.as_str().to_owned())
            .collect(),
        readings,
        steps,
    )
}

/// Every reading is `secret`, raised by `top`, and a doubling of the chain at most doubles the
/// pass steps, with a margin of 10 % for the steps that do not grow.
#[test]
fn the_labels_of_readings_scale_linearly() {
    let sizes = [250, 500, 1_000, 2_000];
    let counts: Vec<usize> = sizes
        .iter()
        .map(|&n| {
            let (codes, readings, steps) = checked(n);
            assert_eq!(codes, Vec::<String>::new(), "{n}");
            assert_eq!(readings, n, "{n}: every reading is `secret`");
            steps
        })
        .collect();
    timing::assert_count_growth("the labels of readings", &sizes, &counts, 1.1);
}
