//! End-to-end tests of tier 1 (`spec/12`): `una check` with the solver, `una obligations`, the
//! proof cache, the queries' determinism and U5003. They need Z3 at the version of
//! `verification.lock` on the path (`compiler/testing.md §3`).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use una_cli::Tier;

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn una(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(args)
        .current_dir(repository())
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).unwrap()
}

fn stderr(output: &Output) -> &str {
    std::str::from_utf8(&output.stderr).unwrap()
}

/// A package under the test's temporary directory with the repository's lock.
fn package(name: &str, files: &[(&str, &str)]) -> PathBuf {
    package_of_edition(name, "0.1", files)
}

/// [`package`] of the given edition (`spec/13 §7`).
fn package_of_edition(name: &str, edition: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("verification")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"{edition}\"\n"),
    )
    .unwrap();
    std::fs::copy(
        repository().join("verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    for (path, text) in files {
        std::fs::write(root.join(path), text).unwrap();
    }
    root
}

/// How tier 1 normalizes the source text of the package at `path`: edition 0.4, which only
/// `ita.una` describes, erases the layout of wrapped lines (ADR 0125, amendment of 2026-10-25).
/// The edition is the manifest's, not the presence of the file (#1175).
fn layout_of(path: &Path) -> una_verify::SourceLayout {
    let ita = path.join("ita.una");
    let edition = match std::fs::read_to_string(&ita) {
        Ok(text) => una_project::ita::parse_ita_manifest(&text).0.edition,
        Err(_) => una_project::Edition::V0_1,
    };
    if edition >= una_project::Edition::V0_4 {
        una_verify::SourceLayout::Wrapped
    } else {
        una_verify::SourceLayout::Joined
    }
}

/// The package at `path` checked at tier 0, and the obligations that tier 1 would generate for
/// it, on a stack as large as tier 1 gives them, but not solved: tests that compare obligations
/// or queries start no solver (#286).
fn generated(path: &Path) -> (una_cli::Checked, Vec<una_verify::Obligation>) {
    let checked = una_cli::check(path, Tier::Zero).unwrap();
    let layout = layout_of(path);
    let obligations = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn_scoped(scope, || {
                una_verify::obligations(&checked.program, &checked.sources, layout)
            })
            .unwrap()
            .join()
            .unwrap()
    });
    (checked, obligations.unwrap())
}

#[test]
fn the_examples_verify_with_every_obligation_proven() {
    // M8 acceptance criterion: all examples verify, with zero refuted and zero unknown.
    for example in ["examples/events", "examples/hello"] {
        let output = una(&["check", example]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{example}: {}",
            stderr(&output)
        );
        assert_eq!(stderr(&output), "", "{example}");
    }
    let output = una(&["--json", "obligations", "examples/events"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let document: Value = serde_json::from_str(stdout(&output)).unwrap();
    assert_eq!(document["schema"], "una.obligations/1");
    let obligations = document["obligations"].as_array().unwrap();
    assert_eq!(obligations.len(), 11);
    assert!(obligations.iter().all(|o| o["status"] == "proven"));
    let postcondition = obligations
        .iter()
        .find(|o| o["kind"] == "postcondition")
        .unwrap();
    assert_eq!(postcondition["declaration"], "events.seating.availability");
    assert_eq!(
        postcondition["goal"],
        "e.remaining < 1 implies result is SoldOut"
    );
    assert_eq!(
        postcondition["source"],
        json!({
            "file": "src/events/seating.una",
            "start": {"line": 23, "col": 11},
            "end": {"line": 23, "col": 52}
        })
    );
    assert_eq!(
        postcondition["hypotheses"],
        json!(["0 <= e.remaining", "e.remaining <= 5000"])
    );
    assert_eq!(postcondition["domain"], "semantic");
    assert!(
        postcondition["id"]
            .as_str()
            .unwrap()
            .strip_prefix("ob-")
            .is_some_and(|hex| hex.len() == 12)
    );
}

#[test]
fn a_second_run_reports_every_obligation_as_cached() {
    // M8 acceptance criterion: the second run of the examples is answered by the cache.
    let first = una(&["--json", "obligations", "examples/events"]);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    let second = una(&["--json", "obligations", "examples/events"]);
    let document: Value = serde_json::from_str(stdout(&second)).unwrap();
    let obligations = document["obligations"].as_array().unwrap();
    assert!(!obligations.is_empty());
    assert!(
        obligations.iter().all(|o| o["cached"] == true),
        "{}",
        stdout(&second)
    );
    let human = una(&["obligations", "examples/events"]);
    assert_eq!(human.status.code(), Some(0));
    let lines: Vec<&str> = stdout(&human).lines().collect();
    assert_eq!(lines.len(), 11);
    assert!(lines.iter().all(|line| line.starts_with("proven ")));
    assert!(lines.iter().all(|line| line.ends_with(" (cached)")));
    assert!(
        lines.iter().any(|line| line.contains(
            "postcondition src/events/seating.una:23:11 e.remaining < 1 implies result is SoldOut"
        )),
        "{}",
        stdout(&human)
    );
}

#[test]
fn the_queries_of_the_examples_are_byte_identical_across_runs() {
    // M8 acceptance criterion and `spec/12 §4`: declarations sorted by name, assertions in
    // generation order. Every package under `examples/` has its own snapshot (R-036).
    let queries = |example: &Path| {
        let (checked, obligations) = generated(example);
        assert_eq!(checked.diagnostics, [], "{}", example.display());
        let text = std::fs::read_to_string(example.join("verification.lock")).unwrap();
        let (lock, _) = una_project::parse_lock(&text);
        let lock = una_verify::Lock {
            text,
            solver_version: lock.solver_version,
            step_budget: lock.step_budget,
            random_seed: lock.random_seed,
        };
        let queries = una_verify::queries(&obligations, &lock);
        let mut out = String::new();
        for (obligation, query) in obligations.iter().zip(queries) {
            out.push_str(&format!(
                "; {} {} {} {}\n{query}\n",
                obligation.id,
                obligation.kind.as_str(),
                obligation.declaration,
                obligation.goal
            ));
        }
        out
    };
    let mut examples: Vec<PathBuf> = std::fs::read_dir(repository().join("examples"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.join("una.toml").is_file())
        .collect();
    examples.sort();
    assert!(examples.len() >= 7, "{examples:?}");
    for example in &examples {
        let name = example.file_name().unwrap().to_string_lossy().into_owned();
        let first = queries(example);
        assert_eq!(first, queries(example), "{name}");
        insta::assert_snapshot!(format!("queries_{name}"), first);
    }
}

#[test]
fn a_refuted_obligation_shows_its_counterexample_and_repairs() {
    // `spec/13 §1`: the obligation, the counterexample and ranked repairs.
    let root = package(
        "availability",
        &[(
            "src/seating.una",
            "module seating

pub kind Seats = Int in 0..5000

pub kind Availability = SoldOut | Few | Open

pub availability(remaining: Seats) -> Availability
  ensures remaining < 1 implies result is SoldOut
=
  when remaining
    in 1..<20 -> Few
    in 0..0 -> SoldOut
    in 20..5000 -> Open
",
        )],
    );
    let output = una(&["check", root.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let broken = package(
        "availability-broken",
        &[(
            "src/seating.una",
            "module seating

pub kind Seats = Int in 0..5000

pub kind Availability = SoldOut | Few | Open

pub availability(remaining: Seats) -> Availability
  ensures remaining < 1 implies result is SoldOut
=
  when remaining
    in 0..<20 -> Few
    in 20..5000 -> Open
",
        )],
    );
    let output = una(&["--json", "check", broken.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    let document: Value = serde_json::from_str(stdout(&output)).unwrap();
    let diagnostic = &document["diagnostics"][0];
    assert_eq!(diagnostic["code"], "U5001");
    assert_eq!(
        diagnostic["message"],
        "postcondition of `availability` is refuted"
    );
    assert_eq!(
        diagnostic["primary"],
        json!({"file": "src/seating.una", "start": {"line": 11, "col": 18}, "end": {"line": 11, "col": 21}})
    );
    assert_eq!(diagnostic["related"][0]["message"], "ensures clause");
    assert_eq!(diagnostic["obligation"]["kind"], "postcondition");
    assert_eq!(diagnostic["obligation"]["status"], "refuted");
    assert_eq!(
        diagnostic["counterexample"],
        json!({"bindings": [
            {"name": "remaining", "value": 0},
            {"name": "result", "value": "Few"}
        ]})
    );
    let repairs = diagnostic["repairs"].as_array().unwrap();
    assert_eq!(repairs[0]["rank"], 1);
    assert_eq!(repairs[0]["escape_cost"], 0);
}

#[test]
fn a_missing_solver_is_u5003_and_never_a_silent_pass() {
    // `spec/12 §5`, `compiler/testing.md §3`: without the pinned solver, tier 1 fails.
    let root = package(
        "no-solver",
        &[(
            "src/app.una",
            "module app

pub kind Seats = Int in 0..5000

pub book(seats: Seats) -> Int = seats

pub first() -> Int = book(4321)
",
        )],
    );
    // ADR 0124 rule 11: nothing on PATH is ever run. A build with the solver linked in proves
    // with an empty PATH; a development build without it is U5003 at the lock's pin, naming
    // the script that builds the library (rule 19).
    let empty = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("no-solver-path");
    std::fs::create_dir_all(&empty).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .env_remove("UNA_SOLVER_STAND_IN")
        .args(["--json", "check", root.to_str().unwrap()])
        .env("PATH", &empty)
        .output()
        .unwrap();
    let document: Value = serde_json::from_str(stdout(&output)).unwrap();
    if una_z3::BUILD.linked {
        assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    } else {
        assert_eq!(output.status.code(), Some(1));
        let diagnostic = &document["diagnostics"][0];
        assert_eq!(diagnostic["code"], "U5003");
        assert_eq!(diagnostic["primary"]["file"], "verification.lock");
        assert_eq!(
            diagnostic["primary"]["start"],
            json!({"line": 3, "col": 20})
        );
        assert!(
            diagnostic["message"]
                .as_str()
                .unwrap()
                .contains("no solver built in; run scripts/build-z3.sh"),
            "{}",
            stdout(&output)
        );
    }
    // Tier 0 needs no solver.
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(["check", "--tier", "0", root.to_str().unwrap()])
        .env("PATH", &empty)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(Path::new(&root).exists());
}

#[test]
fn the_solver_memory_flag_takes_an_integer_from_256_to_1048576() {
    // ADR 0124 rule 6, `spec/13 §5`: another value is a usage error, exit code 2, before any
    // package is read.
    for value in ["255", "x", "1048577", "-1"] {
        let output = Command::new(env!("CARGO_BIN_EXE_una"))
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .env_remove("UNA_PROOF_KEY")
            // `=` keeps `-1` a value rather than a flag of its own.
            .args(["check", &format!("--solver-memory={value}"), "."])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{value}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains("--solver-memory"),
            "{value}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn host_capabilities_are_declared_in_std_host_with_their_signatures() {
    // ADR 0038 D6, `spec/14 §3`: the verifier sees the host capabilities as declarations of
    // `std.host`, and operations name their capability by that identity.
    let checked = una_cli::check(&repository().join("examples/hello"), Tier::Zero).unwrap();
    let host: Vec<(String, Vec<String>)> = checked
        .program
        .host
        .iter()
        .filter_map(|decl| match decl {
            una_core::Decl::Cap { name, ops } => Some((
                name.clone(),
                ops.iter()
                    .map(|op| format!("{}/{}", op.name, op.params.len()))
                    .collect(),
            )),
            _ => None,
        })
        .collect();
    let expected: Vec<(String, Vec<String>)> = [
        ("Console", vec!["print/1"]),
        ("Clock", vec!["now/0"]),
        ("Env", vec!["get/1"]),
        ("Files", vec!["read/1", "write/2"]),
        ("Keys", vec!["hmac/2"]),
    ]
    .into_iter()
    .map(|(name, ops)| {
        (
            name.to_owned(),
            ops.into_iter().map(str::to_owned).collect(),
        )
    })
    .collect();
    assert_eq!(host, expected);
    let mut paths = Vec::new();
    for module in &checked.program.modules {
        for decl in &module.decls {
            if let una_core::Decl::Fun(fun) = decl {
                collect_op_paths(&fun.body, &mut paths);
            }
        }
    }
    assert!(!paths.is_empty());
    assert!(paths.iter().all(|p| p == "std.host.Console"), "{paths:?}");
}

/// The qualified capability of every operation in `expr`.
fn collect_op_paths(expr: &una_core::Expr, out: &mut Vec<String>) {
    use una_core::{Bound, Expr, Rhs};
    match expr {
        Expr::Let { value, body, .. } => {
            match value.as_ref() {
                Bound::Rhs(Rhs::Op {
                    capability_path, ..
                }) => out.push(capability_path.qualified()),
                Bound::Expr(inner) => collect_op_paths(inner, out),
                Bound::Rhs(_) => {}
            }
            collect_op_paths(body, out);
        }
        Expr::If {
            then, otherwise, ..
        } => {
            collect_op_paths(then, out);
            collect_op_paths(otherwise, out);
        }
        Expr::Case {
            arms, otherwise, ..
        } => {
            for arm in arms {
                collect_op_paths(&arm.body, out);
            }
            if let Some(otherwise) = otherwise {
                collect_op_paths(otherwise, out);
            }
        }
        Expr::Handle { body, handler, .. } => {
            collect_op_paths(body, out);
            collect_op_paths(handler, out);
        }
        Expr::Attempt { body, .. }
        | Expr::With { body, .. }
        | Expr::Within { body, .. }
        | Expr::LetTuple { body, .. }
        | Expr::Tasks { body, .. }
        | Expr::Loop { body, .. } => collect_op_paths(body, out),
        Expr::Start { task, .. } | Expr::Quiet { body: task, .. } => collect_op_paths(task, out),
        Expr::Together { tasks, .. } | Expr::FirstOf { tasks, .. } => {
            for task in tasks {
                collect_op_paths(task, out);
            }
        }
        Expr::TogetherFor { task, .. } => collect_op_paths(task, out),
        Expr::Atom { .. } | Expr::Fail { .. } | Expr::Exit { .. } | Expr::Unreachable { .. } => {}
    }
}

#[cfg(unix)]
#[test]
fn a_query_the_solver_rejects_is_u8001_and_exits_with_3() {
    // `spec/13 §5`: an internal compiler error exits with 3 (R-019). A stand-in solver of the
    // pinned version rejects every query, as Z3 does for a query that is not valid SMT-LIB.
    use std::os::unix::fs::PermissionsExt;
    let root = package(
        "rejected-query",
        &[(
            "src/app.una",
            "module app\n\npub half(x: Int, y: Int) -> Int = x / y\n",
        )],
    );
    let bin = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("rejecting-solver");
    std::fs::create_dir_all(&bin).unwrap();
    let solver = bin.join("z3");
    // Written under another name and renamed into place, so that no process can execute the
    // file while it is still open for writing ("text file busy", #286).
    let staged = bin.join("z3.staged");
    // A stand-in for the solver child (ADR 0124; the test hooks): it writes the handshake it
    // is given, reads the query and rejects it.
    std::fs::write(
        &staged,
        "#!/bin/sh\necho \"$1\"\ncat > /dev/null\necho '(error \"line 1 column 1: rejected\")'\n",
    )
    .unwrap();
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&staged, &solver).unwrap();
    let _ = std::fs::remove_dir_all(root.join(".una"));
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .env("UNA_SOLVER_STAND_IN", &solver)
        .args(["--json", "check", root.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", stdout(&output));
    let document: Value = serde_json::from_str(stdout(&output)).unwrap();
    assert_eq!(document["diagnostics"][0]["code"], "U8001");
}

#[test]
fn the_verifier_generates_every_obligation_the_checker_found() {
    // `spec/12 §3` over Core IR agrees with the sites the type checker records since M4: the
    // site of each obligation, or each return path of a return obligation.
    let root = repository().join("tests/conformance");
    let mut targets: Vec<PathBuf> = vec![
        repository().join("examples/events"),
        repository().join("examples/hello"),
    ];
    for area in ["types", "authority", "security", "verification", "runtime"] {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(root.join(area))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| !p.file_name().unwrap().to_string_lossy().starts_with('.'))
            .collect();
        entries.sort();
        targets.extend(entries);
    }
    let mut compared = 0;
    // Every package's differences are gathered before the test fails, so one run shows the
    // whole scope of a drift, never only its first package.
    let mut drift: Vec<String> = Vec::new();
    for target in targets {
        // Tier 1 generates obligations only for a package without diagnostics.
        let (checked, obligations) = generated(&target);
        if !checked.diagnostics.is_empty() {
            continue;
        }
        let key = |kind: una_diag::ObligationKind, file: &str, line: u32, col: u32| {
            format!("{} {file}:{line}:{col}", kind.as_str())
        };
        let mut pending: Vec<String> = checked
            .pending
            .iter()
            .map(|p| {
                key(
                    p.kind,
                    &p.location.file,
                    p.location.start.line,
                    p.location.start.col,
                )
            })
            .collect();
        let mut generated: Vec<String> = Vec::new();
        // Every kind is compared, `effect_condition` included (#615): a site that tier 1
        // generates and the checker does not record fails here, and the reverse too.
        for obligation in &obligations {
            let spans = if obligation.paths.is_empty() {
                vec![obligation.source]
            } else {
                obligation.paths.clone()
            };
            for span in spans {
                let location = checked.sources.location(span);
                generated.push(key(
                    obligation.kind,
                    &location.file,
                    location.start.line,
                    location.start.col,
                ));
            }
        }
        pending.sort();
        // One pending entry per `effect_condition` site, however many routes charge it (#615).
        let sites: Vec<&String> = pending
            .iter()
            .filter(|k| k.starts_with("effect_condition "))
            .collect();
        assert!(
            sites.windows(2).all(|pair| pair[0] != pair[1]),
            "{}: {sites:?}",
            target.display()
        );
        pending.dedup();
        generated.sort();
        generated.dedup();
        let name = target
            .strip_prefix(repository())
            .unwrap_or(&target)
            .display()
            .to_string();
        for site in &generated {
            if pending.binary_search(site).is_err() {
                drift.push(format!(
                    "{name}: tier 1 generates `{site}`, the checker records none"
                ));
            }
        }
        for site in &pending {
            if generated.binary_search(site).is_err() {
                drift.push(format!(
                    "{name}: the checker records `{site}`, tier 1 generates none"
                ));
            }
        }
        // ADR 0101: solved, the same obligations get every value they request; an error of
        // the solver after `sat`, in any outcome, means a wrong script (#668).
        let solved = una_cli::check(&target, Tier::One).unwrap();
        let solver_errors: Vec<String> = solved
            .verification
            .iter()
            .flat_map(|verification| verification.obligations.iter().zip(&verification.outcomes))
            .filter_map(|(obligation, outcome)| {
                let error = outcome.values_error.as_ref()?;
                Some(format!("{}: {error}", obligation.id))
            })
            .collect();
        assert_eq!(solver_errors, Vec::<String>::new(), "{}", target.display());
        // #638: the checker rejects a locked condition outside the encoding as U7002, so no
        // obligation the checker admits may carry one.
        let unencodable: Vec<String> = obligations
            .iter()
            .filter_map(|obligation| {
                let condition = obligation.unencodable.as_ref()?;
                Some(format!("{}: `{condition}`", obligation.id))
            })
            .collect();
        assert_eq!(unencodable, Vec::<String>::new(), "{}", target.display());
        compared += 1;
    }
    assert!(
        drift.is_empty(),
        "{} sites differ between the checker and tier 1:\n{}",
        drift.len(),
        drift.join("\n")
    );
    assert!(compared > 30, "{compared}");
}

/// The single diagnostic of `una --json check` on a package of one module, which must fail.
fn only_diagnostic(name: &str, source: &str) -> Value {
    let root = package(name, &[("src/app.una", source)]);
    let output = una(&["--json", "check", root.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let document: Value = serde_json::from_str(stdout(&output)).unwrap();
    let diagnostics = document["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "{}", stdout(&output));
    diagnostics[0].clone()
}

#[test]
fn a_counterexample_binding_of_a_labeled_type_carries_its_label() {
    // ADR 0078: the solver's values are shown, and a binding whose declared type is labeled
    // above `public`, directly or through its owner, carries the label; a public one has no
    // `label` key. The key is added in every edition, and the human output is unchanged.
    let source = "module app

pub kind Card
  owner: Text
  pin: secret Int

pub card_pin(card: Card) -> secret Int
  ensures result > 0
=
  card.pin + 1
";
    for edition in ["0.1", "0.2"] {
        let name = format!("counterexample-labels-{edition}");
        let root = package_of_edition(&name, edition, &[("src/app.una", source)]);
        let output = una(&["--json", "check", root.to_str().unwrap()]);
        assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
        let document: Value = serde_json::from_str(stdout(&output)).unwrap();
        let diagnostic = &document["diagnostics"][0];
        assert_eq!(diagnostic["code"], "U5001", "{diagnostic}");
        let labels: Vec<(&str, Option<&str>)> = diagnostic["counterexample"]["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| {
                (
                    b["name"].as_str().unwrap(),
                    b.get("label").map(|l| l.as_str().unwrap()),
                )
            })
            .collect();
        assert_eq!(
            labels,
            [
                ("card", None),
                ("card.pin", Some("secret")),
                ("result", Some("secret"))
            ]
        );
        let output = una(&["check", root.to_str().unwrap()]);
        assert!(
            stderr(&output).contains("counterexample: card = Card#0, card.pin = "),
            "{}",
            stderr(&output)
        );
    }
}

/// The descriptions of a diagnostic's repairs, in order.
fn repair_descriptions(diagnostic: &Value) -> Vec<String> {
    diagnostic["repairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repair| repair["description"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn repairs_of_a_precondition_at_a_parameter_of_a_private_function() {
    // R-021, `spec/13 §3`: a stronger parameter type when the value is a parameter, a
    // `requires` clause for a private function, a guard, and `checked`.
    let diagnostic = only_diagnostic(
        "repairs-private-parameter",
        "module app

pub half(n: Int) -> Int
  requires n >= 0
=
  n / 2

halve(n: Int) -> Int = half(n)

pub run() -> Int = halve(4)
",
    );
    assert_eq!(diagnostic["code"], "U5001");
    assert_eq!(
        repair_descriptions(&diagnostic),
        [
            "strengthen the type of the parameter `n` to a refined kind that implies `n >= 0`",
            "add `requires n >= 0` to the enclosing function; its callers then prove it",
            "add a guard (`if` or a `when` arm) that handles the values where `n >= 0` does not hold",
            "check the value with `checked(value, n >= 0) else fail <Variant>`",
        ]
    );
    let costs: Vec<&Value> = diagnostic["repairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repair| &repair["escape_cost"])
        .collect();
    assert_eq!(costs, [&json!(0), &json!(0), &json!(0), &json!(1)]);
    // ADR 0095 rule 5: `checked` says what it costs; the others carry no `escape` key.
    let escapes: Vec<Option<&Value>> = diagnostic["repairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repair| repair.get("escape").map(|escape| &escape["kind"]))
        .collect();
    assert_eq!(escapes, [None, None, None, Some(&json!("checked"))]);
}

#[test]
fn a_refuted_refinement_at_a_value_that_is_not_a_parameter_has_no_parameter_repair() {
    // R-021, `spec/13 §3`: the stronger parameter type applies only when the value is a
    // parameter; `raw - 1` is not one, and a private function may still require it.
    let diagnostic = only_diagnostic(
        "repairs-not-a-parameter",
        "module app

pub kind Seats = Int in 0..5000

pub book(seats: Seats) -> Int = seats

unguarded(raw: Int) -> Int = book(raw - 1)

pub run() -> Int = unguarded(4)
",
    );
    assert_eq!(diagnostic["code"], "U5001");
    let repairs = repair_descriptions(&diagnostic);
    assert_eq!(repairs.len(), 3, "{repairs:?}");
    assert!(repairs[0].starts_with("add `requires "), "{repairs:?}");
    assert!(repairs[1].starts_with("add a guard"), "{repairs:?}");
    assert!(
        repairs[2].starts_with("check the value with"),
        "{repairs:?}"
    );
}

#[test]
fn a_refuted_precondition_inside_a_pub_function_has_no_requires_repair() {
    // R-021, `spec/13 §3`: a `requires` clause is proposed for a private function only.
    let diagnostic = only_diagnostic(
        "repairs-pub-function",
        "module app

pub half(n: Int) -> Int
  requires n >= 0
=
  n / 2

pub halve(n: Int) -> Int = half(n)
",
    );
    assert_eq!(diagnostic["code"], "U5001");
    assert_eq!(
        repair_descriptions(&diagnostic),
        [
            "strengthen the type of the parameter `n` to a refined kind that implies `n >= 0`",
            "add a guard (`if` or a `when` arm) that handles the values where `n >= 0` does not hold",
            "check the value with `checked(value, n >= 0) else fail <Variant>`",
        ]
    );
}

#[test]
fn a_refuted_precondition_on_an_example_do_line_has_no_requires_repair() {
    // R-021, `spec/13 §3`: an example has no `requires` clause, and no parameters.
    let diagnostic = only_diagnostic(
        "repairs-example",
        "module app

pub half(n: Int) -> Int
  requires n >= 0
=
  n / 2

pub minus(n: Int) -> Int = n - 10

example \"half of a smaller number\"
  do half(minus(3))
  expect result == 0
",
    );
    assert_eq!(diagnostic["code"], "U5001");
    let repairs = repair_descriptions(&diagnostic);
    assert_eq!(repairs.len(), 2, "{repairs:?}");
    assert!(repairs[0].starts_with("add a guard"), "{repairs:?}");
    assert!(
        repairs[1].starts_with("check the value with"),
        "{repairs:?}"
    );
}

#[test]
fn a_counterexample_at_a_call_site_shows_the_refuting_values() {
    // ADR 0092 rule 3: an `effect_condition` refuted at a call, whose callee drops the
    // condition, shows the solver's values in the text and the JSON output (#668).
    let source = "module main

pub capability C
  op emit(n: Int) -> Unit

pub kind S
  field: Text

g(x: Int, flag: Bool) -> Unit
  uses @c: C[emit]
=
  @c.emit(x)

pub f(x: Int, y: Int, s: S, flag: Bool) -> Unit
  uses @c: C[emit where not flag]
=
  g(x + y, flag: flag)
";
    let root = package_of_edition(
        "counterexample-call-site",
        "0.3",
        &[("src/main.una", source)],
    );
    let output = una(&["--json", "check", root.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let document: Value = serde_json::from_str(stdout(&output)).unwrap();
    let diagnostic = &document["diagnostics"][0];
    assert_eq!(diagnostic["code"], "U5001", "{diagnostic}");
    let bindings: Vec<(String, Value)> = diagnostic["counterexample"]["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| (b["name"].as_str().unwrap().to_owned(), b["value"].clone()))
        .collect();
    assert!(
        bindings.contains(&("flag".to_owned(), json!(true))),
        "{bindings:?}"
    );
    assert!(
        bindings
            .iter()
            .all(|(_, value)| value != &json!("")
                && !value.as_str().is_some_and(|v| v.ends_with('#'))),
        "{bindings:?}"
    );
    let output = una(&["check", root.to_str().unwrap()]);
    let text = stderr(&output);
    let line = text
        .lines()
        .find(|line| line.contains("counterexample:"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(line.contains("flag = true"), "{line}");
    assert!(
        !line.contains("= ,") && !line.contains("#,") && !line.ends_with('#'),
        "{line}"
    );
}

#[test]
fn a_sum_value_through_a_record_field_shows_in_the_counterexample() {
    // #902: a sum kind's value reached through a record field and passed to a function with no
    // `ensures` is shown by its tag, which no assertion of the query used. The model query
    // named it undeclared, and the counterexample was "not available" with a solver error.
    let source = "module shipping

kind Zone = Home | Neighbor | World

kind Parcel
  grams: Int
  zone: Zone

pub base_rate(grams: Int) -> Int
  requires grams > 0
  ensures result >= 790
=
  790

pub surcharge(zone: Zone) -> Int
=
  when zone
    is Home -> 0
    is Neighbor -> 600
    is World -> 1_800

pub rate(parcel: Parcel) -> Int
  requires parcel.grams > 0
  ensures result >= 790
=
  base_rate(parcel.grams) + parcel.zone.surcharge
";
    let root = package_of_edition(
        "counterexample-sum-field",
        "0.2",
        &[("src/shipping.una", source)],
    );
    let output = una(&["check", root.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let text = stderr(&output);
    assert!(text.contains("U5001"), "{text}");
    let line = text
        .lines()
        .find(|line| line.contains("counterexample:"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(!line.contains("not available"), "{line}");
    assert!(!line.contains("unknown constant"), "{line}");
    assert!(
        ["Home", "Neighbor", "World"]
            .iter()
            .any(|variant| line.contains(variant)),
        "{line}"
    );
}

#[test]
fn a_ground_goal_has_no_repair_and_no_counterexample() {
    // R-021, `spec/13 §3` ("when applicable"): `0 != 0` has no free binder, so no guard,
    // `requires` or type can make it hold, and a counterexample would bind nothing.
    let source = "module app

pub f(x: Int) -> Int = x / 0
";
    let diagnostic = only_diagnostic("repairs-ground", source);
    assert_eq!(diagnostic["code"], "U5001");
    assert_eq!(diagnostic["obligation"]["goal"], "0 != 0");
    assert_eq!(diagnostic["repairs"], json!([]));
    assert_eq!(diagnostic.get("counterexample"), None, "{diagnostic}");
    let root = package("repairs-ground-human", &[("src/app.una", source)]);
    let output = una(&["check", root.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("U5001"), "{}", stderr(&output));
    assert!(
        !stderr(&output).contains("counterexample:"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_closed_goal_without_bindings_has_no_counterexample() {
    // `spec/13 §3` ("when applicable"): `zero(1) > 0` has no free binder but is not a
    // literal, so the model binds nothing, and no empty counterexample is shown.
    let source = "module app

pub kind Pos = Int where it > 0

pub inv(p: Pos) -> Int = 100 / p

pub zero(x: Int) -> Int = 0

pub closed_call() -> Int = inv(zero(1))
";
    let diagnostic = only_diagnostic("counterexample-closed-goal", source);
    assert_eq!(diagnostic["code"], "U5001");
    assert_eq!(diagnostic.get("counterexample"), None, "{diagnostic}");
    let root = package(
        "counterexample-closed-goal-human",
        &[("src/app.una", source)],
    );
    let output = una(&["check", root.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("U5001"), "{}", stderr(&output));
    assert!(
        !stderr(&output).contains("counterexample:"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_refutation_through_uninterpreted_arithmetic_is_validated_ui004() {
    // Issue #72 (UI-004), ADR 0053: `a * b` of two values is the uninterpreted `i!mul!Int`
    // (`spec/12 §2`). A model whose products disagree with real arithmetic is no execution,
    // so the obligation is unknown (U5002), with no counterexample and the same repairs.
    let product = only_diagnostic(
        "uninterpreted-product",
        "module app\n\npub product(a: Nat, b: Nat) -> Nat = a * b\n",
    );
    assert_eq!(product["code"], "U5002");
    assert_eq!(
        product["message"],
        "refinement `result >= 0` could not be proven (the refutation depends on a product, quotient or remainder of two values, which the encoding of `spec/12 §2` leaves uninterpreted)"
    );
    assert_eq!(product["obligation"]["status"], "unknown");
    assert!(product.get("counterexample").is_none(), "{product}");
    assert!(!product["repairs"].as_array().unwrap().is_empty());
    // A model whose products agree with real arithmetic is an execution: refuted, with the
    // counterexample and the plain message.
    let real = only_diagnostic(
        "real-product",
        "module app\n\npub six(a: Int, b: Int) -> Int\n  requires a == 2 and b == 3\n  ensures result != 6\n=\n  a * b\n",
    );
    assert_eq!(real["code"], "U5001");
    assert_eq!(real["message"], "postcondition of `six` is refuted");
    assert_eq!(
        real["counterexample"]["bindings"],
        json!([{"name": "a", "value": 2}, {"name": "b", "value": 3}, {"name": "result", "value": 6}])
    );
    let linear = only_diagnostic(
        "linear-refutation",
        "module app\n\npub less(a: Nat) -> Nat = a - 1\n",
    );
    assert_eq!(linear["message"], "refinement `result >= 0` is refuted");
}

#[test]
fn a_real_counterexample_through_a_product_is_found_again_v12_001() {
    // Issue #104 (V12-001), ADR 0054: when the first model disagrees with real arithmetic,
    // the query is solved once more with the arithmetic interpreted, so a real refutation
    // keeps its status and its counterexample.
    let square = only_diagnostic(
        "real-square",
        "module app\n\npub kind Pos = Int where it > 0\n\npub sq(a: Int) -> Pos = a * a\n",
    );
    assert_eq!(square["code"], "U5001");
    assert_eq!(
        square["counterexample"]["bindings"],
        json!([{"name": "a", "value": 0}, {"name": "result", "value": 0}])
    );
    let quotient = only_diagnostic(
        "real-quotient",
        "module app\n\npub half(x: Int) -> Int\n  requires x == 7\n  ensures result == 3\n=\n  x / (0 - 2)\n",
    );
    assert_eq!(quotient["code"], "U5001");
    assert_eq!(
        quotient["counterexample"]["bindings"],
        json!([{"name": "result", "value": -3}, {"name": "x", "value": 7}])
    );
}

#[test]
fn the_value_that_if_none_use_unwraps_is_named_as_written() {
    // Final review F-004: the ADR 0037 obligation at an annotated `let` names the value that
    // `if none use` unwraps by its expression, not by an internal temporary.
    let unwrapped = only_diagnostic(
        "if-none-unwrapped",
        "module app\n\npub kind Pos = Int where it > 0\n\none(x: Int) -> Pos = 1\n\npub first_fn(fs: List[fn(Int) -> Int]) -> Int\n=\n  let f: fn(Int) -> Pos = fs.first()\n    if none use one\n  f(1)\n",
    );
    assert_eq!(
        unwrapped["message"],
        "refinement `result of fs.first(): result > 0` is refuted"
    );
}

#[test]
fn a_function_with_a_precondition_passed_as_a_value_is_named_as_such() {
    // Issue #76 (UI-007): the precondition is named as written, the message says that the
    // function type has none, and the only repair is a lambda that establishes it.
    let value = only_diagnostic(
        "precondition-function-value",
        "module app\n\npub same(a: Int, b: Int) -> Int\n  requires a == b\n  ensures result == 0\n=\n  a - b\n\npub apply(f: fn(Int, Int) -> Int) -> Int = f(1, 2)\n\npub use_it() -> Int = apply(same)\n",
    );
    assert_eq!(value["code"], "U5001");
    assert_eq!(
        value["message"],
        "precondition `a == b` of `same`, passed as a function value whose type has no precondition, is refuted"
    );
    let repairs = value["repairs"].as_array().unwrap();
    assert_eq!(repairs.len(), 1);
    assert_eq!(
        repairs[0]["description"],
        "pass a lambda that calls `same` only where `a == b` holds, as in `fn(a, b) -> if a == b then same(a, b) else ...`"
    );
}

#[test]
fn a_function_value_repair_guards_every_precondition_v12_003() {
    // Issue #106 (V12-003): with two `requires`, each diagnostic's repair guards both, so
    // applying it settles them all; a function that can fail gets a named helper with a
    // handling clause, since a failing call must be a whole statement (U3013).
    let diagnostics = |name: &str, source: &str| -> Vec<Value> {
        let root = package(name, &[("src/app.una", source)]);
        let output = una(&["--json", "check", root.to_str().unwrap()]);
        let json: Value = serde_json::from_str(stdout(&output)).unwrap();
        json["diagnostics"].as_array().unwrap().clone()
    };
    let pure = diagnostics(
        "two-requires",
        "module app\n\npub same(a: Int, b: Int) -> Int\n  requires a == b\n  requires a > 0\n=\n  a - b\n\npub apply(f: fn(Int, Int) -> Int) -> Int = f(1, 2)\n\npub use_it() -> Int = apply(same)\n",
    );
    assert_eq!(pure.len(), 2);
    for diagnostic in &pure {
        assert_eq!(
            diagnostic["repairs"][0]["description"],
            "pass a lambda that calls `same` only where `a == b and a > 0` holds, as in `fn(a, b) -> if a == b and a > 0 then same(a, b) else ...`"
        );
    }
    let failing = diagnostics(
        "failing-callee",
        "module app\n\npub kind Bad = | Broke\n\npub same(a: Int, b: Int) -> Int\n  requires a == b\n  fails Bad\n=\n  if a > 5 then fail Broke else a - b\n\npub apply(f: fn(Int, Int) -> Int fails Bad) -> Int\n  fails Bad\n=\n  let v = f(1, 2)\n    if error pass\n  v\n\npub use_it() -> Int\n  fails Bad\n=\n  let v = apply(same)\n    if error pass\n  v\n",
    );
    assert_eq!(failing.len(), 1);
    assert_eq!(
        failing[0]["repairs"][0]["description"],
        "pass a named function that calls `same` only where `a == b` holds: `same_where_defined(a: Int, b: Int) -> Int fails Bad`, whose body is `if a == b` with `let v = same(a, b)`, `if error pass` and `v` in its block, and a value in its `else` block"
    );
}

#[test]
fn a_counterexample_that_divides_by_zero_points_at_the_division_399() {
    // ADR 0054, amendment of 2026-10-08 (#399, V-1): a model whose path divides by zero is
    // no run, so the refuted postcondition shows no counterexample and no repair, and points
    // at the division, whose own obligation keeps its counterexample `b = 0`. A zero divisor on
    // a path that does not divide keeps the counterexample.
    let root = package(
        "zero-divisor",
        &[(
            "src/app.una",
            "module app\n\npub divided(a: Int, b: Int) -> Int\n  ensures result == 7\n=\n  a / b\n\npub skipped(a: Int, b: Int) -> Int\n  ensures result != 5\n=\n  if b == 0 then 5 else a / b\n",
        )],
    );
    let output = una(&["--json", "check", root.to_str().unwrap()]);
    let json: Value = serde_json::from_str(stdout(&output)).unwrap();
    let found = json["diagnostics"].as_array().unwrap();
    let by_message = |message: &str| {
        found
            .iter()
            .find(|d| d["message"] == message)
            .unwrap_or_else(|| panic!("no `{message}` in {found:?}"))
    };
    let divided = by_message("postcondition of `divided` is refuted");
    assert!(divided["counterexample"].is_null(), "{divided}");
    assert_eq!(divided["repairs"], json!([]));
    let note = divided["related"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| {
            r["message"]
                .as_str()
                .unwrap()
                .starts_with("the refuting path divides by zero")
        })
        .unwrap_or_else(|| panic!("no note in {divided}"));
    assert_eq!(
        note["message"],
        "the refuting path divides by zero here, so it is no run: fix this division, whose own obligation reports it"
    );
    assert_eq!(note["start"], json!({"line": 6, "col": 3}));
    let division = by_message("division obligation `b != 0` is refuted");
    assert_eq!(
        division["counterexample"],
        json!({"bindings": [{"name": "b", "value": 0}]})
    );
    let skipped = by_message("postcondition of `skipped` is refuted");
    assert_eq!(
        skipped["counterexample"]["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["name"] == "result")
            .unwrap()["value"],
        5
    );
}

#[test]
fn a_helper_repair_writes_the_header_as_the_value_is_used_p1_126() {
    // #126 (P-1 of the deferred review): the helper of a failing callee printed its header
    // without labels, so written as described it was U4001. It now has the labels, the
    // refinements and the type arguments of the use, and checks once written. A guard on
    // labeled data around a call that can fail would reveal that data (U4002), so there the
    // repair says why no helper fits.
    let callee = "module app\n\npub kind Bad = | Broke\n\npub kind Pos = Int where it > 0\n\npub pick[T](a: Int, b: Pos, x: secret T) -> secret T\n  requires a == b\n  fails Bad\n=\n  if b > 5 then fail Broke else x\n\npub apply(f: fn(Int, Pos, secret Text) -> secret Text fails Bad) -> secret Text\n  fails Bad\n=\n  let v = f(1, 2, \"x\")\n    if error pass\n  v\n";
    let check = |name: &str, source: &str| -> Vec<Value> {
        let root = package(name, &[("src/app.una", source)]);
        let output = una(&["--json", "check", root.to_str().unwrap()]);
        let json: Value = serde_json::from_str(stdout(&output)).unwrap();
        json["diagnostics"].as_array().unwrap().clone()
    };
    let passed = format!(
        "{callee}\npub use_it() -> secret Text\n  fails Bad\n=\n  let v = apply(pick)\n    if error pass\n  v\n"
    );
    let found = check("helper-labeled", &passed);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0]["repairs"][0]["description"],
        "pass a named function that calls `pick` only where `a == b` holds: `pick_where_defined(a: Int, b: Pos, x: secret Text) -> secret Text fails Bad`, whose body is `if a == b` with `let v = pick(a, b, x)`, `if error pass` and `v` in its block, and a value in its `else` block"
    );
    // The helper as the repair describes it, with `x` in its `else` block.
    let applied = format!(
        "{callee}\npub use_it() -> secret Text\n  fails Bad\n=\n  let v = apply(pick_where_defined)\n    if error pass\n  v\n\npick_where_defined(a: Int, b: Pos, x: secret Text) -> secret Text\n  fails Bad\n=\n  if a == b\n    let v = pick(a, b, x)\n      if error pass\n    v\n  else\n    x\n"
    );
    assert_eq!(
        check("helper-labeled-applied", &applied),
        Vec::<Value>::new()
    );
    let guarded = check(
        "helper-secret-guard",
        "module app\n\npub kind Bad = | Broke\n\npub hidden(a: secret Int, b: Int) -> Int\n  requires a == b\n  fails Bad\n=\n  if b > 5 then fail Broke else b\n\npub apply(f: fn(secret Int, Int) -> Int fails Bad) -> Int\n  fails Bad\n=\n  let v = f(1, 2)\n    if error pass\n  v\n\npub use_it() -> Int\n  fails Bad\n=\n  let v = apply(hidden)\n    if error pass\n  v\n",
    );
    assert_eq!(guarded.len(), 1, "{guarded:?}");
    assert_eq!(
        guarded[0]["repairs"][0]["description"],
        "`hidden` can fail, and `a == b` reads `secret` data, so a guard around the call would reveal that data through its failure (U4002): pass a function whose type has no precondition and that cannot fail on these values, or make `hidden`'s precondition hold for every argument"
    );
}

#[test]
fn a_function_value_flowing_into_a_function_type_names_its_obligation() {
    // ADR 0037 rules 2 to 6 (F-0001-02, R-013): the goal names the position and the binder,
    // the counterexample binds the binder by its source name, and the only repair is a
    // stronger type at a position of a function type.
    let parameter = only_diagnostic(
        "fn-type-parameter",
        "module app

pub kind Pos = Int where it > 0

pub apply(f: fn(Int) -> Int, x: Int) -> Int = f(x)

pub takes_pos(x: Pos) -> Int = x

pub use_it() -> Int = apply(takes_pos, x: 0)
",
    );
    assert_eq!(parameter["code"], "U5001");
    assert_eq!(
        parameter["message"],
        "refinement `parameter x of takes_pos: x > 0` is refuted"
    );
    assert_eq!(
        parameter["counterexample"],
        json!({"bindings": [{"name": "x", "value": 0}]})
    );
    assert_eq!(
        repair_descriptions(&parameter),
        ["strengthen parameter 1 of `fn(Int) -> Int` to a refined kind that implies `x > 0`"]
    );
    // Rule 12: a lambda whose annotation is stronger than the expected parameter type.
    let lambda = only_diagnostic(
        "fn-type-lambda",
        "module app

pub kind Pos = Int where it > 0

pub apply(f: fn(Int) -> Int, x: Int) -> Int = f(x)

pub use_it() -> Int = apply(fn(y: Pos) -> y, x: 0)
",
    );
    assert_eq!(
        lambda["message"],
        "refinement `parameter y of the lambda: y > 0` is refuted"
    );
    assert_eq!(
        lambda["primary"],
        json!({"file": "src/app.una", "start": {"line": 7, "col": 29}, "end": {"line": 7, "col": 44}})
    );
    assert_eq!(
        lambda["counterexample"],
        json!({"bindings": [{"name": "y", "value": 0}]})
    );
    // Rule 12: a function whose result is less refined than the expected result, with a
    // named obligation and a non-empty counterexample; the call through the value assumes
    // the expected result, so `apply_pos` itself is proven (rule 7).
    let result = only_diagnostic(
        "fn-type-result",
        "module app

pub kind Pos = Int where it > 0

pub apply_pos(f: fn(Int) -> Pos, x: Int) -> Pos = f(x)

pub gives_int(x: Int) -> Int = x

pub use_it() -> Int = apply_pos(gives_int, x: 0)
",
    );
    assert_eq!(
        result["message"],
        "refinement `result of gives_int: result > 0` is refuted"
    );
    assert_eq!(result["obligation"]["kind"], "refinement");
    assert_eq!(
        result["obligation"]["goal"],
        "result of gives_int: result > 0"
    );
    assert_eq!(result["related"][0]["message"], "where clause");
    let bindings = result["counterexample"]["bindings"].as_array().unwrap();
    assert!(
        bindings
            .iter()
            .any(|b| b["name"] == "result" && b["value"].as_i64().is_some_and(|v| v <= 0)),
        "{bindings:?}"
    );
    assert_eq!(
        repair_descriptions(&result),
        ["strengthen the result of `fn(Int) -> Int` to a refined kind that implies `result > 0`"]
    );
}

#[test]
fn a_lambda_given_a_refined_result_proves_it_on_its_paths() {
    // `spec/12 §3`, returns: a lambda's body flows into the result its expected type gives it.
    let diagnostic = only_diagnostic(
        "fn-type-lambda-result",
        "module app

pub kind Pos = Int where it > 0

pub apply(g: fn(Int) -> Pos) -> Int = 100 / g(0)

pub use_it(c: Bool) -> Int = apply(fn(x) -> if c then 1 else 0)
",
    );
    assert_eq!(
        diagnostic["message"],
        "refinement `result of the lambda: result > 0` is refuted"
    );
    assert_eq!(
        diagnostic["primary"]["start"],
        json!({"line": 7, "col": 62})
    );
    assert_eq!(
        repair_descriptions(&diagnostic),
        [
            "add a guard or an arm so that the failing path of the lambda returns a value for which `result > 0` holds"
        ]
    );
}

/// The goals of the diagnostics of a conformance case, as "code line:col goal".
fn case_goals(case: &str) -> Vec<String> {
    let output = una(&["--json", "check", &format!("tests/conformance/{case}")]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let document: Value = serde_json::from_str(stdout(&output)).unwrap();
    document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            format!(
                "{} {}:{} {}",
                d["code"].as_str().unwrap(),
                d["primary"]["start"]["line"],
                d["primary"]["start"]["col"],
                d["obligation"]["goal"].as_str().unwrap()
            )
        })
        .collect()
}

#[test]
fn goal_texts_name_the_parameters_of_a_function_value_and_print_literals_as_values() {
    // R-032: the precondition of a function used as a value is instantiated at the
    // parameters of the lambda that stands for it, and a literal arm prints its value.
    assert_eq!(
        case_goals("verification/fn_value_precondition_goal_text.una"),
        ["U5001 19:17 x == y", "U5001 20:18 needs > 0"]
    );
    assert_eq!(
        case_goals("verification/literal_arms_goal_text.una"),
        ["U5001 10:3 n == 0 or n == 2"]
    );
}

/// The program of the review's `verify-unknown-is-error-cached`: three obligations, which a
/// step budget of 1 leaves unknown.
const UNKNOWN: &str = "module unk

recip(x: Int) -> Int
  requires x > 0
=
  100 / x

pub main() -> Unit
  uses @console: Console[print]
=
  let q = recip(0 - 0)
  @console.print(\"q = {q}\")

example \"zero\"
  do recip(0)
  expect result == 1
";

/// The codes of a JSON document's diagnostics, and whether each obligation came from the
/// cache.
fn codes_and_cached(output: &Output) -> (Vec<String>, Vec<bool>) {
    let document: Value = serde_json::from_str(stdout(output)).unwrap();
    let codes = document["diagnostics"]
        .as_array()
        .map(|d| {
            d.iter()
                .map(|d| d["code"].as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default();
    let cached = document["obligations"]
        .as_array()
        .map(|o| o.iter().map(|o| o["cached"] == true).collect())
        .unwrap_or_default();
    (codes, cached)
}

#[test]
fn an_unknown_obligation_is_an_error_every_run_and_only_a_wallclock_unknown_is_not_cached() {
    // Review control `verify-unknown-is-error-cached` (ADR 0008, ADR 0033, `spec/12 §6`):
    // with `step_budget = 1` every obligation is unknown, U5002 on the first run and again on
    // the second, answered by the cache; `una run` and `una test` refuse to run (exit 1).
    let root = package("unknown-cached", &[("src/unk.una", UNKNOWN)]);
    std::fs::write(
        root.join("una.toml"),
        "[package]\nname = \"unknown_cached\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n\n[run]\nentry = \"unk\"\n",
    )
    .unwrap();
    let lock = std::fs::read_to_string(root.join("verification.lock")).unwrap();
    let lock = lock.replace("step_budget      = 5000000", "step_budget      = 1");
    assert!(lock.contains("step_budget      = 1\n"), "{lock}");
    std::fs::write(root.join("verification.lock"), lock).unwrap();
    let path = root.to_str().unwrap();
    for (run, cached) in [(1, false), (2, true)] {
        let obligations = una(&["--json", "obligations", path]);
        assert_eq!(obligations.status.code(), Some(1), "run {run}");
        let document: Value = serde_json::from_str(stdout(&obligations)).unwrap();
        let listed = document["obligations"].as_array().unwrap();
        assert_eq!(listed.len(), 3, "run {run}");
        assert!(
            listed
                .iter()
                .all(|o| o["status"] == "unknown" && o["cached"] == cached),
            "run {run}: {}",
            stdout(&obligations)
        );
        let check = una(&["--json", "check", path]);
        assert_eq!(check.status.code(), Some(1), "run {run}");
        assert_eq!(
            codes_and_cached(&check).0,
            ["U5002", "U5002", "U5002"],
            "run {run}"
        );
    }
    for args in [
        ["--json", "run", "--grant", "console", path].as_slice(),
        ["--json", "test", path].as_slice(),
    ] {
        let output = una(args);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{args:?}: {}",
            stdout(&output)
        );
        assert!(stdout(&output).contains("U5002"), "{args:?}");
        assert!(!stdout(&output).contains("q = "), "{args:?}");
    }
    // `--solver-wallclock 0` stops every query: unknown with reason `wallclock`, which the
    // cache never keeps, so the second run asks the solver again.
    let root = package("unknown-wallclock", &[("src/unk.una", UNKNOWN)]);
    let path = root.to_str().unwrap();
    for run in 1..=2 {
        let output = una(&["--json", "obligations", "--solver-wallclock", "0", path]);
        assert_eq!(output.status.code(), Some(1), "run {run}");
        let (_, cached) = codes_and_cached(&output);
        assert_eq!(cached, [false, false, false], "run {run}");
        let check = una(&["--json", "check", "--solver-wallclock", "0", path]);
        let document: Value = serde_json::from_str(stdout(&check)).unwrap();
        let messages: Vec<&str> = document["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["message"].as_str().unwrap())
            .collect();
        assert_eq!(messages.len(), 3, "run {run}");
        assert!(
            messages
                .iter()
                .all(|m| m.ends_with("could not be proven (wallclock)")),
            "run {run}: {messages:?}"
        );
    }
}

#[test]
fn an_overflow_repair_guards_with_code_that_verifies_issue_28() {
    // `spec/18 §3`: the first repair of an `overflow` obligation is a guard that is valid
    // code and cannot overflow itself: the operation on `Int`, or the operand's range for a
    // conversion. Applying it verifies.
    for (name, header, operation, guard) in [
        (
            "overflow-add",
            "pub add(a: U8, b: U8) -> U8",
            "a + b",
            "a.to_int() + b.to_int() in 0..255",
        ),
        (
            "overflow-narrow",
            "pub narrow(a: I32) -> U8",
            "a.to_u8()",
            "a in 0..255",
        ),
        (
            "overflow-negate",
            "pub negate(a: I8) -> I8",
            "-a",
            "-a.to_int() in -128..127",
        ),
    ] {
        let failing = format!("module app\n\n{header} = {operation}\n");
        let root = package_of_edition(name, "0.2", &[("src/app.una", &failing)]);
        let output = una(&["--json", "check", root.to_str().unwrap()]);
        assert_eq!(output.status.code(), Some(1), "{name}: {}", stderr(&output));
        let document: Value = serde_json::from_str(stdout(&output)).unwrap();
        let diagnostics = document["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 1, "{name}: {}", stdout(&output));
        assert_eq!(diagnostics[0]["code"], "U5001", "{name}");
        let repairs = repair_descriptions(&diagnostics[0]);
        assert_eq!(
            repairs.first().map(String::as_str),
            Some(format!("guard the operation with `if {guard}`").as_str()),
            "{name}"
        );
        let guarded = format!("module app\n\n{header} = if {guard} then {operation} else 0\n");
        let root = package_of_edition(
            &format!("{name}-guarded"),
            "0.2",
            &[("src/app.una", &guarded)],
        );
        let output = una(&["check", root.to_str().unwrap()]);
        assert_eq!(output.status.code(), Some(0), "{name}: {}", stderr(&output));
    }
}

#[test]
fn a_forged_or_foreign_cache_entry_is_solved_again() {
    // ADR 0064 (v0.1 review R-018): an entry is an answer only under the user's key, so an edit
    // of `.una/cache/proofs/` cannot turn a refutation into a proof.
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("forgery");
    let _ = std::fs::remove_dir_all(&scratch);
    let package = scratch.join("div0");
    std::fs::create_dir_all(package.join("src")).unwrap();
    let source =
        repository().join("docs/reviews/v0.1-review/adversarial/escape-proof-cache-forgery");
    for file in ["una.toml", "verification.lock", "src/div0.una"] {
        std::fs::copy(source.join(file), package.join(file)).unwrap();
    }
    let run = |home: &str, key: Option<&Path>, args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_una"));
        command
            .env("HOME", scratch.join(home))
            .env_remove("UNA_PROOF_KEY")
            .args(args)
            .arg(&package);
        if let Some(key) = key {
            command.env("UNA_PROOF_KEY", key);
        }
        command.output().unwrap()
    };
    let report = |home: &str, key: Option<&Path>| {
        let output = run(home, key, &["--json", "report"]);
        let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        (
            document["obligations"]["cached"].clone(),
            document["tcb"]["proof_cache_key"].clone(),
        )
    };
    let refuted =
        |output: &Output| output.status.code() == Some(1) && stderr(output).contains("U5001");
    // The first run solves and caches the refutation under the key it makes: a missing key is
    // made, with no note (#1164).
    let first = run("alice", None, &["check"]);
    assert!(refuted(&first));
    assert!(
        !stderr(&first).contains("proof cache key"),
        "{}",
        stderr(&first)
    );
    assert_eq!(
        report("alice", None),
        (json!(1), json!("~/.una/proof-cache.key"))
    );
    // R-018's forgery: every entry overwritten with a bare `proven`.
    let proofs = package.join(".una/cache/proofs");
    for entry in std::fs::read_dir(&proofs).unwrap() {
        let path = entry.unwrap().path();
        std::fs::write(&path, r#"{"status":"proven","reason":null,"values":[]}"#).unwrap();
    }
    assert!(refuted(&run("alice", None, &["check"])));
    // Another user's key does not authenticate alice's entries; the query is solved again.
    assert_eq!(
        report("bob", None),
        (json!(0), json!("~/.una/proof-cache.key"))
    );
    // With no key to read, the cache is off.
    let unreadable = scratch.join("a-directory");
    std::fs::create_dir_all(&unreadable).unwrap();
    assert_eq!(report("alice", Some(&unreadable)), (json!(0), json!(null)));
    // It exists and is not a regular file: a note says the cache is off (#1164).
    let noted = run("alice", Some(&unreadable), &["check"]);
    assert!(
        stderr(&noted).contains(&format!(
            "note: the proof cache key `{}` is not used, since it is not a regular file; the proof cache is off\n",
            unreadable.display()
        )),
        "{}",
        stderr(&noted)
    );
    // The override names the key file, as the report shows it.
    let named = scratch.join("team.key");
    std::fs::write(&named, [7u8; 32]).unwrap();
    // A key file is used only when it is a private regular file of the user (#1164): one the
    // group or others can read is refused with a note, and the 0600 one is used, with none.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&named, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(report("carol", Some(&named)).1, json!(null));
        let noted = run("carol", Some(&named), &["check"]);
        assert!(
            stderr(&noted).contains(
                "is not used, since the group or others have permissions on it; the proof cache is off (chmod 600 it)"
            ),
            "{}",
            stderr(&noted)
        );
        // Under `--json`, stderr stays empty (`spec/13 §5`): the report's null is the signal.
        let quiet = run("carol", Some(&named), &["--json", "check"]);
        assert_eq!(stderr(&quiet), "");
        let _: serde_json::Value = serde_json::from_slice(&quiet.stdout).unwrap();
        // A key of the wrong size is noted too, with no permission hint.
        let short = scratch.join("short.key");
        std::fs::write(&short, b"short").unwrap();
        std::fs::set_permissions(&short, std::fs::Permissions::from_mode(0o600)).unwrap();
        let noted = run("carol", Some(&short), &["check"]);
        assert!(
            stderr(&noted).contains(
                "is not used, since its size is outside 16 to 4096 bytes; the proof cache is off\n"
            ),
            "{}",
            stderr(&noted)
        );
        std::fs::set_permissions(&named, std::fs::Permissions::from_mode(0o600)).unwrap();
        let used = run("carol", Some(&named), &["check"]);
        assert!(
            !stderr(&used).contains("proof cache key"),
            "{}",
            stderr(&used)
        );
    }
    let shown = json!(named.to_str().unwrap());
    assert_eq!(report("carol", Some(&named)).1, shown);
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn a_damaged_cache_entry_is_solved_again_and_never_reused_11() {
    // #11 (R-037, cache robustness): every entry of a refuted package's cache, damaged in each
    // way a disk or an editor can damage it, reads as no answer. The query is solved again:
    // the obligation stays refuted and none of its results comes from the cache.
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("damaged-cache");
    let _ = std::fs::remove_dir_all(&scratch);
    let package = scratch.join("div0");
    std::fs::create_dir_all(package.join("src")).unwrap();
    let source =
        repository().join("docs/reviews/v0.1-review/adversarial/escape-proof-cache-forgery");
    for file in ["una.toml", "verification.lock", "src/div0.una"] {
        std::fs::copy(source.join(file), package.join(file)).unwrap();
    }
    let report = || {
        let output = Command::new(env!("CARGO_BIN_EXE_una"))
            .env("HOME", scratch.join("home"))
            .env_remove("UNA_PROOF_KEY")
            .args(["--json", "report"])
            .arg(&package)
            .output()
            .unwrap();
        let document: Value = serde_json::from_slice(&output.stdout).unwrap();
        document["obligations"].clone()
    };
    assert_eq!(
        report(),
        json!({"proven": 0, "refuted": 1, "unknown": 0, "cached": 0})
    );
    let proofs = package.join(".una/cache/proofs");
    let entries = || -> Vec<PathBuf> {
        std::fs::read_dir(&proofs)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|e| e == "json"))
            .collect()
    };
    assert!(!entries().is_empty());
    // The refutation, answered by the cache on the second run.
    assert_eq!(
        report(),
        json!({"proven": 0, "refuted": 1, "unknown": 0, "cached": 1})
    );
    let flip = |text: &str| {
        let mut entry: Value = serde_json::from_str(text).unwrap();
        entry["answer"]["status"] = json!("proven");
        entry["answer"]["values"] = json!([]);
        serde_json::to_vec(&entry).unwrap()
    };
    type Damage = fn(&str, &dyn Fn(&str) -> Vec<u8>) -> Vec<u8>;
    let damages: [(&str, Damage); 5] = [
        ("an answer flipped to proven under its tag", |text, flip| {
            flip(text)
        }),
        ("a truncated file", |text, _| {
            text.as_bytes()[..text.len() / 2].to_vec()
        }),
        ("bytes that are not utf-8", |_, _| vec![0xff, 0xfe, 0x7b]),
        ("an empty file", |_, _| Vec::new()),
        ("an entry with no tag", |_, _| {
            br#"{"status":"proven","reason":null,"values":[]}"#.to_vec()
        }),
    ];
    for (what, damage) in damages {
        for path in entries() {
            let text = std::fs::read_to_string(&path).unwrap();
            std::fs::write(&path, damage(&text, &flip)).unwrap();
        }
        assert_eq!(
            report(),
            json!({"proven": 0, "refuted": 1, "unknown": 0, "cached": 0}),
            "{what}"
        );
    }
    let _ = std::fs::remove_dir_all(&scratch);
}

#[cfg(unix)]
#[test]
fn a_query_the_solver_rejects_is_an_internal_error_11() {
    // #11 (R-037): a solver that answers `(error ...)` rejected a query the encoder wrote, a
    // bug of the encoding: U8001 with exit code 3 (`spec/13 §5`), never a verdict. A stand-in
    // `z3` on the PATH reports the locked version and rejects every query.
    use std::os::unix::fs::PermissionsExt;
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("solver-error");
    let _ = std::fs::remove_dir_all(&scratch);
    let bin = scratch.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let lock = std::fs::read_to_string(repository().join("verification.lock")).unwrap();
    let version = lock
        .lines()
        .find_map(|line| line.strip_prefix("solver_version"))
        .and_then(|rest| rest.split('"').nth(1))
        .unwrap()
        .to_owned();
    let staged = bin.join("z3.tmp");
    // A stand-in for the solver child (ADR 0124; the test hooks): it writes the handshake it
    // is given, reads the query and rejects it. The version check is against the solver built
    // into `una`, with no process started (rule 10), so the pinned version is not printed.
    let _ = version;
    std::fs::write(
        &staged,
        "#!/bin/sh\necho \"$1\"\ncat > /dev/null\necho '(error \"line 1 column 1: unknown constant\")'\n",
    )
    .unwrap();
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&staged, bin.join("z3")).unwrap();
    let root = package(
        "solver-error",
        &[(
            "src/app.una",
            "module app\n\npub kind Pos = Int where it > 0\n\npub one() -> Pos = 1\n",
        )],
    );
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", scratch.join("home"))
        .env_remove("UNA_PROOF_KEY")
        .env("UNA_SOLVER_STAND_IN", bin.join("z3"))
        .args(["--json", "check"])
        .arg(&root)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", stdout(&output));
    let document: Value = serde_json::from_str(stdout(&output)).unwrap();
    let diagnostics = document["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0]["code"], "U8001");
    let message = diagnostics[0]["message"].as_str().unwrap();
    assert!(
        message.starts_with("internal error: obligation ob-")
            && message.contains("could not be encoded: the solver rejected the query: (error"),
        "{message}"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn an_interpolation_names_what_it_cannot_write_and_the_constraint_it_needs() {
    // ADR 0068 D4 and D8: U2001 names the position of the function, through a list, and U2026
    // says that interpolation needs `Eq`, with the repair that writes it.
    let diagnostic = |name: &str, source: &str| {
        let root = package_of_edition(name, "0.2", &[("src/app.una", source)]);
        let output = una(&["--json", "check", root.to_str().unwrap()]);
        let document: Value = serde_json::from_str(stdout(&output)).unwrap();
        let diagnostics = document["diagnostics"].as_array().unwrap().clone();
        assert_eq!(diagnostics.len(), 1, "{}", stdout(&output));
        diagnostics[0].clone()
    };
    let function = diagnostic(
        "interpolate-function",
        "module app\n\npub kind Step\n  run: fn(Int) -> Int\n\npub kind Plan\n  steps: List[Step]\n\npub shown(p: Plan) -> Text = \"plan {p}\"\n",
    );
    assert_eq!(function["code"], "U2001");
    assert_eq!(
        function["message"],
        "interpolation writes values that hold no function and no `Hashed`, and field `run` of `Step` holds a function"
    );
    let unconstrained = diagnostic(
        "interpolate-param",
        "module app\n\npub shown[T](x: T) -> Text = \"is {x}\"\n",
    );
    assert_eq!(unconstrained["code"], "U2026");
    assert_eq!(
        unconstrained["message"],
        "interpolation on values of the type parameter `T` needs the constraint `Eq`; write `[T: Eq]`"
    );
    assert_eq!(
        repair_descriptions(&unconstrained),
        ["constrain `T` by `Eq`"]
    );
    // D8: U4003 at a sink names the labeled field of the interpolated record.
    let secret = diagnostic(
        "interpolate-secret",
        "module app\n\npub kind Account\n  owner: Text\n  pin: secret Int\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"account {open()}\")\n\nopen() -> Account = Account(owner: \"Ana\", pin: 1)\n",
    );
    assert_eq!(secret["code"], "U4003");
    assert_eq!(
        repair_descriptions(&secret),
        [
            "field `pin` of `Account` is `secret`: interpolate only the fields that `@console.print` accepts",
            "declassify `pin` through a policy of a trusted module",
        ]
    );
    // ADR 0095: the policy widens the trusted base, so it is the last repair, marked.
    assert_eq!(secret["repairs"][1]["escape"]["kind"], "trusted");
    assert!(secret["repairs"][0].get("escape").is_none());
}

#[test]
fn an_assignment_s_counterexample_shows_the_value_before_it() {
    // #277: the refinement of `n = n - 1` is about the new value; the counterexample shows the
    // `n` the right-hand side reads, and the new value as `n (assigned)`.
    let root = package_of_edition(
        "assigned",
        "0.2",
        &[(
            "src/app.una",
            "module app

pub kind Pos = Int where it >= 1

pub step(start: Pos) -> Pos
=
  let edit n: Pos = start
  n = n - 1
  n
",
        )],
    );
    let output = una(&["--json", "check", root.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let document: Value = serde_json::from_str(stdout(&output)).unwrap();
    let diagnostic = &document["diagnostics"][0];
    assert_eq!(diagnostic["code"], "U5001");
    assert_eq!(
        diagnostic["counterexample"],
        json!({"bindings": [
            {"name": "n", "value": 1},
            {"name": "n (assigned)", "value": 0},
            {"name": "start", "value": 1}
        ]})
    );
}

/// A function with `n` editable bindings, all updated in one loop, whose postcondition reads
/// the loop's final state: the loop state is a tuple of `n` components (`spec/17 §4`).
fn loop_state_source(n: usize) -> String {
    let mut out = String::from(
        "module wide\n\npub f(x: Int) -> Int\n  requires x >= 0\n  ensures result >= 0\n=\n",
    );
    for i in 0..n {
        out.push_str(&format!("  let edit e{i} = x\n"));
    }
    out.push_str("  for k in 0..<2\n");
    for i in 0..n {
        out.push_str(&format!("    e{i} = e{i} + 1\n"));
    }
    out.push_str("  e0\n");
    out
}

/// The bytes of every query of the package `wide` with a loop state of `n` components.
fn loop_state_query_bytes(n: usize) -> usize {
    let root = package_of_edition(
        &format!("wide-{n}"),
        "0.2",
        &[("src/wide.una", &loop_state_source(n))],
    );
    let (checked, obligations) = generated(&root);
    assert_eq!(checked.diagnostics, [], "{n}");
    let text = std::fs::read_to_string(root.join("verification.lock")).unwrap();
    let (lock, _) = una_project::parse_lock(&text);
    let lock = una_verify::Lock {
        text,
        solver_version: lock.solver_version,
        step_budget: lock.step_budget,
        random_seed: lock.random_seed,
    };
    let bytes = una_verify::queries(&obligations, &lock)
        .iter()
        .map(String::len)
        .sum();
    let _ = std::fs::remove_dir_all(&root);
    bytes
}

#[test]
fn a_loop_state_of_n_components_gives_queries_linear_in_n() {
    // #333: each projection of a loop-state tuple was named after the whole tuple sort, so the
    // query text grew as n². With a short id per tuple sort it grows linearly: 8x the
    // components give about 8x the bytes, against about 50x before. The count is exact, so
    // the bound needs no timing margin.
    let small = loop_state_query_bytes(50);
    let large = loop_state_query_bytes(400);
    let growth = large as f64 / small as f64;
    eprintln!("8x the loop state: {growth:.1}x the query bytes ({small}, {large})");
    assert!(
        growth < 12.0,
        "8x the loop state gave {growth:.1}x the query bytes"
    );
}

/// A package of edition 0.4 under the test's temporary directory, with `source` as its module
/// `facts`: described by `ita.una` and locked by the lock of a conformance case of edition 0.4
/// (ADR 0122).
fn facts_package_0_4(name: &str, source: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("verification")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("ita.una"),
        format!("ita {name}\n  version 0.1.0\n  edition 0.4\n"),
    )
    .unwrap();
    std::fs::copy(
        repository().join("tests/conformance/verification/collection_facts/ita.lock"),
        root.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(root.join("src/facts.una"), source).unwrap();
    root
}

/// The queries of every obligation of the edition 0.4 package `name` whose module is
/// `source`, as text, generated but not solved.
fn queries_0_4(name: &str, source: &str) -> Vec<String> {
    let root = facts_package_0_4(name, source);
    let (checked, obligations) = generated(&root);
    assert!(!obligations.is_empty(), "{name}: {:?}", checked.diagnostics);
    let text = std::fs::read_to_string(repository().join("verification.lock")).unwrap();
    let (lock, _) = una_project::parse_lock(&text);
    let lock = una_verify::Lock {
        text,
        solver_version: lock.solver_version,
        step_budget: lock.step_budget,
        random_seed: lock.random_seed,
    };
    let queries = una_verify::queries(&obligations, &lock);
    let _ = std::fs::remove_dir_all(&root);
    queries
}

/// `n` updates of a hash set at one key, then a lookup of another: n frames, one key term
/// besides the updated key.
fn frames_source(n: usize) -> String {
    let mut out = String::from(
        "module facts\n\nimport std.collections (HashSet)\n\npub f(s: HashSet[Int], x: Int, y: Int) -> Bool\n  ensures result\n=\n  let edit t = s\n",
    );
    for _ in 0..n {
        out.push_str("  edit t.add(item: x)\n");
    }
    out.push_str("  t.has(item: y)\n");
    out
}

/// One update of a hash set, under `requires` clauses that look `n` keys up, one each: one
/// frame, n key terms besides the updated key.
fn keys_source(n: usize) -> String {
    let params: Vec<String> = (0..n).map(|i| format!("y{i}: Int")).collect();
    let lookups: String = (0..n)
        .map(|i| format!("  requires s.has(item: y{i})\n"))
        .collect();
    format!(
        "module facts\n\nimport std.collections (HashSet)\n\npub f(s: HashSet[Int], x: Int, {}) -> Bool\n{lookups}  ensures result\n=\n  let edit t = s\n  edit t.remove(item: x)\n  t.len >= 0\n",
        params.join(", "),
    )
}

/// The bytes of `queries`, and the number of frame instances they hold (ADR 0119 V3).
fn frame_instances(queries: &[String]) -> (usize, usize) {
    let bytes = queries.iter().map(String::len).sum();
    let instances = queries
        .iter()
        .map(|query| query.matches("(=> (and (i!frame!").count())
        .sum();
    (bytes, instances)
}

#[test]
fn collection_frames_grow_linearly_in_updates_and_in_key_terms() {
    // ADR 0119 V8: a frame is instantiated once per key term that the query looks up, so a
    // query with k frames and t key terms gains O(k · t) facts. With t fixed, 8x the updates
    // give about 8x the bytes; with k fixed, 8x the key terms give about 8x the bytes. A frame
    // instantiated at every `Int` term, the lengths included, would grow as n² with the
    // updates. The counts are exact, so the bound needs no timing margin.
    for (what, source) in [
        ("updates", frames_source as fn(usize) -> String),
        ("key terms", keys_source),
    ] {
        let name = if what == "updates" { "frames" } else { "keys" };
        let (small, small_instances) =
            frame_instances(&queries_0_4(&format!("{name}_50"), &source(50)));
        let (large, large_instances) =
            frame_instances(&queries_0_4(&format!("{name}_400"), &source(400)));
        assert!(
            small_instances >= 50 && large_instances >= 400,
            "{what}: {small_instances} and {large_instances} frame instances"
        );
        let growth = large as f64 / small as f64;
        eprintln!("8x the {what}: {growth:.1}x the query bytes ({small}, {large})");
        assert!(
            growth < 12.0,
            "8x the {what} gave {growth:.1}x the query bytes"
        );
    }
}

/// Every conformance case and example of the repository: package directories and single files.
fn every_program() -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![
        repository().join("tests/conformance"),
        repository().join("examples"),
    ];
    while let Some(dir) = pending.pop() {
        if dir.join("una.toml").is_file() {
            found.push(dir);
            continue;
        }
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() && !path.ends_with("target") {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "una") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[test]
fn no_two_tuple_sorts_share_a_name() {
    // #333: a tuple sort is named by a 128-bit digest of its components. Across every query of
    // every program of the repository that checks, no query declares a symbol twice, and every
    // projection of a tuple sort has the same component sort in every query, which two
    // distinct tuple sorts sharing a name would break.
    let mut projections: BTreeMap<String, String> = BTreeMap::new();
    let mut tuple_sorts = 0;
    for program in every_program() {
        let (checked, obligations) = generated(&program);
        if !checked.diagnostics.is_empty() || obligations.is_empty() {
            continue;
        }
        let text = std::fs::read_to_string(repository().join("verification.lock")).unwrap();
        let (lock, _) = una_project::parse_lock(&text);
        let lock = una_verify::Lock {
            text,
            solver_version: lock.solver_version,
            step_budget: lock.step_budget,
            random_seed: lock.random_seed,
        };
        for query in una_verify::queries(&obligations, &lock) {
            let mut declared = BTreeSet::new();
            for line in query.lines() {
                let Some(rest) = line
                    .strip_prefix("(declare-sort ")
                    .or_else(|| line.strip_prefix("(declare-fun "))
                else {
                    continue;
                };
                let name = rest.split(' ').next().unwrap().to_owned();
                assert!(
                    declared.insert(name.clone()),
                    "{}: {name} twice",
                    program.display()
                );
                if name.starts_with("i!t!") {
                    let result = rest.rsplit(' ').next().unwrap().trim_end_matches(')');
                    let known = projections.entry(name.clone()).or_insert_with(|| {
                        tuple_sorts += 1;
                        result.to_owned()
                    });
                    assert_eq!(known, result, "{}: {name}", program.display());
                }
            }
        }
    }
    assert!(
        tuple_sorts > 0,
        "no query of the repository projects a tuple"
    );
}

#[test]
fn a_kind_core_does_not_declare_stops_tier_1() {
    // #407: generating this package's obligations looks `Holder` up by its declaration, for
    // its constructor and the function value in its field (ADR 0037 rule 2). When Core names a
    // kind that no declaration has, generation stops with an internal error, rather than go on
    // with the facts or checks that the lookup would have given.
    let root = package_of_edition(
        "unresolved_kind",
        "0.2",
        &[(
            "src/app.una",
            "module app\n\npub kind Pos = Int where it > 0\n\npub kind Holder[T]\n  f: T\n\nsame(x: Int) -> Int = x\n\npub make() -> Holder[fn(Int) -> Pos] = Holder(f: same)\n",
        )],
    );
    let (mut checked, obligations) = generated(&root);
    assert!(
        obligations.iter().any(|o| o.goal.contains("result > 0")),
        "the function value has its obligation: {:?}",
        obligations.iter().map(|o| &o.goal).collect::<Vec<_>>()
    );
    for module in &mut checked.program.modules {
        for decl in &mut module.decls {
            if let una_core::Decl::Data { name, .. } = decl
                && name == "Holder"
            {
                *name = "Renamed".to_owned();
            }
        }
    }
    let regenerated = std::thread::scope(|scope| {
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
    });
    assert_eq!(
        regenerated.map(|o| o.len()),
        Err(una_verify::Problem::UnresolvedKind {
            module: "app".to_owned(),
            name: "Holder".to_owned(),
        })
    );
}

#[test]
fn every_family_of_the_prelude_verifies_without_an_internal_error() {
    // #407: the verifier stops on a kind it cannot resolve, so a program that inspects each
    // family of the prelude's kinds under a contract verifies cleanly at tier 1.
    let families: [(&str, &str); 7] = [
        (
            "map",
            "module family\n\npub frequent(counts: Map[Text, Int]) -> Int\n  ensures result >= 0\n=\n  let kept = counts.entries().filter(keep: fn(e) -> e.value >= 1)\n  kept.len\n",
        ),
        (
            "json",
            "module family\n\npub port(text: Text) -> Int\n  ensures result >= 0\n=\n  let r = attempt text.parse_json()\n  when r\n    is Ok(doc) ->\n      when doc\n        is Object(fields) -> 1\n        is Number(n) -> 2\n        else -> 0\n    is Err(e) ->\n      when e\n        is Syntax(at) -> 3\n        else -> 4\n",
        ),
        (
            "text",
            "module family\n\npub leading(field: Text) -> Int\n  ensures result >= 0\n=\n  let r = attempt field.parse_int()\n  when r\n    is Ok(n) -> if n >= 0 then n else 0\n    is Err(e) ->\n      when e\n        is NotADigit(at) -> 1\n        else -> 0\n",
        ),
        (
            "http",
            "module family\n\npub status(city: Text) -> internal Int\n  ensures result >= 0\n  uses @http: Http[get]\n=\n  let accept = Header(name: \"Accept\", value: \"application/json\")\n  let r = attempt @http.get(\"https://api.example.com/v1/now?city={city}\", headers: [accept], key_name: None)\n  when r\n    is Ok(response) -> if response.status >= 0 then response.status else 0\n    is Err(e) ->\n      when e\n        is Denied -> 1\n        else -> 0\n",
        ),
        (
            "files",
            "module family\n\npub size(path: Text) -> internal Int\n  ensures result >= 0\n  uses @files: Files[read]\n=\n  let r = attempt @files.read(path)\n  when r\n    is Ok(text) -> text.len\n    is Err(e) ->\n      when e\n        is NotFound -> 1\n        else -> 0\n",
        ),
        (
            "keys",
            "module family\n\npub signed(name: Text) -> internal Int\n  ensures result >= 0\n  uses @keys: Keys[hmac]\n=\n  let r = attempt @keys.hmac(key_name: \"pepper\", data: name)\n  when r\n    is Ok(h) -> 1\n    is Err(e) ->\n      when e\n        is UnknownKey -> 2\n",
        ),
        (
            "option",
            "module family\n\npub first(o: Option[Int]) -> Int\n  ensures result >= 0\n=\n  when o\n    is Some(x) -> if x >= 0 then x else 0\n    is None -> 0\n",
        ),
    ];
    for (family, source) in families {
        let root = package_of_edition(
            &format!("prelude_family_{family}"),
            "0.2",
            &[("src/family.una", source)],
        );
        let checked = una_cli::check(&root, Tier::One).unwrap();
        let found: Vec<String> = checked
            .diagnostics
            .iter()
            .map(|d| format!("{} {}", d.code, d.message))
            .collect();
        assert_eq!(found, Vec::<String>::new(), "{family}");
    }
}

#[test]
fn a_contract_across_a_loop_names_the_map_and_the_helper_pattern() {
    // #404 (dogfood D-3): the reason names a map as a map, with no length and no citation of
    // the specification, and points at the pattern that proves such a contract.
    let root = package_of_edition(
        "loop_contract_map",
        "0.2",
        &[(
            "src/t.una",
            "module t\n\npub tally(votes: List[Text]) -> Map[Text, Int]\n  ensures result.len <= votes.len\n=\n  let edit counts: Map[Text, Int] = empty_map()\n  for vote in votes\n    let n = counts.get(key: vote)\n      if none use 0\n    edit counts.set(key: vote, value: n + 1)\n  counts\n",
        )],
    );
    let checked = una_cli::check(&root, Tier::One).unwrap();
    let messages: Vec<String> = checked
        .diagnostics
        .iter()
        .map(|d| format!("{} {}", d.code, d.message))
        .collect();
    assert_eq!(
        messages,
        [
            "U5002 postcondition of `tally` could not be proven (the loop forgets every fact about the map `counts` but the refinements of its type, so the counterexample sets `counts` freely; state what one iteration does as the `ensures` of a helper with an `edit` parameter, and call the helper in the loop)"
        ]
    );
}

/// A package of edition `0.3` with `n` functions, each conditioning an operation of its binding
/// on a parameter and performing it under that condition, the next calling the previous one
/// (ADR 0092).
fn conditioned_source(n: usize) -> String {
    let mut out = String::from("module chain\n\npub capability Log\n  op write(n: Int) -> Unit\n");
    for i in 0..n {
        out.push_str(&format!(
            "\npub f{i}(n: Int, dry: Bool) -> Unit\n  uses @log: Log[write where not dry]\n=\n  if not dry\n"
        ));
        if i == 0 {
            out.push_str("    @log.write(n)\n");
        } else {
            out.push_str(&format!("    f{}(n, dry: dry)\n", i - 1));
        }
        out.push_str("  unit\n");
    }
    out
}

/// The `effect_condition` obligations of [`conditioned_source`] and the bytes of all queries.
fn conditioned_obligations(n: usize) -> (usize, usize) {
    let root = package_of_edition(
        &format!("chain-{n}"),
        "0.3",
        &[("src/chain.una", &conditioned_source(n))],
    );
    let (checked, obligations) = generated(&root);
    assert_eq!(checked.diagnostics, [], "{n}");
    let effects = obligations
        .iter()
        .filter(|o| o.kind == una_diag::ObligationKind::EffectCondition)
        .count();
    let text = std::fs::read_to_string(root.join("verification.lock")).unwrap();
    let (lock, _) = una_project::parse_lock(&text);
    let lock = una_verify::Lock {
        text,
        solver_version: lock.solver_version,
        step_budget: lock.step_budget,
        random_seed: lock.random_seed,
    };
    let bytes = una_verify::queries(&obligations, &lock)
        .iter()
        .map(String::len)
        .sum();
    let _ = std::fs::remove_dir_all(&root);
    (effects, bytes)
}

#[test]
fn conditions_give_one_obligation_per_site_and_queries_linear_in_the_sites() {
    // ADR 0092 rule 3: one query per site of a conditioned operation. Each function has one
    // site, its operation or its call of the previous function, so the count is exact; the
    // query text of each site does not grow with the others, so 8x the functions give about
    // 8x the bytes.
    let (small_effects, small) = conditioned_obligations(25);
    let (large_effects, large) = conditioned_obligations(200);
    assert_eq!((small_effects, large_effects), (25, 200));
    let growth = large as f64 / small as f64;
    eprintln!("8x the conditioned functions: {growth:.1}x the query bytes ({small}, {large})");
    assert!(
        growth < 12.0,
        "8x the conditioned functions gave {growth:.1}x the query bytes"
    );
}

/// `n` locked public functions, each with one conditioned operation, and their `[contracts]`
/// (ADR 0092 rule 8, as amended).
fn locked_package(n: usize) -> PathBuf {
    let mut source =
        String::from("module locks\n\npub capability Log\n  op write(n: Int) -> Unit\n");
    let mut contracts = String::from("\n[contracts]\n");
    for i in 0..n {
        source.push_str(&format!(
            "\npub f{i}(dry: Bool) -> Unit\n  uses @log: Log[write where not dry]\n=\n  if not dry\n    @log.write({i})\n  unit\n"
        ));
        contracts.push_str(&format!(
            "\"locks.f{i}\" = [\"@log: Log[write where not dry]\"]\n"
        ));
    }
    let root = package_of_edition(&format!("locks-{n}"), "0.3", &[("src/locks.una", &source)]);
    let manifest = root.join("una.toml");
    let mut text = std::fs::read_to_string(&manifest).unwrap();
    text.push_str(&contracts);
    std::fs::write(&manifest, text).unwrap();
    root
}

/// The `contract_lock` obligations of [`locked_package`] and the bytes of their queries.
fn lock_obligations(n: usize) -> (usize, usize) {
    let root = locked_package(n);
    let (checked, obligations) = generated(&root);
    assert_eq!(checked.diagnostics, [], "{n}");
    let locks: Vec<&una_verify::Obligation> = obligations
        .iter()
        .filter(|o| o.kind == una_diag::ObligationKind::ContractLock)
        .collect();
    let text = std::fs::read_to_string(root.join("verification.lock")).unwrap();
    let (lock, _) = una_project::parse_lock(&text);
    let lock = una_verify::Lock {
        text,
        solver_version: lock.solver_version,
        step_budget: lock.step_budget,
        random_seed: lock.random_seed,
    };
    let owned: Vec<una_verify::Obligation> = locks.into_iter().cloned().collect();
    let bytes = una_verify::queries(&owned, &lock)
        .iter()
        .map(String::len)
        .sum();
    let _ = std::fs::remove_dir_all(&root);
    (owned.len(), bytes)
}

#[test]
fn locks_give_one_obligation_per_locked_condition_and_grow_linearly() {
    // ADR 0092 rule 8, as amended: one pass over the locks and the locked headers, and one
    // `contract_lock` query per locked condition that the header grants. 8x the locked
    // functions give exactly 8x the obligations and about 8x the query bytes.
    let (small_locks, small) = lock_obligations(25);
    let (large_locks, large) = lock_obligations(200);
    assert_eq!((small_locks, large_locks), (25, 200));
    let growth = large as f64 / small as f64;
    eprintln!("8x the locked functions: {growth:.1}x the query bytes ({small}, {large})");
    assert!(
        growth < 12.0,
        "8x the locked functions gave {growth:.1}x the query bytes"
    );
}

/// Every `effect_condition` site of the conformance cases of ADR 0092, by case: the file, line
/// and column of the site, and the conditioned operation. Proven sites are listed with refuted
/// ones: the list is what the verifier must generate, no more and no less.
const EFFECT_SITES: &[(&str, &[&str])] = &[
    (
        "authority/op_condition_accepted",
        &["src/pricing.una:15:5 @catalog.set_price"],
    ),
    (
        "verification/effect_condition_call",
        &[
            "src/calls.una:17:5 @log.write",
            "src/calls.una:23:3 @log.write",
            "src/calls.una:29:5 @log.write",
            "src/calls.una:35:3 @log.write",
            "src/calls.una:40:3 @log.write",
            "src/calls.una:45:3 @main.write",
        ],
    ),
    (
        "verification/effect_condition_operation",
        &[
            "src/ops.una:14:3 @log.write",
            "src/ops.una:20:5 @log.write",
            "src/ops.una:27:3 @log.write",
            "src/ops.una:32:25 @log.write",
            "src/ops.una:38:41 @log.write",
            "src/ops.una:46:3 @log.write",
            "src/ops.una:53:5 @log.write",
        ],
    ),
    (
        "verification/effect_condition_value",
        &[
            "src/values.una:14:3 @log.write",
            "src/values.una:20:5 @log.write",
            "src/values.una:31:3 @log.write",
            "src/values.una:36:3 @main.write",
        ],
    ),
    (
        "verification/effect_condition_with",
        &[
            "src/installs.una:21:5 @log.write",
            "src/installs.una:26:15 @log.write",
            "src/installs.una:30:15 @log.write",
            "src/installs.una:40:25 @log.write",
            "src/installs.una:45:25 @log.write",
        ],
    ),
    (
        "verification/effect_condition_uses_of",
        &[
            "src/main.una:15:3 @log.write",
            "src/main.una:22:5 @log.write",
            "src/main.una:29:12 @log.write",
            "src/main.una:35:12 @log.write",
            "src/main.una:44:36 @log.write",
            "src/main.una:63:5 @log.write",
            "src/main.una:63:5 @log.write",
        ],
    ),
    (
        "verification/effect_condition_policy",
        &[
            "src/app/main.una:11:21 @keys.hmac",
            "src/app/main.una:22:23 @keys.hmac",
        ],
    ),
];

#[test]
fn effect_condition_sites_are_exactly_the_pinned_ones() {
    // This pins the verifier's list with each site's operation, so a site that goes missing
    // from both tiers at once fails here; the corpus cross-check compares the checker's sites
    // with these (#615).
    for (case, expected) in EFFECT_SITES {
        let (checked, obligations) = generated(&repository().join("tests/conformance").join(case));
        assert_eq!(checked.diagnostics, [], "{case}");
        let mut found: Vec<String> = obligations
            .iter()
            .filter(|o| o.kind == una_diag::ObligationKind::EffectCondition)
            .map(|o| {
                let location = checked.sources.location(o.source);
                let operation = o
                    .site
                    .effect
                    .as_ref()
                    .map_or("?", |effect| effect.operation.as_str());
                format!(
                    "{}:{}:{} {operation}",
                    location.file, location.start.line, location.start.col
                )
            })
            .collect();
        found.sort();
        let mut expected: Vec<String> = expected.iter().map(|s| (*s).to_owned()).collect();
        expected.sort();
        assert_eq!(found, expected, "{case}");
    }
}

/// A package whose `update` is locked by `lock`, a TOML string as `una.toml` writes it: the
/// header leaves `set_price` without the lock's condition `cents > 0`.
fn escaped_lock_package(name: &str, lock: &str) -> PathBuf {
    let source = "module shop\n\npub capability Catalog\n  op set_price(sku: Int, cents: Int) -> Unit\n  op set_name(sku: Int, name: Text) -> Unit\n  op set_rate(sku: Int, rate: Int) -> Unit\n\nvalid(sku: Int) -> Bool = sku > 0\n\npub update(sku: Int, cents: Int, name: Text) -> Unit\n  uses @catalog: Catalog[set_price, set_name where name == \"a\", set_rate]\n=\n  @catalog.set_price(sku, cents)\n  if name == \"a\"\n    @catalog.set_name(sku, name)\n  unit\n";
    let root = package_of_edition(name, "0.3", &[("src/shop.una", source)]);
    let manifest = root.join("una.toml");
    let mut text = std::fs::read_to_string(&manifest).unwrap();
    text.push_str(&format!("\n[contracts]\n\"shop.update\" = [{lock}]\n"));
    std::fs::write(&manifest, text).unwrap();
    root
}

#[test]
fn a_lock_string_with_escapes_reads_as_its_value() {
    // #727: the conditions of a lock whose string has escapes, or is a multi-line string, are
    // quoted as the string reads, in the diagnostics and in the goals, and their obligations
    // have the ids of the same lock written without escapes. Before, every position after an
    // escape was placed too early in the file, so a condition was quoted from the wrong bytes.
    let forms = [
        (
            "escaped",
            r#""@catalog: Catalog[set_name where name == \"a\", set_price where cents\u0020> 0, set_rate]""#,
        ),
        (
            "literal",
            r#"'@catalog: Catalog[set_name where name == "a", set_price where cents > 0, set_rate]'"#,
        ),
        (
            "multi-line",
            "\"\"\"\n@catalog: Catalog[set_name where name == \"a\", \\\n    set_price where cents > 0, set_rate]\"\"\"",
        ),
    ];
    let mut seen = Vec::new();
    for (form, lock) in forms {
        let root = escaped_lock_package(&format!("escaped-lock-{form}"), lock);
        let checked = una_cli::check(&root, Tier::One).unwrap();
        let messages: Vec<(&str, &str)> = checked
            .diagnostics
            .iter()
            .map(|d| (d.code.as_str(), d.message.as_str()))
            .collect();
        assert_eq!(
            messages,
            [(
                "U3023",
                "the header of `update` lets `@catalog.set_price` run where `cents > 0` does not hold, which `una.toml` locks"
            )],
            "{form}"
        );
        let (_, obligations) = generated(&root);
        let mut locks: Vec<(String, String)> = obligations
            .iter()
            .filter(|o| o.kind == una_diag::ObligationKind::ContractLock)
            .map(|o| (o.goal.clone(), o.id.clone()))
            .collect();
        locks.sort();
        let goals: Vec<&str> = locks.iter().map(|(goal, _)| goal.as_str()).collect();
        assert_eq!(goals, ["cents > 0", "name == \"a\""], "{form}");
        seen.push(locks);
        let _ = std::fs::remove_dir_all(&root);
    }
    assert!(seen.windows(2).all(|pair| pair[0] == pair[1]), "{seen:?}");
    // A condition after an escape that the checker rejects is quoted as written.
    let root = escaped_lock_package(
        "escaped-lock-rejected",
        r#""@catalog: Catalog[set_name where name == \"a\", set_rate where valid(sku)]""#,
    );
    let checked = una_cli::check(&root, Tier::Zero).unwrap();
    let messages: Vec<&str> = checked
        .diagnostics
        .iter()
        .filter(|d| d.code.as_str() == "U7002")
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        messages,
        [
            "the lock of `update` conditions `set_rate` by `valid(sku)`, which is not a `Bool` of the refinement language over its parameters: the call of function `valid` is outside the refinement language; the refinement language calls only predicates"
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn units_are_erased_before_the_verifier() {
    // ADR 0104 rule 5: units are erased before obligations are generated, so the query texts
    // of a program with units are byte-identical to those of its erasure `E`, in which every
    // `measure` and `of` is deleted, `magnitude()` and `measured()` are their receiver, and
    // `convert()` by `k` is `* k`.
    let measured = "module m\n\npub measure Meter\n\npub measure Second\n\npub measure Kilometer = 1000 * Meter\n\npub kind Speed = Int of Meter / Second in 0..300\n\npub speed(d: Int of Meter in 0..3000, t: Int of Second in 10..100) -> Speed = d / t\n\npub up(k: I32 of Kilometer) -> I32 of Meter = k.convert()\n\npub raw(d: Int of Meter, n: Int) -> Int\n  requires d.magnitude() > n\n=\n  d.magnitude() - n\n\npub tagged(n: I32) -> I32 of Meter = (n + 1).measured()\n";
    let erased = "module m\n\npub kind Speed = Int in 0..300\n\npub speed(d: Int in 0..3000, t: Int in 10..100) -> Speed = d / t\n\npub up(k: I32) -> I32 = k * 1000\n\npub raw(d: Int, n: Int) -> Int\n  requires d > n\n=\n  d - n\n\npub tagged(n: I32) -> I32 = (n + 1)\n";
    let lock_text = std::fs::read_to_string(repository().join("verification.lock")).unwrap();
    let (parsed, _) = una_project::parse_lock(&lock_text);
    let lock = una_verify::Lock {
        text: lock_text,
        solver_version: parsed.solver_version,
        step_budget: parsed.step_budget,
        random_seed: parsed.random_seed,
    };
    let mut scripts = Vec::new();
    for (name, source) in [("units-measured", measured), ("units-erased", erased)] {
        let root = package_of_edition(name, "0.3", &[("src/m.una", source)]);
        let (checked, obligations) = generated(&root);
        assert_eq!(checked.diagnostics, [], "{name}");
        assert!(!obligations.is_empty(), "{name}");
        scripts.push(una_verify::queries(&obligations, &lock));
        let _ = std::fs::remove_dir_all(&root);
    }
    assert_eq!(scripts[0], scripts[1]);
    // ADR 0104, note of 2026-10-15: `una core` exports the erased program, so the two
    // documents are the same but for source locations and source texts, which differ
    // because `E` deletes text.
    fn strip(value: &mut Value) {
        // A location, `{file, start, end}`, wherever it is written.
        if let Value::Object(map) = value
            && map.contains_key("file")
            && map.contains_key("start")
            && map.contains_key("end")
        {
            *value = Value::Null;
            return;
        }
        match value {
            Value::Object(map) => {
                map.remove("span");
                map.remove("text");
                for (_, v) in map.iter_mut() {
                    strip(v);
                }
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut documents = Vec::new();
    for (dir, source) in [
        ("units-core-measured", measured),
        ("units-core-erased", erased),
    ] {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(dir)
            .join("units");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("una.toml"),
            "[package]\nname = \"units\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n",
        )
        .unwrap();
        std::fs::copy(
            repository().join("verification.lock"),
            root.join("verification.lock"),
        )
        .unwrap();
        std::fs::write(root.join("src/m.una"), source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_una"))
            .args(["--json", "core", root.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(output.status.success(), "{dir}: {}", stdout(&output));
        let mut document: Value = serde_json::from_str(stdout(&output)).unwrap();
        strip(&mut document);
        documents.push(document);
        let _ = std::fs::remove_dir_all(&root);
    }
    assert_eq!(documents[0], documents[1]);
}

/// The queries of `source`, a package of edition 0.3, by goal.
fn float_queries(name: &str, source: &str) -> Vec<(String, String)> {
    let root = package_of_edition(name, "0.3", &[("src/m.una", source)]);
    let (checked, obligations) = generated(&root);
    assert_eq!(checked.diagnostics, [], "{name}");
    let text = std::fs::read_to_string(root.join("verification.lock")).unwrap();
    let (lock, _) = una_project::parse_lock(&text);
    let lock = una_verify::Lock {
        text,
        solver_version: lock.solver_version,
        step_budget: lock.step_budget,
        random_seed: lock.random_seed,
    };
    let queries = una_verify::queries(&obligations, &lock);
    let _ = std::fs::remove_dir_all(&root);
    obligations
        .iter()
        .map(|o| o.goal.clone())
        .zip(queries)
        .collect()
}

/// A function of `lets` float lets and one list index, as ADR 0114 rule 16.7 states it.
fn float_lets(lets: usize) -> String {
    let mut body = String::from("  let f0 = k * 2.0\n");
    for i in 1..lets {
        let op = ["*", "+", "/", "-"][i % 4];
        body.push_str(&format!("  let f{i} = f{} {op} 1.5\n", i - 1));
    }
    format!(
        "module m\n\nget(xs: List[Int], i: Int) -> Int\n  requires 0 <= i and i < xs.len\n=\n  when at(xs, index: i)\n    is Some(v) -> v\n    is None -> 0\n\npub scaled(xs: List[Int], i: Int, k: F64) -> F64\n  requires 0 <= i and i + 1 < xs.len\n=\n{body}  let n = get(xs, i: i + 1)\n  f{} + n.to_f64()\n",
        lets - 1
    )
}

#[test]
fn integer_obligations_beside_float_lets_have_no_float_term() {
    // ADR 0114 rules 8.2, 8.3 and 16.7: the index obligation of a function with float lets
    // keeps no float hypothesis, so its query is in `QF_UFLIA` with no `F64` term, and its
    // text does not grow with the number of float lets: the cone is linear in Π, and the
    // integer query is the same at 5 and at 60 lets.
    let index = |lets: usize| {
        let queries = float_queries(&format!("float_lets_{lets}"), &float_lets(lets));
        let (_, query) = queries
            .iter()
            .find(|(goal, _)| goal.contains("(i + 1) < xs.len"))
            .cloned()
            .unwrap_or_else(|| panic!("no index obligation: {queries:?}"));
        // The temporary of the index is numbered after the lets.
        query.replace(&format!("%t{lets}|"), "%t|")
    };
    let small = index(5);
    let large = index(60);
    for query in [&small, &large] {
        assert!(query.contains("(set-logic QF_UFLIA)"), "{query}");
        assert!(
            !query.contains("Float64") && !query.contains("fp."),
            "{query}"
        );
    }
    assert_eq!(small, large);
}

#[test]
fn float_queries_are_in_logic_all_with_the_fixed_budget() {
    // ADR 0114 rules 8.2 and 8.3: a query with an `F64` term declares `ALL` and the fixed
    // step budget 5,000,000, whatever the lock sets; literals are their exact bits and the
    // orders are the defined functions `i!lt!F64` and `i!le!F64`.
    let root = package_of_edition(
        "float_budget",
        "0.3",
        &[(
            "src/m.una",
            "module m\n\npub half(x: F64) -> F64\n  requires x <= 1.0\n  ensures result <= 0.5\n=\n  x / 2.0\n",
        )],
    );
    let lock_path = root.join("verification.lock");
    let text = std::fs::read_to_string(&lock_path)
        .unwrap()
        .replace("step_budget      = 5000000", "step_budget      = 9000000");
    std::fs::write(&lock_path, &text).unwrap();
    let (checked, obligations) = generated(&root);
    assert_eq!(checked.diagnostics, []);
    let (lock, _) = una_project::parse_lock(&text);
    assert_eq!(lock.step_budget, 9_000_000);
    let lock = una_verify::Lock {
        text,
        solver_version: lock.solver_version,
        step_budget: lock.step_budget,
        random_seed: lock.random_seed,
    };
    let queries = una_verify::queries(&obligations, &lock);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(queries.len(), 1, "{queries:?}");
    let query = &queries[0];
    assert!(query.contains("(set-option :rlimit 5000000)"), "{query}");
    assert!(query.contains("(set-logic ALL)"), "{query}");
    // 1.0 and 0.5 by their bits, and `/` rounded to nearest, ties to even.
    assert!(
        query.contains(
            "(fp #b0 #b01111111111 #b0000000000000000000000000000000000000000000000000000)"
        ),
        "{query}"
    );
    assert!(
        query.contains(
            "(fp #b0 #b01111111110 #b0000000000000000000000000000000000000000000000000000)"
        ),
        "{query}"
    );
    assert!(query.contains("(fp.div RNE |u!v!x|"), "{query}");
    assert!(query.contains("(define-fun i!lt!F64 "), "{query}");
    assert!(query.contains("(define-fun i!le!F64 "), "{query}");
}

#[test]
fn the_cone_query_of_a_float_function_is_stable_across_runs() {
    // ADR 0114 rule 16.7, "Order": the cone query's text is stable, so its cache key is.
    let source = float_lets(12);
    let first = float_queries("float_stable_a", &source);
    let second = float_queries("float_stable_b", &source);
    assert_eq!(first, second);
}

/// A package of edition 0.4 under the test's temporary directory, described by `ita.una`, with
/// the lock of the conformance cases of that edition (ADR 0122).
fn package_0_4(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("verification")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("ita.una"),
        format!("ita {name}\n  version 0.1.0\n  edition 0.4\n"),
    )
    .unwrap();
    std::fs::copy(
        repository().join("tests/conformance/types/decimal_accepted/ita.lock"),
        root.join("ita.lock"),
    )
    .unwrap();
    for (path, text) in files {
        std::fs::write(root.join(path), text).unwrap();
    }
    root
}

#[test]
fn a_wrapped_line_keeps_the_ids_and_the_shown_text_of_its_obligations() {
    // `spec/12 §8`, ADR 0125 amendment of 2026-10-25: the canonical form of edition 0.4 wraps
    // the long call, and its obligations keep the ids, goals and hypotheses that the same call
    // on one line has in edition 0.3, whose canonical form keeps it on one line. Nothing
    // shown carries the wrapped layout.
    let declarations = "module app\n\npub kind Seats = Int in 0..5000\n\npub kind Booking\n  seats: Seats\n  label: Text\n\npub half(n: Int) -> Int\n  requires n >= 0\n=\n  n / 2\n\n";
    let call = "pub run() -> Int = half(Booking(seats: 20, label: \"a booking label long enough to carry this line past the width\").seats)\n";
    let wrapped = "pub run() -> Int = half(Booking(seats: 20,\n    label: \"a booking label long enough to carry this line past the width\"\n  ).seats)\n";
    let joined_root = package_of_edition(
        "wrapped_ids_0_3",
        "0.3",
        &[("src/app.una", &format!("{declarations}{call}"))],
    );
    let wrapped_root = package_0_4(
        "wrapped_ids_0_4",
        &[("src/app.una", &format!("{declarations}{wrapped}"))],
    );
    assert_eq!(layout_of(&joined_root), una_verify::SourceLayout::Joined);
    assert_eq!(layout_of(&wrapped_root), una_verify::SourceLayout::Wrapped);
    let (joined_checked, joined) = generated(&joined_root);
    let (wrapped_checked, wrapped_obligations) = generated(&wrapped_root);
    // Each source is canonical in its edition (no U7001).
    assert!(
        joined_checked.diagnostics.is_empty(),
        "{:?}",
        joined_checked.diagnostics
    );
    assert!(
        wrapped_checked.diagnostics.is_empty(),
        "{:?}",
        wrapped_checked.diagnostics
    );
    assert!(!wrapped_obligations.is_empty());
    let shown = |obligations: &[una_verify::Obligation]| -> Vec<(String, String, Vec<String>)> {
        obligations
            .iter()
            .map(|o| (o.id.clone(), o.goal.clone(), o.hypotheses.clone()))
            .collect()
    };
    assert_eq!(shown(&wrapped_obligations), shown(&joined));
    for (_, goal, hypotheses) in shown(&wrapped_obligations) {
        assert!(!goal.contains('\n'), "{goal:?}");
        assert!(
            hypotheses.iter().all(|text| !text.contains('\n')),
            "{hypotheses:?}"
        );
    }
    // The layout is what erases the wrapping: read as an earlier edition reads it, the goal
    // of the call's precondition would keep the space before the closer.
    let as_joined = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn_scoped(scope, || {
                una_verify::obligations(
                    &wrapped_checked.program,
                    &wrapped_checked.sources,
                    una_verify::SourceLayout::Joined,
                )
            })
            .unwrap()
            .join()
            .unwrap()
    })
    .unwrap();
    assert_ne!(shown(&as_joined), shown(&joined));
    std::fs::remove_dir_all(&joined_root).unwrap();
    std::fs::remove_dir_all(&wrapped_root).unwrap();
}
