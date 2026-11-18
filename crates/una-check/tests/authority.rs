//! Capabilities, effects and failures (M5): capability contexts and sufficiency, `with` and
//! handlers, the inference of private headers, failures and their handling, one effect per
//! line, the second-class rules, examples, `std.testing`, grants and the escape budget
//! (`spec/04 §4, §5, §8`, `spec/05`, `spec/06`, `spec/08 §2`, `spec/10 §5, §6, §9`,
//! `spec/14 §3, §4`, ADR 0028).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use pretty_assertions::assert_eq;
use una_check::{ModuleInput, PackageInput, TypeCheck, check_names, check_types};
use una_diag::{Code, Diagnostic, Widening};
use una_project::{Manifest, parse_manifest};
use una_syntax::{Interner, SourceDb, lex, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n";

/// Runs the name and type checks over `files`.
fn run(files: &[(&str, &str)], single: bool) -> (Vec<Diagnostic>, TypeCheck) {
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let mut manifest = None;
    let mut settings = Manifest::default();
    let mut modules = Vec::new();
    for (name, text) in files {
        let id = db.add(*name, text.as_bytes());
        if *name == "una.toml" {
            manifest = Some(id);
            settings = parse_manifest(text).0;
            continue;
        }
        let lexed = lex(db.file(id), id, &mut interner);
        let parsed = parse(&lexed, id, &interner);
        let broken = !lexed.errors.is_empty() || !parsed.errors.is_empty();
        assert!(!broken, "{name} has syntax errors: {:?}", parsed.errors);
        modules.push((id, parsed.file, broken));
    }
    let input = PackageInput {
        sources: &db,
        interner: &interner,
        single,
        modules: modules
            .iter()
            .map(|(id, tree, broken)| ModuleInput {
                id: *id,
                tree,
                broken: *broken,
            })
            .collect(),
        manifest,
        settings: &settings,
        run: None,
        earlier_errors: false,
        locks: &[],
    };
    let names = check_names(&input);
    let types = check_types(&input);
    (names, types)
}

/// Every diagnostic of `files`, as "code file:line:col message" in output order.
fn check(files: &[(&str, &str)], single: bool) -> Vec<String> {
    let (names, types) = run(files, single);
    let mut diagnostics: Vec<Diagnostic> = names.into_iter().chain(types.diagnostics).collect();
    diagnostics.sort_by(|a, b| {
        (&a.primary.file, a.primary.start, a.code).cmp(&(&b.primary.file, b.primary.start, b.code))
    });
    diagnostics
        .iter()
        .map(|d| {
            format!(
                "{} {}:{}:{} {}",
                d.code, d.primary.file, d.primary.start.line, d.primary.start.col, d.message
            )
        })
        .collect()
}

/// A single file, whose declarations start on line 3.
fn single(body: &str) -> Vec<String> {
    check(&[("t.una", &format!("module t\n\n{body}"))], true)
}

/// A package with `una.toml` followed by `manifest`, and the given modules.
fn package(manifest: &str, modules: &[(&str, &str)]) -> Vec<String> {
    let manifest = format!("{MANIFEST}{manifest}");
    let mut files = vec![("una.toml", manifest.as_str())];
    files.extend_from_slice(modules);
    check(&files, false)
}

/// Capabilities shared by the cases below, on lines 3 to 14.
const CAPS: &str = "kind StoreError = Timeout | Corrupted\n\
                    \n\
                    kind LookupError = Missing | Unavailable\n\
                    \n\
                    capability Log\n  op write(line: Text) -> Unit\n  op flush() -> Unit\n\
                    \n\
                    capability Store\n  op find(key: Int) -> Option[Int]\n    fails StoreError\n\
                    \n";

/// A single file that starts with [`CAPS`]; `body` starts on line 15.
fn with_caps(body: &str) -> Vec<String> {
    single(&format!("{CAPS}{body}"))
}

// ----- using capabilities (U3001, U3002, U3004; `spec/05 §3`, `spec/04 §2` rule 4) -----

#[test]
fn operations_need_a_binding_that_permits_them() {
    assert_eq!(
        with_caps(
            "pub a() -> Unit\n  uses @log: Log[write]\n=\n  @log.write(\"x\")\n\
             \n\
             pub b() -> Unit\n  uses @log: Log\n=\n  @log.flush()\n\
             \n\
             pub c() -> Unit\n  uses @log: Log[write]\n=\n  @log.flush()\n\
             \n\
             pub d() -> Unit\n  uses @log: Log[write]\n=\n  @other.write(\"x\")\n\
             \n\
             pub e() -> Unit = @log.write(\"x\")\n"
        ),
        [
            "U3002 t.una:28:8 `@log` permits `write`, not `flush`",
            "U3001 t.una:33:3 `@other` is not in scope",
            "U3004 t.una:35:19 `@log` is used here, but this code declares no `uses`, so it is pure",
        ]
    );
}

// ----- passing capabilities (U3001 to U3003; `spec/05 §4`, `spec/10 §2` Suff) -----

#[test]
fn calls_need_sufficient_capabilities() {
    assert_eq!(
        with_caps(
            "pub write(line: Text) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(line)\n\
             \n\
             pub attenuated() -> Unit\n  uses @log: Log\n=\n  write(\"x\")\n\
             \n\
             pub fewer() -> Unit\n  uses @log: Log[flush]\n=\n  write(\"x\")\n\
             \n\
             pub absent() -> Unit\n  uses @other: Log\n=\n  write(\"x\")\n\
             \n\
             pub another() -> Unit\n  uses @log: Store\n=\n  write(\"x\")\n\
             \n\
             pub pure() -> Unit = write(\"x\")\n"
        ),
        [
            "U3002 t.una:28:3 `write` needs `@log` to permit `write`, which it does not here",
            "U3001 t.una:33:3 `write` needs `@log: Log[write]`, which is not in scope here",
            "U3001 t.una:38:3 `write` needs `@log` to be `Log`, and here it is `Store`",
            "U3003 t.una:40:22 `write` uses `@log: Log[write]`, but this code declares no `uses`, so its effects would exceed its header",
        ]
    );
}

// ----- `with` (U3001, U3002, U3010; `spec/05 §4`, `spec/10 §6`) -----

#[test]
fn with_rebinds_names_and_installs_handlers() {
    assert_eq!(
        with_caps(
            "pub write(line: Text) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(line)\n\
             \n\
             handler quiet() for Log\n  op write(line) = unit\n  op flush() = unit\n\
             \n\
             handler cached(rows: List[Int]) for Store\n  op find(key) = rows.first()\n\
             \n\
             handler forwarding() for Log\n  uses @out: Log[write]\n  op write(line) = @out.write(line)\n  op flush() = unit\n\
             \n\
             pub rebound() -> Unit\n  uses @primary: Log[write]\n=\n  write(\"x\") with @log = @primary\n\
             \n\
             pub installed() -> Unit\n=\n  write(\"x\") with @log = quiet()\n\
             \n\
             pub nowhere() -> Unit\n  uses @log: Log[write]\n=\n  write(\"x\") with @log = @missing\n\
             \n\
             pub wrong_handler() -> Unit\n=\n  write(\"x\") with @log = cached(rows: [])\n\
             \n\
             pub not_a_handler() -> Unit\n=\n  write(\"x\") with @log = write(\"y\")\n\
             \n\
             pub handler_needs() -> Unit\n=\n  write(\"x\") with @log = forwarding()\n\
             \n\
             pub handler_served() -> Unit\n  uses @out: Log[write]\n=\n  write(\"x\") with @log = forwarding()\n\
             \n\
             pub one_statement() -> Unit\n  uses @primary: Log[write]\n=\n  write(\"x\") with @log = @primary\n  write(\"y\")\n"
        ),
        [
            "U3001 t.una:44:26 `@missing` is not in scope",
            "U3010 t.una:48:26 this handler implements `Store`, and `write` needs `@log` to be `Log`",
            "U3010 t.una:52:26 `write` is not a handler; `with` and `given` install handlers",
            "U3003 t.una:56:26 handler `forwarding` uses `@out: Log[write]`, but this code declares no `uses`, so its effects would exceed its header",
            "U3001 t.una:67:3 `write` needs `@log: Log[write]`, which is not in scope here",
        ]
    );
}

#[test]
fn a_capability_reference_is_bound_once_per_construct() {
    assert_eq!(
        with_caps(
            "handler quiet() for Log\n  op write(line) = unit\n  op flush() = unit\n\
             \n\
             pub note(line: Text) -> Unit\n  uses @log: Log[write]\n  uses @log: Log[flush]\n=\n  @log.write(line)\n\
             \n\
             pub rebinds() -> Unit\n  uses @x: Log[write]\n=\n  note(\"a\") with @log = @x, @log = @x\n\
             \n\
             example \"given twice\"\n  given @log = quiet()\n  given @log = quiet()\n  do note(\"x\")\n  expect true\n"
        ),
        [
            "U1002 t.una:21:8 capability reference `@log` is bound twice",
            "U1002 t.una:28:29 capability reference `@log` is bound twice",
            "U1002 t.una:32:9 capability reference `@log` is bound twice",
        ]
    );
}

// ----- inference of private headers (U3001, U3008, U3009, U3010; `spec/04 §5`, ADR 0007) -----

#[test]
fn private_functions_infer_uses_from_their_callers() {
    assert_eq!(
        with_caps(
            "helper(line: Text) -> Unit = @log.write(line)\n\
             \n\
             twice(line: Text) -> Unit\n=\n  helper(line)\n  @log.flush()\n\
             \n\
             pub direct() -> Unit\n  uses @log: Log\n=\n  twice(\"x\")\n\
             \n\
             pub rebound() -> Unit\n  uses @main: Log\n=\n  twice(\"x\") with @log = @main\n\
             \n\
             pub narrow() -> Unit\n  uses @log: Log[write]\n=\n  twice(\"x\")\n\
             \n\
             pub pure() -> Unit = helper(\"x\")\n\
             \n\
             orphan() -> Unit = @nobody.write(\"x\")\n\
             \n\
             split(line: Text) -> Unit = @sink.write(line)\n\
             \n\
             pub to_log() -> Unit\n  uses @sink: Log\n=\n  split(\"a\")\n\
             \n\
             pub to_store() -> Unit\n  uses @sink: Store\n=\n  split(\"b\")\n"
        ),
        [
            "U3002 t.una:35:3 `twice` needs `@log` to permit `flush`, which it does not here",
            "U3003 t.una:37:22 `helper` uses `@log: Log[write]`, but this code declares no `uses`, so its effects would exceed its header",
            "U3001 t.una:39:20 no caller of `orphan` binds `@nobody`, so its capability cannot be inferred",
            "U3001 t.una:41:29 the callers of `split` bind `@sink` to `Log` and to `Store`",
        ]
    );
}

#[test]
fn installed_handlers_decide_only_what_bindings_leave_open() {
    assert_eq!(
        with_caps(
            "helper(line: Text) -> Unit = @log.write(line)\n\
             \n\
             handler quiet() for Log\n  op write(line) = unit\n  op flush() = unit\n\
             \n\
             handler cached() for Store\n  op find(key) = None\n\
             \n\
             pub bound() -> Unit\n  uses @log: Log[write]\n=\n  helper(\"x\")\n\
             \n\
             pub wrong() -> Unit\n=\n  helper(\"x\") with @log = cached()\n\
             \n\
             only_handled(line: Text) -> Unit = @out.write(line)\n\
             \n\
             pub installs() -> Unit\n=\n  only_handled(\"x\") with @out = quiet()\n"
        ),
        [
            "U3010 t.una:31:27 this handler implements `Store`, and `helper` needs `@log` to be `Log`",
        ]
    );
}

#[test]
fn declared_requirements_decide_before_callers() {
    assert_eq!(
        single(
            "capability Log\n  op write(line: Text) -> Unit\n\
             \n\
             capability Audit\n  op record(line: Text) -> Unit\n\
             \n\
             pub write(line: Text) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(line)\n\
             \n\
             pub record(line: Text) -> Unit\n  uses @log: Audit[record]\n=\n  @log.record(line)\n\
             \n\
             wraps(line: Text) -> Unit = write(line)\n\
             \n\
             wraps_again(line: Text) -> Unit = wraps(line)\n\
             \n\
             calls_both(line: Text) -> Unit\n=\n  write(line)\n  record(line)\n\
             \n\
             pub binds_other() -> Unit\n  uses @log: Audit\n=\n  wraps(\"x\")\n\
             \n\
             unknown(line: Text) -> Unit\n=\n  @log.shout(line)\n  wraps(line)\n"
        ),
        [
            "U3001 t.una:25:3 `calls_both` calls code that needs `@log` to be `Log` and code that needs it to be `Audit`",
            "U3001 t.una:31:3 `wraps` needs `@log` to be `Log`, and here it is `Audit`",
            "U1001 t.una:35:8 capability `Log` has no operation `shout`",
        ]
    );
}

#[test]
fn private_functions_infer_one_error_kind() {
    assert_eq!(
        with_caps(
            "raises(x: Int) -> Int = if x > 0 then x else fail Missing\n\
             \n\
             passes(x: Int) -> Int\n=\n  raises(x)\n    if error pass\n\
             \n\
             pub same(x: Int) -> Int\n  fails LookupError\n=\n  passes(x)\n    if error pass\n\
             \n\
             pub other(x: Int) -> Int\n  fails StoreError\n=\n  passes(x)\n    if error pass\n\
             \n\
             two_kinds(x: Int) -> Int = if x > 0 then fail Missing else fail Timeout\n\
             \n\
             pub unhandled(x: Int) -> Int\n  fails LookupError\n=\n  let y = raises(x)\n  y\n"
        ),
        [
            "U3008 t.una:32:5 `if error pass` passes a `LookupError` failure, and this code fails with `StoreError`; use `if error fail` with a variant of `StoreError`",
            "U3009 t.una:34:65 `Timeout` is a variant of `StoreError`, and this code fails with `LookupError`",
            "U3007 t.una:39:11 `raises` can fail with `LookupError`; handle the failure with an `if error` clause, or wrap the call in `attempt`",
        ]
    );
}

#[test]
fn inferred_error_kinds_include_passed_operation_failures() {
    // The operation's capability is inferred in the same pass as the error kind.
    assert_eq!(
        with_caps(
            "record(key: Int) -> Int\n=\n  let found = @store.find(key)\n    if error pass\n  found.or_else(default: 0)\n\
             \n\
             pub total(key: Int) -> Int\n  uses @store: Store[find]\n  fails StoreError\n=\n  record(key)\n    if error pass\n\
             \n\
             pub wrong(key: Int) -> Int\n  uses @store: Store[find]\n  fails LookupError\n=\n  record(key)\n    if error pass\n"
        ),
        [
            "U3008 t.una:33:5 `if error pass` passes a `StoreError` failure, and this code fails with `LookupError`; use `if error fail` with a variant of `LookupError`",
        ]
    );
}

// ----- handlers (U3008 to U3011; `spec/05 §5`) -----

#[test]
fn handlers_implement_every_operation_of_their_capability() {
    assert_eq!(
        with_caps(
            "handler partial() for Log\n  op write(line) = unit\n\
             \n\
             handler extra() for Log\n  op write(line) = unit\n  op flush() = unit\n  op close() = unit\n\
             \n\
             handler arity() for Log\n  op write() = unit\n  op flush() = unit\n\
             \n\
             handler not_capability() for StoreError\n  op write(line) = unit\n\
             \n\
             handler wrong_failure() for Store\n  op find(key) = fail Missing\n\
             \n\
             handler wrong_pass() for Store\n  op find(key) =\n    forward(key)\n      if error pass\n\
             \n\
             forward(key: Int) -> Option[Int] = if key > 0 then None else fail Missing\n\
             \n\
             handler outside() for Log\n  op write(line) = @elsewhere.write(line)\n  op flush() = unit\n\
             \n\
             handler inside() for Log\n  uses @out: Log[write]\n  op write(line) = @out.write(line)\n  op flush() = @out.flush()\n"
        ),
        [
            "U3011 t.una:15:9 handler `partial` does not implement the operation `flush` of `Log`",
            "U3010 t.una:21:6 capability `Log` has no operation `close`",
            "U3010 t.una:24:6 operation `write` of `Log` takes 1 parameter, and this implementation names 0",
            "U3010 t.una:27:30 `StoreError` is not a capability; `for` names the capability that a handler implements",
            "U3009 t.una:31:23 `Missing` is a variant of `LookupError`, and this code fails with `StoreError`",
            "U3008 t.una:36:7 `if error pass` passes a `LookupError` failure, and this code fails with `StoreError`; use `if error fail` with a variant of `StoreError`",
            "U3001 t.una:41:20 `@elsewhere` is not in scope",
            "U3002 t.una:47:21 `@out` permits `write`, not `flush`",
        ]
    );
}

// ----- failures (U3007 to U3009; `spec/06 §2, §3, §5`) -----

#[test]
fn failures_are_declared_and_handled_where_they_occur() {
    assert_eq!(
        with_caps(
            "pub handled(key: Int) -> Int\n  uses @store: Store\n  fails LookupError\n=\n  let v = @store.find(key)\n    if error fail Unavailable\n    if none fail Missing\n  v\n\
             \n\
             pub recovered(key: Int) -> Option[Int]\n  uses @store: Store\n=\n  @store.find(key)\n    if error use None\n\
             \n\
             pub attempted(key: Int) -> Bool\n  uses @store: Store\n=\n  let r = attempt @store.find(key)\n  r.is_ok()\n\
             \n\
             pub unhandled_let(key: Int) -> Unit\n  uses @store: Store\n=\n  let v = @store.find(key)\n  unit\n\
             \n\
             pub unhandled_final(key: Int) -> Option[Int]\n  uses @store: Store\n=\n  @store.find(key)\n\
             \n\
             pub wrong_kind() -> Int\n  fails LookupError\n=\n  fail Timeout\n\
             \n\
             pub cannot_fail() -> Int = fail Missing\n\
             \n\
             pub checked_kind(raw: Text) -> Text\n  fails LookupError\n=\n  checked(raw, raw.len > 0) else fail Corrupted\n"
        ),
        [
            "U3007 t.una:39:11 `@store.find` can fail with `StoreError`; handle the failure with an `if error` clause, or wrap the call in `attempt`",
            "U3007 t.una:45:3 `@store.find` can fail with `StoreError`; handle the failure with an `if error` clause, or wrap the call in `attempt`",
            "U3009 t.una:50:8 `Timeout` is a variant of `StoreError`, and this code fails with `LookupError`",
            "U3009 t.una:52:33 this code declares no error kind, so it cannot fail with `Missing`; declare `fails` in its header",
            "U7003 t.una:57:3 module `t` uses `checked` 1 time, and `[escape_budget] checked_per_module` allows 0",
            "U3009 t.una:57:39 `Corrupted` is a variant of `StoreError`, and this code fails with `LookupError`",
        ]
    );
}

// ----- one effect per line (U3013; `spec/04 §4`, ADR 0009) -----

#[test]
fn effects_and_failures_stand_alone_on_their_line() {
    assert_eq!(
        with_caps(
            "pub count(key: Int) -> Int\n  uses @store: Store\n  fails LookupError\n=\n  let a = (@store.find(key))\n    if error fail Unavailable\n  let b = attempt (@store.find(key))\n  let c = or_else(@store.find(key), default: 0)\n    if error fail Unavailable\n  let d = 1 + size(key)\n  let e = \"{size(key)}\"\n  0\n\
             \n\
             pub size(k: Int) -> Int\n  uses @store: Store\n=\n  k\n\
             \n\
             pub pure_nesting(xs: List[Int]) -> Int = sum(xs) + abs(min(1, b: 2))\n\
             \n\
             pub logs(line: Text) -> Unit\n  uses @log: Log\n=\n  if line.len > 0 then @log.write(line) else unit\n"
        ),
        [
            "U3013 t.una:22:19 `@store.find` has effects or can fail, so it must be the whole expression of a statement or `let`; bind its result with `let` first",
            "U3013 t.una:24:15 `size` has effects or can fail, so it must be the whole expression of a statement or `let`; bind its result with `let` first",
            "U3013 t.una:25:13 `size` has effects or can fail, so it must be the whole expression of a statement or `let`; bind its result with `let` first",
            "U3013 t.una:38:24 `@log.write` has effects or can fail, so it must be the whole expression of a statement or `let`; bind its result with `let` first",
        ]
    );
}

// ----- the second-class rules (U3005, U3006; `spec/05 §6`, `spec/10 §9`) -----

#[test]
fn capability_references_are_not_values() {
    assert_eq!(
        with_caps(
            "pub show(x: Int) -> Text = \"{x}\"\n\
             \n\
             pub a() -> Unit\n  uses @log: Log\n=\n  let l = @log\n  unit\n\
             \n\
             pub b() -> Text\n  uses @log: Log\n=\n  show(@log)\n\
             \n\
             pub c() -> Bool\n  uses @log: Log\n=\n  @log == @log\n\
             \n\
             pub d() -> Text\n  uses @log: Log\n=\n  \"{@log}\"\n\
             \n\
             kind Box\n  count: Int\n\
             \n\
             pub e() -> Box\n  uses @log: Log\n=\n  Box(count: @log)\n\
             \n\
             pub f() -> Int\n  uses @log: Log\n=\n  @log\n"
        ),
        [
            "U3005 t.una:20:11 capability `@log` is used as a value; capabilities pass by name, through `uses` and `with`",
            "U3005 t.una:26:8 capability `@log` is used as a value; capabilities pass by name, through `uses` and `with`",
            "U3005 t.una:31:3 capability `@log` is used as a value; capabilities pass by name, through `uses` and `with`",
            "U3005 t.una:31:11 capability `@log` is used as a value; capabilities pass by name, through `uses` and `with`",
            "U3005 t.una:36:5 capability `@log` is used as a value; capabilities pass by name, through `uses` and `with`",
            "U3005 t.una:44:14 capability `@log` is used as a value; capabilities pass by name, through `uses` and `with`",
            "U3005 t.una:49:3 capability `@log` is used as a value; capabilities pass by name, through `uses` and `with`",
        ]
    );
}

#[test]
fn effectful_functions_are_values_only_as_arguments() {
    assert_eq!(
        with_caps(
            "pub each(xs: List[Text], f: fn(Text) -> Unit uses @log: Log[write]) -> Unit\n  uses @log: Log[write]\n=\n  unit\n\
             \n\
             pub write(line: Text) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(line)\n\
             \n\
             pub passed(xs: List[Text]) -> Unit\n  uses @log: Log[write]\n=\n  each(xs, f: fn(t) -> @log.write(t))\n  each(xs, f: write)\n\
             \n\
             pub bound() -> Unit\n  uses @log: Log[write]\n=\n  let f: fn(Text) -> Unit uses @log: Log[write] = fn(t) -> @log.write(t)\n  let g = write\n  unit\n\
             \n\
             pub returned() -> fn(Text) -> Unit uses @log: Log[write]\n  uses @log: Log[write]\n=\n  write\n\
             \n\
             kind Holder\n  run: fn(Text) -> Unit uses @log: Log[write]\n\
             \n\
             pub pure_expected(xs: List[Text]) -> List[Unit]\n  uses @log: Log[write]\n=\n  xs.map(each: write)\n\
             \n\
             pub pure_lambda(xs: List[Text]) -> List[Unit]\n  uses @log: Log[write]\n=\n  xs.map(each: fn(t) -> @log.write(t))\n\
             \n\
             pub failing(xs: List[Int]) -> List[Int]\n=\n  xs.map(each: fn(x) -> if x > 0 then x else fail Missing)\n"
        ),
        [
            "U3006 t.una:34:51 a lambda that uses capabilities or can fail may only be passed directly as an argument",
            "U3006 t.una:35:11 `write` uses capabilities or can fail, so as a value it may only be passed directly as an argument",
            "U3006 t.una:41:3 `write` uses capabilities or can fail, so as a value it may only be passed directly as an argument",
            "U3006 t.una:44:8 field `run` of `Holder` has a function type with `uses` or `fails`, which a kind cannot store",
            "U3003 t.una:49:16 this function uses `@log: Log[write]`, which the expected function type does not allow",
            "U3004 t.una:54:25 `@log` is used here, but this code declares no `uses`, so it is pure; a function passed to a prelude function such as `map`, `filter` or `fold` may use capabilities only from edition 0.2",
            "U3009 t.una:58:51 this code declares no error kind, so it cannot fail with `Missing`; declare `fails` in its header",
        ]
    );
}

/// The program of [`u3004_in_a_pure_lambda_keeps_the_message_of_its_edition`]: a pure
/// lambda passed to the prelude's `map`, line 11, and to a public function, line 16.
const PURE_LAMBDAS: &str = "module t\n\n\
                            capability Log\n  op write(line: Text) -> Unit\n\n\
                            pub apply(x: Text, f: fn(Text) -> Unit) -> Unit = f(x)\n\n\
                            pub mapped(xs: List[Text]) -> List[Unit]\n  uses @log: Log[write]\n=\n  xs.map(each: fn(t) -> @log.write(t))\n\n\
                            pub applied(x: Text) -> Unit\n  uses @log: Log[write]\n=\n  apply(x, f: fn(t) -> @log.write(t))\n";

#[test]
fn u3004_in_a_pure_lambda_keeps_the_message_of_its_edition() {
    // Edition 0.1 keeps the v0.1.3 message, and names the edition when the lambda is an
    // argument of a prelude function, whose functions are pure there (#256).
    assert_eq!(
        single(PURE_LAMBDAS.trim_start_matches("module t\n\n")),
        [
            "U3004 t.una:11:25 `@log` is used here, but this code declares no `uses`, so it is pure; a function passed to a prelude function such as `map`, `filter` or `fold` may use capabilities only from edition 0.2",
            "U3004 t.una:16:24 `@log` is used here, but this code declares no `uses`, so it is pure",
        ]
    );
    // Edition 0.2: `map` takes the effect through its `uses of` clause.
    let manifest = MANIFEST.replace("edition = \"0.1\"", "edition = \"0.2\"");
    let files = [("una.toml", manifest.as_str()), ("src/t.una", PURE_LAMBDAS)];
    assert_eq!(
        check(&files, false),
        [
            "U3004 src/t.una:16:24 `@log` is used in a lambda whose type declares no `uses`, so the lambda is pure; pass it directly as an argument for a parameter whose type or `uses of` clause allows the effect",
        ]
    );
}

#[test]
fn parameters_of_effectful_function_types_are_second_class_too() {
    assert_eq!(
        with_caps(
            "pub each(xs: List[Text], f: fn(Text) -> Unit uses @log: Log[write]) -> Unit\n  uses @log: Log[write]\n=\n  f(\"first\")\n  each(xs, f)\n\
             \n\
             pub same(f: fn(Text) -> Unit uses @log: Log[write]) -> fn(Text) -> Unit uses @log: Log[write] = f\n\
             \n\
             pub boxed(f: fn(Text) -> Unit fails StoreError) -> Option[fn(Text) -> Unit fails StoreError] = Some(f)\n"
        ),
        [
            "U3006 t.una:21:97 `f` has a function type with `uses` or `fails`, so it may only be called or passed directly as an argument",
            "U3016 t.una:23:59 this type argument is, or holds, a function type with `uses` or `fails`, which is second-class, so it cannot instantiate a type parameter; take the function as a parameter of function type instead",
            "U3006 t.una:23:101 `f` has a function type with `uses` or `fails`, so it may only be called or passed directly as an argument",
        ]
    );
}

#[test]
fn lambdas_capture_the_capabilities_their_type_declares() {
    assert_eq!(
        with_caps(
            "pub each(xs: List[Text], f: fn(Text) -> Unit uses @log: Log[write]) -> Unit\n  uses @log: Log[write]\n=\n  unit\n\
             \n\
             pub held(xs: List[Text]) -> Unit\n  uses @log: Log\n=\n  each(xs, f: fn(t) -> @log.write(t))\n\
             \n\
             pub flushes(xs: List[Text]) -> Unit\n  uses @log: Log\n=\n  each(xs, f: fn(t) -> @log.flush())\n\
             \n\
             pub not_held(xs: List[Text]) -> Unit\n  uses @other: Log\n=\n  each(xs, f: fn(t) -> @other.write(t)) with @log = @other\n"
        ),
        [
            "U3002 t.una:28:29 `@log` permits `write`, not `flush`",
            "U3001 t.una:33:24 `@other` is not in scope",
        ]
    );
}

// ----- examples (U1001, U3001, U3010, U3014; `spec/08 §2`) -----

#[test]
fn examples_take_every_capability_from_given() {
    assert_eq!(
        with_caps(
            "pub write(line: Text) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(line)\n\
             \n\
             pub greet() -> Unit\n  uses @console: Console\n=\n  @console.print(\"hi\")\n\
             \n\
             pub lookup(key: Int) -> Int\n  uses @store: Store\n  fails LookupError\n=\n  let v = @store.find(key)\n    if error fail Unavailable\n    if none fail Missing\n  v\n\
             \n\
             handler quiet() for Log\n  op write(line) = unit\n  op flush() = unit\n\
             \n\
             handler cached() for Store\n  op find(key) = Some(key)\n\
             \n\
             example \"given handlers\"\n  given @log = quiet()\n  given @store = cached()\n  do write(\"{lookup(1)}\")\n  expect performed @log.write\n  expect not performed @store.find\n\
             \n\
             example \"no given\"\n  do write(\"x\")\n  expect true\n\
             \n\
             example \"host\"\n  do greet()\n  expect true\n\
             \n\
             example \"wrong handler\"\n  given @log = cached()\n  do write(\"x\")\n  expect true\n\
             \n\
             example \"performed\"\n  given @log = quiet()\n  do write(\"x\")\n  expect performed @audit.write\n  expect performed @log.close\n"
        ),
        [
            "U3001 t.una:49:6 `write` needs `@log: Log[write]`, which no `given` line binds",
            "U3014 t.una:53:6 `greet` needs `@console: Console[print]`, a host capability, and examples cannot use host capabilities; give it a handler with `given`",
            "U3010 t.una:57:16 this handler implements `Store`, and `write` needs `@log` to be `Log`",
            "U3001 t.una:64:20 no `given` line binds `@audit`",
            "U1001 t.una:65:25 capability `Log` has no operation `close`",
        ]
    );
}

// ----- `std.testing` (U3015; `spec/14 §4`) -----

#[test]
fn std_testing_serves_handlers_and_examples() {
    assert_eq!(
        single(
            "import std.testing (fake_hash)\n\
             \n\
             capability Hasher\n  op hash(seed: Int) -> internal Hashed\n\
             \n\
             handler fake() for Hasher\n  op hash(seed) = fake_hash(seed)\n\
             \n\
             pub digest(seed: Int) -> internal Hashed\n  uses @hasher: Hasher\n=\n  @hasher.hash(seed)\n\
             \n\
             example \"a fake hash\"\n  given @hasher = fake()\n  do digest(1)\n  expect result == fake_hash(1)\n\
             \n\
             pub direct(seed: Int) -> internal Hashed = fake_hash(seed)\n"
        ),
        [
            "U3015 t.una:21:44 `fake_hash` belongs to `std.testing`, which only handler operations and examples may use",
        ]
    );
}

// ----- grants (U3012; `spec/04 §8`, `spec/05 §7`) -----

#[test]
fn run_grants_cover_the_host_capabilities_of_main() {
    let main = "module app.main\n\
                \n\
                pub main() -> Unit\n  uses @console: Console\n  uses @files: Files[write]\n  uses @clock: Clock\n  fails FileError\n=\n  @console.print(\"hi\")\n  @files.write(\"out\", content: \"x\")\n    if error pass\n  let now = @clock.now()\n  unit\n";
    assert_eq!(
        package(
            "\n[run]\nentry = \"app.main\"\ngrants = [\"console\", \"files:./out:read\"]\n",
            &[("src/app/main.una", main)]
        ),
        [
            "U3012 src/app/main.una:5:3 `main` uses `write` of `Files`, which `[run] grants` does not grant",
            "U3012 src/app/main.una:6:3 `main` uses `now` of `Clock`, which `[run] grants` does not grant",
        ]
    );
    assert_eq!(
        package(
            "\n[run]\nentry = \"app.main\"\ngrants = [\"console\", \"clock\", \"files:./out:read,write\"]\n",
            &[("src/app/main.una", main)]
        ),
        [""; 0]
    );
    // Without `[run] grants`, `una run` checks its own grants (M7).
    assert_eq!(
        package(
            "\n[run]\nentry = \"app.main\"\n",
            &[("src/app/main.una", main)]
        ),
        [""; 0]
    );
    // A capability that is not a host capability can never be granted.
    let custom = "module app.main\n\
                  \n\
                  capability Store\n  op get() -> Int\n\
                  \n\
                  pub main() -> Unit\n  uses @store: Store\n=\n  let n = @store.get()\n  unit\n";
    assert_eq!(
        package(
            "\n[run]\nentry = \"app.main\"\n",
            &[("src/app/main.una", custom)]
        ),
        [
            "U3012 src/app/main.una:7:3 `main` uses `@store: Store`, which is not a host capability, so no grant can provide it",
        ]
    );
}

// ----- the escape budget (U7003; `spec/06 §6`) -----

#[test]
fn checked_is_counted_against_the_escape_budget() {
    let module = "module app.parse\n\
                  \n\
                  kind ParseError = | Empty\n\
                  \n\
                  pub a(raw: Text) -> Text\n  fails ParseError\n=\n  checked(raw, raw.len > 0) else fail Empty\n\
                  \n\
                  pub b(raw: Text) -> Text\n  fails ParseError\n=\n  checked(raw, raw.len > 1) else fail Empty\n\
                  \n\
                  pub c(raw: Text) -> Text\n  fails ParseError\n=\n  checked(raw, raw.len > 2) else fail Empty\n";
    assert_eq!(
        package(
            "\n[escape_budget]\nchecked_per_module = 3\n",
            &[("src/app/parse.una", module)]
        ),
        [""; 0]
    );
    assert_eq!(
        package(
            "\n[escape_budget]\nchecked_per_module = 1\n",
            &[("src/app/parse.una", module)]
        ),
        [
            "U7003 src/app/parse.una:13:3 module `app.parse` uses `checked` 3 times, and `[escape_budget] checked_per_module` allows 1",
        ]
    );
    assert_eq!(
        package("", &[("src/app/parse.una", module)]),
        [
            "U7003 src/app/parse.una:8:3 module `app.parse` uses `checked` 3 times, and `[escape_budget] checked_per_module` allows 0",
        ]
    );
    // The uses after the first one beyond the budget are related spans.
    let manifest = format!("{MANIFEST}\n[escape_budget]\nchecked_per_module = 1\n");
    let (_, types) = run(
        &[("una.toml", &manifest), ("src/app/parse.una", module)],
        false,
    );
    let related: Vec<String> = types
        .diagnostics
        .iter()
        .flat_map(|d| &d.related)
        .map(|r| {
            format!(
                "{}:{} {}",
                r.location.start.line, r.location.start.col, r.message
            )
        })
        .collect();
    assert_eq!(related, ["18:3 another use beyond the budget"]);
}

// ----- repairs (ADR 0028) -----

/// The mechanical edits of the diagnostics of a single file, as
/// "code: description [start-end -> replacement]".
fn repairs(body: &str) -> Vec<String> {
    let (_, types) = run(&[("t.una", &format!("module t\n\n{body}"))], true);
    types
        .diagnostics
        .iter()
        .flat_map(|d| {
            d.repairs.iter().filter_map(move |repair| {
                repair.edit.as_ref().map(|edit| {
                    format!(
                        "{}: {} [{}:{}-{}:{} -> {:?}]",
                        d.code,
                        repair.description,
                        edit.location.start.line,
                        edit.location.start.col,
                        edit.location.end.line,
                        edit.location.end.col,
                        edit.replacement
                    )
                })
            })
        })
        .collect()
}

#[test]
fn authority_diagnostics_carry_mechanical_edits_to_the_header() {
    assert_eq!(
        repairs(
            "kind LookupError = Missing | Unavailable\n\
             \n\
             capability Log\n  op write(line: Text) -> Unit\n  op flush() -> Unit\n\
             \n\
             pub note(line: Text) -> Unit\n  uses @log: Log[write, flush]\n=\n  @log.write(line)\n\
             \n\
             pub flushes() -> Unit\n  uses @log: Log[write]\n=\n  @log.flush()\n\
             \n\
             pub calls_note() -> Unit\n  uses @primary: Log[write]\n=\n  note(\"x\") with @log = @primary\n\
             \n\
             pub short_pure() -> Unit = note(\"x\")\n\
             \n\
             pub long_pure() -> Unit\n=\n  note(\"x\")\n  unit\n\
             \n\
             pub declared_other() -> Unit\n  uses @other: Log[write]\n  fails LookupError\n=\n  note(\"x\")\n  unit\n\
             \n\
             pub no_fails() -> Int = fail Missing\n\
             \n\
             pub with_ensures(x: Int) -> Int\n  ensures result == x\n=\n  if x > 0 then x else fail Missing\n"
        ),
        [
            "U3002: add `flush` to the `uses` clause of `@log` [15:3-15:24 -> \"uses @log: Log[write, flush]\"]",
            "U3002: add `flush` to the `uses` clause of `@primary` [20:3-20:28 -> \"uses @primary: Log[write, flush]\"]",
            "U3003: declare `uses @log: Log[write, flush]` in the header [24:25-24:28 -> \"\\n  uses @log: Log[write, flush]\\n=\\n  \"]",
            "U3003: declare `uses @log: Log[write, flush]` in the header [26:24-26:24 -> \"\\n  uses @log: Log[write, flush]\"]",
            "U3001: declare `uses @log: Log[write, flush]` in the header [33:3-33:3 -> \"uses @log: Log[write, flush]\\n  \"]",
            "U3009: declare `fails LookupError` in the header [38:22-38:25 -> \"\\n  fails LookupError\\n=\\n  \"]",
            "U3009: declare `fails LookupError` in the header [41:22-41:22 -> \"\\n  fails LookupError\"]",
        ]
    );
}

#[test]
fn an_operation_another_binding_permits_ranks_that_binding_first() {
    // Issue #69 (UI-005): widening the attenuated binding would undo what the header states,
    // so using the binding that already permits the operation ranks first.
    let (_, types) = run(
        &[(
            "t.una",
            concat!(
                "module t\n\n",
                "pub capability Payments\n",
                "  op charged(amount: Int) -> Unit\n",
                "  op refund(amount: Int) -> Unit\n",
                "\n",
                "pub settle(amount: Int) -> Unit\n",
                "  uses @shop: Payments[charged]\n",
                "  uses @agent: Payments[refund]\n",
                "=\n",
                "  @shop.refund(amount)\n",
            ),
        )],
        true,
    );
    // ADR 0097: the ranks are those of output order, and the widening is marked.
    let ranked: Vec<(u32, String, Option<Widening>)> = types
        .diagnostics
        .iter()
        .flat_map(|d| {
            d.repairs_in_order()
                .into_iter()
                .map(|r| (r.rank, r.repair.description.clone(), r.repair.widens))
        })
        .collect();
    assert_eq!(
        ranked,
        [
            (1, "use `@agent`, which permits `refund`".to_owned(), None),
            (
                2,
                "add `refund` to the `uses` clause of `@shop`".to_owned(),
                Some(Widening::Authority)
            ),
        ]
    );
}

#[test]
fn a_call_another_binding_permits_ranks_the_rebinding_first() {
    // Issue #69 (UI-005, final review F-002): at a call whose callee needs an operation the
    // attenuated binding lacks, rebinding the callee's reference to a binding that permits
    // everything it needs ranks first; widening the attenuated binding comes second, and is
    // the only repair when no binding permits everything the callee needs.
    let (_, types) = run(
        &[(
            "t.una",
            concat!(
                "module t\n\n",
                "pub capability Shelf\n",
                "  op stock(count: Int) -> Unit\n",
                "  op unstock(count: Int) -> Unit\n",
                "\n",
                "pub put_back(count: Int) -> Unit\n",
                "  uses @front: Shelf[unstock]\n",
                "=\n",
                "  @front.unstock(count)\n",
                "\n",
                "pub both(count: Int) -> Unit\n",
                "  uses @front: Shelf[stock, unstock]\n",
                "=\n",
                "  @front.unstock(count)\n",
                "\n",
                "pub by_call(count: Int) -> Unit\n",
                "  uses @front: Shelf[stock]\n",
                "  uses @back: Shelf[unstock]\n",
                "=\n",
                "  put_back(count)\n",
                "\n",
                "pub with_clause(count: Int) -> Unit\n",
                "  uses @front: Shelf[stock]\n",
                "  uses @back: Shelf[unstock]\n",
                "  uses @side: Shelf[stock]\n",
                "=\n",
                "  put_back(count) with @other = @side\n",
                "\n",
                "pub needs_both(count: Int) -> Unit\n",
                "  uses @front: Shelf[stock]\n",
                "  uses @back: Shelf[unstock]\n",
                "=\n",
                "  both(count)\n",
            ),
        )],
        true,
    );
    let ranked: Vec<(u32, String, String)> = types
        .diagnostics
        .iter()
        .filter(|d| d.code == Code::U3002)
        .flat_map(|d| {
            d.repairs_in_order().into_iter().map(|r| {
                (
                    r.rank,
                    r.repair.description.clone(),
                    r.repair
                        .edit
                        .as_ref()
                        .map_or(String::new(), |e| e.replacement.clone()),
                )
            })
        })
        .collect();
    let rebind = "add `with @front = @back` to this statement, since `@back` permits `unstock`";
    let widen = "add `unstock` to the `uses` clause of `@front`";
    let widened = "uses @front: Shelf[stock, unstock]";
    let expected: Vec<(u32, String, String)> = [
        (1, rebind, " with @front = @back"),
        (2, widen, widened),
        (1, rebind, ", @front = @back"),
        (2, widen, widened),
        (1, widen, widened),
    ]
    .into_iter()
    .map(|(rank, description, text)| (rank, description.to_owned(), text.to_owned()))
    .collect();
    assert_eq!(ranked, expected);
}
