//! The report's `preview_constructs` (ADR 0121 rule 8, `spec/13 §6`): for each entry of the
//! edition ledger (rule 9), a package that uses its construct gets its entry, and a construct
//! added to the ledger without a package here fails, so the report never leaves out silently a
//! construct a package uses.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use una_project::ledger::LEDGER;

fn una_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

const LOCK: &str = "lock 1\n\nsolver\n  encoding \"una-smt-1\"\n  name \"z3\"\n  version \"5.1.0\"\n  logic \"QF_UFLIA\"\n  steps 5000000\n  seed 0\n";

/// A package that uses one construct: its edition, the sections of `ita.una` after the head,
/// and its module `src/m.una`.
struct Case {
    construct: &'static str,
    edition: &'static str,
    sections: &'static str,
    module: &'static str,
}

const UNTIL: &str = "module m

pub upto(n: Int) -> Int
=
  let edit i = 0
  for until i >= n decreases n - i
    i = i + 1
  i
";

const EXITS: &str = "module m

pub exits(xs: List[Int]) -> Int
=
  let edit n = 0
  for x in xs
    stop when x > 3
    if x == 2
      skip
    n = n + x
  n
";

const EVER: &str = "module m

pub forever() -> Unit
  diverges
=
  for ever
    stop
  unit
";

const EVER_WITHIN: &str = "module m

import std.time (millisecond)

pub bounded() -> Unit
  diverges
=
  for ever within 100 * millisecond
    stop
  unit
";

const DRIVE: &str = "module m

import std.drive (Tick)

pub init() -> Int = 0

pub step(edit state: Int, input: Tick) -> Unit
=
  state = state + 1
";

const STOP_AFTER: &str = "module m

pub count() -> Int
  diverges
=
  let edit n = 0
  for ever
    n = n + 1
  n

test \"three steps\"
  do count()
  expect result == 3
  stop after 3
";

const TIME: &str = "module m

import std.time (Millisecond, millisecond)

pub delay() -> Int of Millisecond = 5 * millisecond
";

const WITHIN: &str = "module m

import std.time (millisecond)

pub kind RouteError = NoPath | TimedOut

shortest(n: Int) -> Int
  fails RouteError
=
  if n < 0 then fail NoPath else n

pub plan(n: Int) -> Int
  fails RouteError
=
  let route = shortest(n) within 50 * millisecond
    if error pass
  route
";

const FIRST_OF: &str = "module m

count(n: Int) -> Int = n + 1

pub raced(n: Int) -> Int
=
  let value = first of
    count(n)
    count(n + 1)
  value
";

const TASKS: &str = "module m

bump(n: Int) -> Unit = unit

pub run(n: Int) -> Unit
=
  tasks
    start bump(n)
  unit
";

const PLAIN: &str = "module m

pub one() -> Int = 1
";

const LIMITER: &str = "module m

import std.limit (Limit, limiter)
import std.time (minute)

acquired() -> Int
  uses @gate: Limit[acquire]
=
  let got = attempt @gate.acquire()
  1

pub paced() -> Int
=
  let n = acquired() with @gate = limiter(at_most: 1, per: minute, waiting: 0)
  n
";

const SERVER: &str = "module m

import std.host (Request)

pub path(request: internal Request) -> internal List[Text] = request.segments
";

const SERVE: &str = "module srv

import std.host (Reply, Request)

pub handle(request: internal Request) -> internal Reply
=
  Reply(status: 200, headers: [], body: \"ok\")
";

const COLLECTIONS: &str = "module m

import std.collections (Stack, empty_stack)

pub pages() -> Stack[Text]
=
  var pages: Stack[Text] = empty_stack()
  edit pages.push(item: \"home\")
  pages
";

const MAP_UPDATE: &str = "module m

pub counts() -> Map[Text, Int]
=
  var counts: Map[Text, Int] = empty_map()
  edit counts.update(key: \"a\", initial: 0, change: fn(n) -> n + 1)
  counts
";

const DOT: &str = "module m

pub inner(a: Array[F64], b: Array[F64]) -> F64 = a.dot(with: b)
";

/// One package per construct of the ledger.
const CASES: &[Case] = &[
    Case {
        construct: "for until",
        edition: "0.4",
        sections: "",
        module: UNTIL,
    },
    Case {
        construct: "stop and skip",
        edition: "0.4",
        sections: "",
        module: EXITS,
    },
    Case {
        construct: "diverges",
        edition: "0.4",
        sections: "",
        module: EVER,
    },
    Case {
        construct: "for ever",
        edition: "0.4",
        sections: "",
        module: EVER,
    },
    Case {
        construct: "for ever within",
        edition: "0.4",
        sections: "",
        module: EVER_WITHIN,
    },
    Case {
        construct: "drive",
        edition: "0.4",
        sections: "\ndrive\n  module m\n  ticks 16\n",
        module: DRIVE,
    },
    Case {
        construct: "stop after",
        edition: "0.4",
        sections: "",
        module: STOP_AFTER,
    },
    Case {
        construct: "std.time",
        edition: "0.4",
        sections: "",
        module: TIME,
    },
    Case {
        construct: "within",
        edition: "0.4",
        sections: "",
        module: WITHIN,
    },
    Case {
        construct: "first of",
        edition: "0.4",
        sections: "",
        module: FIRST_OF,
    },
    Case {
        construct: "tasks and start",
        edition: "0.4",
        sections: "",
        module: TASKS,
    },
    Case {
        construct: "grant limits",
        edition: "0.4",
        sections: "\nrun\n  grants\n    http \"https://api.example.com\" get\n      at most 1 per minute\n",
        module: PLAIN,
    },
    Case {
        construct: "limiter",
        edition: "0.4",
        sections: "",
        module: LIMITER,
    },
    Case {
        construct: "ita.una",
        edition: "0.4",
        sections: "",
        module: PLAIN,
    },
    Case {
        construct: "ita.lock",
        edition: "0.4",
        sections: "",
        module: PLAIN,
    },
    Case {
        construct: "needs",
        edition: "0.4",
        sections: "\nneeds\n  util from path \"deps/util\"\n    accept name\n",
        module: PLAIN,
    },
    Case {
        construct: "HttpServer",
        edition: "0.4",
        sections: "",
        module: SERVER,
    },
    Case {
        construct: "serve",
        edition: "0.4",
        sections: "\nrun\n  grants\n    serve \"http://127.0.0.1:8080\"\n\nserve\n  module srv\n",
        module: SERVE,
    },
    Case {
        construct: "std.collections",
        edition: "0.5",
        sections: "",
        module: COLLECTIONS,
    },
    Case {
        construct: "Map.update",
        edition: "0.5",
        sections: "",
        module: MAP_UPDATE,
    },
    Case {
        construct: "Decimal",
        edition: "0.5",
        sections: "",
        module: "module m\n\npub price(x: Decimal[2]) -> Decimal[2] = x + 1.5\n",
    },
    Case {
        construct: "Array, Matrix and Tensor",
        edition: "0.5",
        sections: "",
        module: "module m\n\npub same(a: Array[F64]) -> Array[F64] = a\n",
    },
    Case {
        construct: "sum",
        edition: "0.5",
        sections: "",
        module: "module m\n\npub total(a: Array[F64]) -> F64 = a.sum()\n",
    },
    Case {
        construct: "dot",
        edition: "0.5",
        sections: "",
        module: DOT,
    },
    Case {
        construct: "matrix kernels",
        edition: "0.5",
        sections: "",
        module: "module m\n\npub product(a: Matrix[F64], b: Matrix[F64]) -> Matrix[F64] = a.multiply(by: b)\n",
    },
    Case {
        construct: "simd as a name",
        edition: "0.5",
        sections: "",
        module: "module m\n\npub simd(x: Int) -> Int = x\n",
    },
    Case {
        construct: "with: and and: labels",
        edition: "0.5",
        sections: "",
        module: DOT,
    },
    Case {
        construct: "F32",
        edition: "0.5",
        sections: "",
        module: "module m\n\npub same(x: F32) -> F32 = x\n",
    },
    Case {
        construct: "Complex",
        edition: "0.5",
        sections: "",
        module: "module m\n\npub make(re: F64, im: F64) -> Complex[F64] = complex(re, im)\n",
    },
    Case {
        construct: "module constants",
        edition: "0.5",
        sections: "",
        module: "module m\n\npub const limit: Int = 3\n\npub twice() -> Int = 2 * limit\n",
    },
    Case {
        construct: "const and var",
        edition: "0.5",
        sections: "",
        module: "module m\n\npub double(x: Int) -> Int\n=\n  var y = x\n  y = y + x\n  y\n",
    },
];

fn package(root: &Path, index: usize, case: &Case) -> PathBuf {
    let dir = root.join(format!("{index:02}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let file = if case.module.starts_with("module srv") {
        "srv.una"
    } else {
        "m.una"
    };
    std::fs::write(dir.join("src").join(file), case.module).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita demo\n  version 0.1.0\n  edition {}\n{}",
            case.edition, case.sections
        ),
    )
    .unwrap();
    std::fs::write(dir.join("ita.lock"), LOCK).unwrap();
    // The modules are written for reading; `una fmt` gives them their canonical form.
    una_in(&dir, &["fmt", "."]);
    if case.sections.contains("needs") {
        let util = dir.join("deps/util");
        std::fs::create_dir_all(util.join("src")).unwrap();
        std::fs::write(
            util.join("ita.una"),
            "ita util\n  version 0.1.0\n  edition 0.4\n",
        )
        .unwrap();
        std::fs::write(util.join("ita.lock"), LOCK).unwrap();
        std::fs::write(
            util.join("src/text.una"),
            "module text\n\npub two() -> Int = 2\n",
        )
        .unwrap();
        let output = una_in(&dir, &["ita", "lock"]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    dir
}

/// The report of `dir` as JSON, and its human `edition:` line.
fn report(dir: &Path) -> (Value, String) {
    let output = una_in(dir, &["--json", "report", "."]);
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let json: Value = serde_json::from_str(&text).unwrap_or_else(|_| {
        panic!(
            "{}: no report: {text}{}",
            dir.display(),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(json["schema"], "una.report/2", "{text}");
    let human = una_in(dir, &["report", "."]);
    let line = String::from_utf8_lossy(&human.stdout)
        .lines()
        .find(|line| line.starts_with("edition: "))
        .unwrap_or_default()
        .to_owned();
    (json, line)
}

#[test]
fn every_ledger_entry_has_a_package_that_uses_it() {
    // A construct added to the ledger without a case here fails, as one whose case the
    // report does not list.
    let ledger: BTreeSet<&str> = LEDGER.iter().map(|entry| entry.name).collect();
    let cases: BTreeSet<&str> = CASES.iter().map(|case| case.construct).collect();
    assert_eq!(ledger, cases, "the cases and the ledger disagree");
    assert_eq!(cases.len(), CASES.len(), "a construct has two cases");
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("preview-constructs");
    for (index, case) in CASES.iter().enumerate() {
        let dir = package(&root, index, case);
        let (json, line) = report(&dir);
        let entry = LEDGER
            .iter()
            .find(|entry| entry.name == case.construct)
            .unwrap();
        let listed = json["edition"]["preview_constructs"]
            .as_array()
            .unwrap_or_else(|| panic!("{}: {json}", case.construct));
        let found = listed
            .iter()
            .find(|item| item["construct"] == case.construct)
            .unwrap_or_else(|| {
                panic!(
                    "{}: the report does not list it: {}",
                    case.construct, json["edition"]
                )
            });
        assert_eq!(found["record"], entry.record, "{}", case.construct);
        assert!(found["sites"].as_u64().unwrap() >= 1, "{}", case.construct);
        let unsettled: Vec<&str> = entry.unsettled.iter().map(|u| u.name).collect();
        assert_eq!(found["unsettled"], serde_json::json!(unsettled));
        assert!(
            line.starts_with(&format!("edition: {} (preview): ", case.edition))
                && line.contains(&format!("{} ({}, ", case.construct, entry.record)),
            "{}: {line}",
            case.construct
        );
        // Ordered by record, then construct.
        let keys: Vec<(String, String)> = listed
            .iter()
            .map(|item| {
                (
                    item["record"].as_str().unwrap().to_owned(),
                    item["construct"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "{}", case.construct);
    }
}

#[test]
fn the_human_line_names_sites_and_unsettled_points() {
    // ADR 0121 rule 8: `sum (ADR 0120, 1 site; unsettled: ...)`, with `sites` for more.
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("preview-constructs-line");
    let dir = package(
        &root,
        0,
        &Case {
            construct: "sum",
            edition: "0.5",
            sections: "",
            module: "module m\n\npub total(a: Array[F64]) -> F64 = a.sum()\n\npub twice(a: Array[F64]) -> F64 = a.sum() + a.sum()\n",
        },
    );
    let (json, line) = report(&dir);
    let sum = json["edition"]["preview_constructs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["construct"] == "sum")
        .unwrap()
        .clone();
    assert_eq!(sum["sites"], 3, "{json}");
    assert!(
        line.contains(
            "sum (ADR 0120, 3 sites; unsettled: the accumulation order of 0120's kernels (L, KB))"
        ),
        "{line}"
    );
    assert!(line.contains("ita.una (ADR 0122, 1 site)"), "{line}");
}

#[test]
fn a_stable_edition_and_edition_0_3_list_none() {
    // ADR 0121 rules 8 and 13: `[]` for a package of an edition with no ledger entry, and the
    // human line keeps its form.
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("preview-constructs-none");
    for edition in ["0.2", "0.3"] {
        let dir = root.join(edition);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/m.una"), PLAIN).unwrap();
        std::fs::write(
            dir.join("una.toml"),
            format!("[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"{edition}\"\n"),
        )
        .unwrap();
        let lock = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../verification.lock");
        std::fs::copy(lock, dir.join("verification.lock")).unwrap();
        let (json, line) = report(&dir);
        assert_eq!(json["edition"]["preview_constructs"], serde_json::json!([]));
        let preview = if edition == "0.3" { " (preview)" } else { "" };
        assert_eq!(line, format!("edition: {edition}{preview}"));
    }
}

#[test]
fn a_dependency_s_constructs_are_not_the_root_s() {
    // Only the root's modules count: a dependency's `for ever` is not listed for the root.
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("preview-constructs-dependency");
    let dir = package(
        &root,
        0,
        &Case {
            construct: "needs",
            edition: "0.4",
            sections: "\nneeds\n  util from path \"deps/util\"\n    accept name\n",
            module: PLAIN,
        },
    );
    std::fs::write(
        dir.join("deps/util/src/text.una"),
        EVER.replace("module m", "module text"),
    )
    .unwrap();
    let output = una_in(&dir, &["ita", "lock"]);
    assert_eq!(output.status.code(), Some(0));
    let (json, _) = report(&dir);
    let names: Vec<&str> = json["edition"]["preview_constructs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["construct"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"needs"), "{names:?}");
    assert!(
        !names.contains(&"for ever") && !names.contains(&"diverges"),
        "{names:?}"
    );
}

#[test]
fn a_package_that_writes_no_construct_lists_only_its_project_files() {
    // No site is invented: a module of edition 0.4 or 0.5 that writes nothing of the ledger
    // lists only the files every such package has.
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("preview-constructs-plain");
    for (index, edition) in ["0.4", "0.5"].into_iter().enumerate() {
        let dir = package(
            &root,
            index,
            &Case {
                construct: "ita.una",
                edition,
                sections: "",
                module: "module m\n\npub kind Point\n  x: Int\n  y: F64\n\npub add(p: Point, n: List[Int]) -> Int = p.x + n.len\n",
            },
        );
        let (json, line) = report(&dir);
        assert_eq!(
            json["edition"]["preview_constructs"],
            serde_json::json!([
                { "construct": "ita.lock", "record": "ADR 0122", "sites": 1, "unsettled": [] },
                { "construct": "ita.una", "record": "ADR 0122", "sites": 1, "unsettled": [] }
            ]),
            "{edition}"
        );
        assert_eq!(
            line,
            format!(
                "edition: {edition} (preview): ita.lock (ADR 0122, 1 site), ita.una (ADR 0122, 1 site)"
            )
        );
    }
}
