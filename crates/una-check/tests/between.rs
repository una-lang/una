//! The code between two requests of one task (ADR 0116, amendment of 2026-10-25, P11, issue
//! #1155): the summaries `Pre`, `Suf` and `Gap` over the control-flow graph of Core IR, through
//! the test entry point `between_summaries`.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use una_check::{Between, ModuleInput, PackageInput, between_summaries, check_names, check_types};
use una_core::{Label, Level};
use una_project::parse_manifest;
use una_syntax::{Interner, KeywordSet, SourceDb, lex_in, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.4\"\n";

/// `⊤` for these packages, which declare no category.
fn top() -> Label {
    Label {
        level: Level::Restricted,
        ..Label::public()
    }
}

/// The summaries of each unit of the module `app` whose text is `text`, with the check's
/// diagnostics, which must be empty.
fn summaries(text: &str) -> Vec<(String, Between)> {
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let manifest = db.add("una.toml", MANIFEST.as_bytes());
    let (settings, _) = parse_manifest(MANIFEST);
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
    let check = check_types(&input);
    let errors: Vec<String> = check
        .diagnostics
        .iter()
        .filter(|d| d.severity == una_diag::Severity::Error && d.code.as_str() != "U8101")
        .map(|d| format!("{} {}", d.code.as_str(), d.message))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    between_summaries(&check.program, &top())
}

/// `(reaches, pre, suf, gap, whole)` of the unit `name`, as text.
fn of(units: &[(String, Between)], name: &str) -> (bool, String, String, String, String) {
    let (_, b) = units.iter().find(|(n, _)| n == name).expect("the unit is in Core IR");
    (
        b.reaches,
        b.pre.to_string(),
        b.suf.to_string(),
        b.gap.to_string(),
        b.whole.to_string(),
    )
}

/// Functions the cases share: `fetch` makes one request and handles its failure; `get` also
/// returns a value; `crunch` works on a `secret`, `crunch_open` on a `public`; `work` works on a
/// `secret` and can fail; `tail` makes a request and then works on its `secret`, `head` the other
/// way.
const SHARED: &str = "module app

import std.time (millisecond)

pub kind Problem = | Broke

fetch(url: Text) -> Unit
  uses @http: Http[get]
=
  let outcome = attempt @http.get(url, headers: [], key_name: None)
  unit

get(url: Text) -> Int
  uses @http: Http[get]
=
  let outcome = attempt @http.get(url, headers: [], key_name: None)
  0

crunch(key: secret Int) -> secret Int = key * 3

crunch_open(n: Int) -> Int = n * 3

mix(a: Int, key: secret Int) -> secret Int = a + key

work(key: secret Int, n: Int) -> secret Int
  fails Problem
=
  if n > 0 then fail Broke else key * 3

tail(url: Text, key: secret Int) -> Unit
  uses @http: Http[get]
=
  fetch(url)
  let spent = crunch(key)
  unit

head(url: Text, key: secret Int) -> Unit
  uses @http: Http[get]
=
  let spent = crunch(key)
  fetch(url)
";

/// The summaries of `run`, whose body is `body`, beside [`SHARED`].
fn run(body: &str) -> (bool, String, String, String, String) {
    let text = format!(
        "{SHARED}\npub run(url: Text, key: secret Int, n: Int) -> Unit\n  uses @http: Http[get]\n=\n{body}"
    );
    of(&summaries(&text), "app.run")
}

/// `Gap` of `run`.
fn gap(body: &str) -> String {
    let (reaches, _, _, gap, _) = run(body);
    assert!(reaches);
    gap
}

#[test]
fn work_between_two_requests_counts_and_its_public_twin_does_not() {
    // P10's example: `crunch(key)` between two requests.
    assert_eq!(
        gap("  fetch(url)\n  let spent = crunch(key)\n  fetch(url)\n"),
        "secret"
    );
    assert_eq!(
        gap("  fetch(url)\n  let spent = crunch_open(n)\n  fetch(url)\n"),
        "internal"
    );
}

#[test]
fn work_before_the_first_request_that_decides_nothing_does_not_count() {
    // Refused under P10, only for work before the first request; every bucket starts full.
    assert_eq!(
        gap("  let spent = crunch(key)\n  fetch(url)\n  fetch(url)\n"),
        "internal"
    );
    // After the last request, the same.
    assert_eq!(
        gap("  fetch(url)\n  fetch(url)\n  let spent = crunch(key)\n  unit\n"),
        "internal"
    );
    let (_, pre, suf, _, whole) = run("  let spent = crunch(key)\n  fetch(url)\n  fetch(url)\n");
    assert_eq!(
        (pre.as_str(), suf.as_str(), whole.as_str()),
        ("secret", "internal", "secret")
    );
}

#[test]
fn code_after_a_request_counts_up_to_the_next() {
    // A call that has effects is a whole statement (U3013), so the code after a request is in the
    // statements after it: `mix` reads `key` after the request of `get`.
    assert_eq!(
        gap("  let a = get(url)\n  let b = mix(a, key)\n  fetch(url)\n"),
        "secret"
    );
    assert_eq!(
        gap("  let a = get(url)\n  let b = mix(a, key)\n  unit\n"),
        "internal"
    );
    assert_eq!(
        gap("  let a = get(url)\n  let b = crunch_open(a)\n  fetch(url)\n"),
        "internal"
    );
}

#[test]
fn a_decision_on_a_secret_that_decides_no_request_does_not_count() {
    // A request under a condition above `internal` is refused by the program counter (U4002),
    // so a condition can only decide whether a request happens at `internal` at most, and the
    // request's own label, which joins the program counter, covers it.
    assert_eq!(
        gap("  let spent = if key > 0 then crunch(key) else 0\n  fetch(url)\n  fetch(url)\n"),
        "internal"
    );
    assert_eq!(
        gap("  for i in 0..n\n    let spent = crunch(key)\n    unit\n  fetch(url)\n  fetch(url)\n"),
        "internal"
    );
}

#[test]
fn failure_edges_lead_to_the_handling_clause() {
    // A step that can fail decides whether the requests after it happen, so the label of its
    // failure counts before a request, even the first: `work` fails on `n`, a `public`, and
    // its work on the `secret` does not count there. A failure on a `secret` is refused
    // (U4002).
    assert_eq!(
        gap("  let w = work(key, n)\n    if error use 0\n  fetch(url)\n  fetch(url)\n"),
        "internal"
    );
    // Between two requests, the work of the step counts, whether it fails or not.
    assert_eq!(
        gap("  fetch(url)\n  let w = work(key, n)\n    if error use 0\n  fetch(url)\n"),
        "secret"
    );
    let units = summaries(&format!("{SHARED}\n"));
    let (b, _) = (
        units
            .iter()
            .find(|(n, _)| n == "app.work")
            .unwrap()
            .1
            .clone(),
        (),
    );
    assert_eq!(
        (b.fails.to_string(), b.whole.to_string()),
        ("public".to_owned(), "secret".to_owned())
    );
}

#[test]
fn both_orders_of_pre_and_suf() {
    // `tail` works on the `secret` after its request, `head` before it.
    assert_eq!(gap("  tail(url, key)\n  fetch(url)\n"), "secret");
    assert_eq!(gap("  fetch(url)\n  head(url, key)\n"), "secret");
    let units = summaries(&format!("{SHARED}\n"));
    let (_, pre, suf, _, _) = of(&units, "app.tail");
    assert_eq!((pre.as_str(), suf.as_str()), ("internal", "secret"));
    let (_, pre, suf, _, _) = of(&units, "app.head");
    assert_eq!((pre.as_str(), suf.as_str()), ("secret", "internal"));
}

#[test]
fn a_loop_whose_body_reaches_a_request_counts_whole() {
    // The back edge: one iteration's end comes before the next iteration's request.
    assert_eq!(
        gap("  for i in 0..n\n    fetch(url)\n    let spent = crunch(key)\n    unit\n  unit\n"),
        "secret"
    );
    assert_eq!(
        gap("  for i in 0..n\n    let spent = crunch(key)\n    fetch(url)\n  unit\n"),
        "secret"
    );
    assert_eq!(
        gap("  for i in 0..n\n    fetch(url)\n    let spent = crunch_open(n)\n    unit\n  unit\n"),
        "internal"
    );
}

#[test]
fn a_lambda_applied_between_requests_counts() {
    assert_eq!(
        gap("  fetch(url)\n  let xs = [1, 2].map(each: fn(x) -> x + key)\n  fetch(url)\n"),
        "secret"
    );
    assert_eq!(
        gap("  fetch(url)\n  let xs = [1, 2].map(each: fn(x) -> x + n)\n  fetch(url)\n"),
        "internal"
    );
}

#[test]
fn a_deadline_decides_from_every_step_inside_it() {
    // The test entry counts each deadline's decision as `⊤`: a `within` from which a request can
    // be reached counts it; one after the last request does not.
    assert_eq!(
        gap(
            "  let v = crunch_open(n) within 5 * millisecond\n    if error use 0\n  fetch(url)\n  fetch(url)\n"
        ),
        "restricted"
    );
    assert_eq!(
        gap(
            "  fetch(url)\n  fetch(url)\n  let v = crunch_open(n) within 5 * millisecond\n    if error use 0\n  unit\n"
        ),
        "internal"
    );
}

#[test]
fn a_start_is_a_call_of_its_task() {
    // The started task makes a request, so the work after the `start`, up to the end of the
    // `tasks` statement, which waits for the task, counts.
    assert_eq!(
        gap("  tasks\n    start fetch(url)\n    let spent = crunch(key)\n    unit\n  unit\n"),
        "secret"
    );
    assert_eq!(
        gap("  tasks\n    start fetch(url)\n    let spent = crunch_open(n)\n    unit\n  unit\n"),
        "internal"
    );
    // Before the statement, nothing decides it.
    assert_eq!(
        gap("  let spent = crunch(key)\n  tasks\n    start fetch(url)\n    unit\n  unit\n"),
        "internal"
    );
}
