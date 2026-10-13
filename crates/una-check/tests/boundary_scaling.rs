//! Scaling of the checks of calls into a boundary (ADR 0093): each call's grants are checked
//! once, and the releases reachable from the calls are found by one reverse search of the call
//! graph, so a package with a chain of `n` boundary functions and `n` calls into them is checked
//! in linear time (#565). Timed, so this file holds one test.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::time::{Duration, Instant};

use una_check::{ModuleInput, PackageInput, check_names, check_types};
use una_project::parse_manifest;
use una_syntax::{Interner, SourceDb, lex, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n\n[boundaries]\n\"vendor\" = []\n\n[trusted]\nmodules = [\"release\"]\n";

const RELEASE: &str = "trusted module release\n\npub policy reveal\n  from secret Int\n  to public Int\n  via open\n  purpose \"scale\"\n  owner ops\n\nopen(n: secret Int) -> public Int = n\n";

/// A boundary module of `n` functions, each calling the next, the last one releasing.
fn vendor(n: usize) -> String {
    let mut text = String::from("module vendor\n\nimport release (reveal)\n");
    for i in 0..n {
        let body = if i + 1 == n {
            "reveal(n)".to_owned()
        } else {
            format!("f{}(n)", i + 1)
        };
        text.push_str(&format!("\npub f{i}(n: secret Int) -> Int = {body}\n"));
    }
    text
}

/// A module of `n` functions, each calling into the boundary.
fn app(n: usize) -> String {
    let names: Vec<String> = (0..n).map(|i| format!("f{i}")).collect();
    let mut text = format!("module app\n\nimport vendor ({})\n", names.join(", "));
    for i in 0..n {
        text.push_str(&format!("\npub g{i}(n: secret Int) -> Int = f{i}(n)\n"));
    }
    text
}

/// The codes `check_types` reports for the package of size `n`, and the time it took.
fn checked(n: usize) -> (Vec<String>, Duration) {
    let texts = [
        ("src/release.una", RELEASE.to_owned()),
        ("src/vendor.una", vendor(n)),
        ("src/app.una", app(n)),
    ];
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let manifest = db.add("una.toml", MANIFEST.as_bytes());
    let (settings, errors) = parse_manifest(MANIFEST);
    assert!(errors.is_empty(), "{errors:?}");
    let ids: Vec<_> = texts
        .iter()
        .map(|(path, text)| db.add(*path, text.as_bytes()))
        .collect();
    let parsed: Vec<_> = ids
        .iter()
        .map(|&id| {
            let lexed = lex(db.file(id), id, &mut interner);
            let parsed = parse(&lexed, id, &interner);
            assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
            parsed
        })
        .collect();
    let input = PackageInput {
        sources: &db,
        interner: &interner,
        single: false,
        modules: ids
            .iter()
            .zip(&parsed)
            .map(|(&id, parsed)| ModuleInput {
                id,
                tree: &parsed.file,
                broken: false,
            })
            .collect(),
        manifest: Some(manifest),
        settings: &settings,
        run: None,
        earlier_errors: false,
        locks: &[],
    };
    assert!(check_names(&input).is_empty());
    let started = Instant::now();
    let diagnostics = check_types(&input).diagnostics;
    let took = started.elapsed();
    (
        diagnostics
            .iter()
            .map(|d| d.code.as_str().to_owned())
            .collect(),
        took,
    )
}

/// A doubling of the package may not take more than 2.5 times as long, and every call into the
/// boundary reaches the release at the end of the chain: one U3024 each.
#[test]
fn boundary_checks_scale_linearly() {
    let sizes = [250, 500, 1_000, 2_000];
    let times: Vec<Duration> = sizes
        .iter()
        .map(|&n| {
            (0..3)
                .map(|_| {
                    let (codes, took) = checked(n);
                    assert_eq!(codes, vec!["U3024".to_owned(); n]);
                    took
                })
                .min()
                .unwrap()
        })
        .collect();
    for (pair, sizes_pair) in times.windows(2).zip(sizes.windows(2)) {
        let (small, large) = (pair[0], pair[1]);
        assert!(
            large.as_secs_f64() <= 2.5 * small.as_secs_f64() + 0.02,
            "{} calls took {large:?}, {} took {small:?}",
            sizes_pair[1],
            sizes_pair[0]
        );
    }
}
