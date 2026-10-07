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
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
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

/// The package at `path` checked at tier 0, and the obligations that tier 1 would generate for
/// it, on a stack as large as tier 1 gives them, but not solved: tests that compare obligations
/// or queries start no solver (#286).
fn generated(path: &Path) -> (una_cli::Checked, Vec<una_verify::Obligation>) {
    let checked = una_cli::check(path, Tier::Zero).unwrap();
    let obligations = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn_scoped(scope, || {
                una_verify::obligations(&checked.program, &checked.sources)
            })
            .unwrap()
            .join()
            .unwrap()
    });
    (checked, obligations)
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
    let empty = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("no-solver-path");
    std::fs::create_dir_all(&empty).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
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
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
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
        Expr::Attempt { body, .. }
        | Expr::With { body, .. }
        | Expr::LetTuple { body, .. }
        | Expr::Loop { body, .. } => collect_op_paths(body, out),
        Expr::Together { tasks, .. } => {
            for task in tasks {
                collect_op_paths(task, out);
            }
        }
        Expr::TogetherFor { task, .. } => collect_op_paths(task, out),
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
    // Written under another name and renamed into place, so that no process can execute the
    // file while it is still open for writing ("text file busy", #286).
    let staged = bin.join("z3.staged");
    std::fs::write(
        &staged,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'Z3 version 5.1.0 - 64 bit'; exit 0; fi\ncat > /dev/null\necho '(error \"line 1 column 1: rejected\")'\n",
    )
    .unwrap();
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&staged, &solver).unwrap();
    let _ = std::fs::remove_dir_all(root.join(".una"));
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
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
fn a_closed_goal_without_bindings_has_no_counterexample_n009() {
    // N-009, `spec/13 §3` ("when applicable"): `zero(1) > 0` has no free binder but is not a
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
    // The first run solves and caches the refutation under the key it makes.
    assert!(refuted(&run("alice", None, &["check"])));
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
    // The override names the key file, as the report shows it.
    let named = scratch.join("team.key");
    std::fs::write(&named, [7u8; 32]).unwrap();
    let shown = json!(named.to_str().unwrap());
    assert_eq!(report("carol", Some(&named)).1, shown);
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
            "field `pin` of `Account` is `secret`: interpolate the fields that `@console.print` accepts instead, or declassify `pin` through a policy"
        ]
    );
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
