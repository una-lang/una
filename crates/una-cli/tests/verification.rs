//! End-to-end tests of tier 1 (`spec/12`): `una check` with the solver, `una obligations`, the
//! proof cache, the queries' determinism and U5003. They need Z3 at the version of
//! `verification.lock` on the path (`compiler/testing.md §3`).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

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
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("verification")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n"),
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
    // generation order.
    let queries = || {
        let checked = una_cli::check(&repository().join("examples/events"), Tier::One).unwrap();
        assert_eq!(checked.diagnostics, []);
        let verification = checked.verification.unwrap();
        let text = std::fs::read_to_string(repository().join("examples/events/verification.lock"))
            .unwrap();
        let (lock, _) = una_project::parse_lock(&text);
        let lock = una_verify::Lock {
            text,
            solver_version: lock.solver_version,
            step_budget: lock.step_budget,
            random_seed: lock.random_seed,
        };
        let queries = una_verify::queries(&verification.obligations, &lock);
        let mut out = String::new();
        for (obligation, query) in verification.obligations.iter().zip(queries) {
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
    let first = queries();
    assert_eq!(first, queries());
    insta::assert_snapshot!("queries_events", first);
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
    let empty = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("no-solver-path");
    std::fs::create_dir_all(&empty).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .args(["--json", "check", root.to_str().unwrap()])
        .env("PATH", &empty)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let document: Value = serde_json::from_str(stdout(&output)).unwrap();
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
            .contains("install it with scripts/install-z3.sh"),
        "{}",
        stdout(&output)
    );
    // Tier 0 needs no solver.
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .args(["check", "--tier", "0", root.to_str().unwrap()])
        .env("PATH", &empty)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(Path::new(&root).exists());
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
        Expr::Attempt { body, .. } | Expr::With { body, .. } => collect_op_paths(body, out),
        Expr::Atom { .. } | Expr::Fail { .. } | Expr::Unreachable { .. } => {}
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
    std::fs::write(
        &solver,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'Z3 version 5.1.0 - 64 bit'; exit 0; fi\ncat > /dev/null\necho '(error \"line 1 column 1: rejected\")'\n",
    )
    .unwrap();
    std::fs::set_permissions(&solver, std::fs::Permissions::from_mode(0o755)).unwrap();
    let _ = std::fs::remove_dir_all(root.join(".una"));
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .args(["--json", "check", root.to_str().unwrap()])
        .env("PATH", &bin)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", stdout(&output));
    let document: Value = serde_json::from_str(stdout(&output)).unwrap();
    assert_eq!(document["diagnostics"][0]["code"], "U8001");
}

#[test]
fn the_verifier_generates_every_obligation_the_checker_found() {
    // `spec/12 §3` over Core IR agrees with the sites the type checker records since M4: the
    // site of each obligation, or each return path of a return obligation (DECISION D-113).
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
    for target in targets {
        let checked = una_cli::check(&target, Tier::One).unwrap();
        let Some(verification) = checked.verification else {
            continue;
        };
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
        for obligation in &verification.obligations {
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
        pending.dedup();
        generated.sort();
        generated.dedup();
        assert_eq!(generated, pending, "{}", target.display());
        compared += 1;
    }
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
fn a_forged_proof_cache_entry_is_trusted_and_counted_as_cached() {
    // R-018, ADR 0041, adversarial `escape-proof-cache-forgery`: the proof cache is part of
    // the Trusted zone in v0.1. A forged entry turns a refuted query into proven, as the
    // boundary states, and the report counts that result as one from the cache.
    let root = package(
        "escape-proof-cache-forgery",
        &[(
            "src/div0.una",
            "module div0\n\npub f(x: Int) -> Int = x / 0\n",
        )],
    );
    let path = root.to_str().unwrap();
    let output = una(&["--json", "report", path]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let report: Value = serde_json::from_str(stdout(&output)).unwrap();
    assert_eq!(
        report["obligations"],
        json!({"proven": 0, "refuted": 1, "unknown": 0, "cached": 0})
    );
    assert_eq!(report["tcb"]["proof_cache"], ".una/cache/");
    let output = una(&["check", path]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("U5001"), "{}", stderr(&output));
    let entries: Vec<PathBuf> = std::fs::read_dir(root.join(".una/cache/proofs"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(entries.len(), 1, "{entries:?}");
    std::fs::write(
        &entries[0],
        r#"{"status":"proven","reason":null,"values":[]}"#,
    )
    .unwrap();
    let output = una(&["check", path]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let output = una(&["--json", "report", path]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let report: Value = serde_json::from_str(stdout(&output)).unwrap();
    assert_eq!(
        report["obligations"],
        json!({"proven": 1, "refuted": 0, "unknown": 0, "cached": 1})
    );
    let output = una(&["obligations", path]);
    assert!(
        stdout(&output).trim_end().ends_with(" (cached)"),
        "{}",
        stdout(&output)
    );
    // A clean cache verifies everything again.
    std::fs::remove_dir_all(root.join(".una")).unwrap();
    let output = una(&["check", path]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
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
