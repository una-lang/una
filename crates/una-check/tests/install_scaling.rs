//! Scaling of the charge of a handler at its installation (ADR 0093 rule 15): the summary of
//! what each binding leads to is closed by a worklist in which each pair of a binding and a
//! boundary follows each edge once, so a chain of `n` functions that grant a binding on by name
//! down to one call into a boundary is checked in linear time (#669). Timed, so this file holds
//! one test.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::time::{Duration, Instant};

use una_check::{ModuleInput, PackageInput, check_names, check_types};
use una_project::parse_manifest;
use una_syntax::{Interner, SourceDb, lex, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n\n[boundaries]\n\"vendor\" = [\"Log[write]\"]\n";

const CAPS: &str = "module caps\n\npub capability Log\n  op write(n: Int) -> Int\n\npub capability Disk\n  op put(n: Int) -> Int\n";

const VENDOR: &str = "module vendor\n\nimport caps (Log)\n\npub show(n: Int) -> Int\n  uses @log: Log[write]\n=\n  @log.write(n)\n";

/// A module of `n` functions, each granting its `@log` on by name to the next, the last one to
/// the boundary, and a handler outside the ceiling installed above the first.
fn app(n: usize) -> String {
    let mut text = String::from(
        "module app\n\nimport caps (Disk, Log)\nimport vendor (show)\n\nhandler heavy() for Log\n  uses @disk: Disk[put]\n  op write(n) = @disk.put(n)\n",
    );
    for i in 0..n {
        let body = if i + 1 == n {
            "show(n) with @log = @log".to_owned()
        } else {
            format!("f{}(n)", i + 1)
        };
        text.push_str(&format!(
            "\nf{i}(n: Int) -> Int\n  uses @log: Log[write]\n=\n  {body}\n"
        ));
    }
    text.push_str(
        "\npub top(n: Int) -> Int\n  uses @disk: Disk[put]\n=\n  f0(n) with @log = heavy()\n",
    );
    text
}

/// The codes `check_types` reports for the package of size `n`, and the time it took.
fn checked(n: usize) -> (Vec<String>, Duration) {
    let texts = [
        ("src/caps.una", CAPS.to_owned()),
        ("src/vendor.una", VENDOR.to_owned()),
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

/// A doubling of the chain may not take more than 2.5 times as long, and the installation at its
/// top is charged once: one U3021.
#[test]
fn installation_charge_scales_linearly() {
    let sizes = [250, 500, 1_000, 2_000];
    let times: Vec<Duration> = sizes
        .iter()
        .map(|&n| {
            (0..3)
                .map(|_| {
                    let (codes, took) = checked(n);
                    assert_eq!(codes, vec!["U3021".to_owned()]);
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
            "{} functions took {large:?}, {} took {small:?}",
            sizes_pair[1],
            sizes_pair[0]
        );
    }
}
