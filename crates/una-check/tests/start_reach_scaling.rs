//! Scaling of the reach of a `start` (ADR 0123 IF6, issue #1674): the call graph, its cycles and
//! what each node reaches are computed once per package, in O(V + E), and each `start` that moves
//! reply rights costs O(size of its seed) lookups. A package of `n` request functions, each started
//! by its own `start` and each calling a generic function, would take quadratic time with a walk
//! of the graph per `start` (review M3 of #1678). Timed over a span of 8 times the size, by the
//! shared helper of `una-cli/tests/common/timing.rs` (#1197), which separates linear (about 8 to
//! 12 times) from quadratic (about 64 times) on a shared machine.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::time::Instant;

#[path = "../../una-cli/tests/common/timing.rs"]
mod timing;

use una_check::{ModuleInput, PackageInput, check_names, check_types};
use una_project::parse_manifest;
use una_syntax::{Interner, KeywordSet, SourceDb, lex_in, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.4\"\n";

/// A module of `n` request functions `f<i>`, each calling the generic `same` and replying, and
/// one `tasks` block with a `start` of each.
fn app(n: usize) -> String {
    let mut text = String::from(
        "module app\n\nimport std.host (Exchange, HttpServer, Reply)\n\nsame[T](x: T) -> T = x\n",
    );
    for i in 0..n {
        text.push_str(&format!(
            "\nf{i}(exchange: internal Exchange) -> Unit\n  uses @server: HttpServer[reply]\n=\n  let n = same({i})\n  @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: \"ok\"))\n    if error use unit\n"
        ));
    }
    text.push_str("\npub serve_all(exchanges: internal List[Exchange]) -> Unit\n  uses @server: HttpServer[reply]\n=\n  tasks\n    for exchange in exchanges\n");
    for i in 0..n {
        text.push_str(&format!("      start f{i}(exchange)\n"));
    }
    text
}

/// The codes of the package of size `n`, and the seconds its type check took.
fn checked(n: usize) -> (Vec<String>, f64) {
    let text = app(n);
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let manifest = db.add("una.toml", MANIFEST.as_bytes());
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
    let started = Instant::now();
    let check = check_types(&input);
    let took = started.elapsed().as_secs_f64();
    (
        check
            .diagnostics
            .iter()
            .map(|d| format!("{} {}", d.code.as_str(), d.message))
            .collect(),
        took,
    )
}

/// 8 times the `start`s take less than 24 times as long, and every `start` is accepted: its
/// task reaches no cycle and holds `internal` data at most.
#[test]
fn the_reach_of_many_starts_scales_linearly() {
    let span = [250, 2_000];
    timing::assert_growth(
        "the reach of `start`",
        &span,
        0.0,
        timing::FLOOR,
        24.0,
        |n| {
            let (codes, took) = checked(n);
            assert_eq!(codes, Vec::<String>::new(), "{n} starts");
            took
        },
    );
}
