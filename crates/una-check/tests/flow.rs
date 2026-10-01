//! Information flow (M6): labels on types and their inference, flows into declared types, the
//! program counter, failures and calls under it, sinks, policies and trusted modules, and
//! label categories (`spec/07`, `spec/10`, `spec/14 §3`, ADR 0031).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use pretty_assertions::assert_eq;
use una_check::{ModuleInput, PackageInput, TypeCheck, check_names, check_types};
use una_diag::Diagnostic;
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

// ----- explicit flows (U4001; `spec/07 §3` rules 1 and 5) -----

#[test]
fn values_flow_into_declared_types_below_their_labels() {
    assert_eq!(
        single(
            "kind Box[T]\n  item: T\n\
             \n\
             pub returned(pin: secret Int) -> Int = pin\n\
             \n\
             pub kept(pin: secret Int) -> secret Int = pin\n\
             \n\
             pub argument(pin: secret Int) -> Int\n=\n  plain(pin)\n\
             \n\
             pub annotated(pin: secret Int) -> Unit\n=\n  let copy: Int = pin\n  unit\n\
             \n\
             pub field(pin: secret Int) -> Box[Int] = Box(item: pin)\n\
             \n\
             pub field_kept(pin: secret Int) -> Box[secret Int] = Box(item: pin)\n\
             \n\
             pub generic(xs: List[secret Int]) -> Option[Int] = xs.first()\n\
             \n\
             pub generic_kept(xs: List[secret Int]) -> secret Option[secret Int] = xs.first()\n\
             \n\
             pub raised(pin: Int) -> secret Int = pin\n\
             \n\
             pub plain(x: Int) -> Int = x\n"
        ),
        [
            "U4001 t.una:6:40 the result of `returned` is `Int`, and this value is `secret Int`: a flow from `secret` to `public`",
            "U4001 t.una:12:9 parameter `x` of `plain` is `Int`, and this value is `secret Int`: a flow from `secret` to `public`",
            "U4001 t.una:16:19 `copy` is `Int`, and this value is `secret Int`: a flow from `secret` to `public`",
            "U4001 t.una:19:42 the result of `field` is `Box[Int]`, and this value is `Box[secret Int]`: a flow from `secret` to `public`",
            "U4001 t.una:23:52 the result of `generic` is `Option[Int]`, and this value is `Option[secret Int]`: a flow from `secret` to `public`",
        ]
    );
}

// ----- the program counter (U4001; `spec/07 §3` rule 2, T-If, T-Case) -----

#[test]
fn branches_on_labeled_values_raise_their_result() {
    assert_eq!(
        single(
            "pub classic(pin: secret Int) -> Int\n=\n  if pin == 1234\n    1\n  else\n    0\n\
             \n\
             pub line(pin: secret Int) -> Int = if pin > 0 then 1 else 0\n\
             \n\
             pub through_let(pin: secret Int) -> Int\n=\n  let flag = if pin > 0 then 1 else 0\n  flag\n\
             \n\
             pub labeled_result(pin: secret Int) -> secret Int = if pin > 0 then 1 else 0\n\
             \n\
             pub arms(pin: secret Int) -> Int\n=\n  when pin\n    in 0..9 -> 0\n    else -> 1\n\
             \n\
             pub unit_result(pin: secret Int) -> Unit = if pin > 0 then unit else unit\n\
             \n\
             pub chosen(c: secret Bool, f: fn(Int) -> Int, g: fn(Int) -> Int) -> Int\n=\n  let h = if c then f else g\n  h(1)\n"
        ),
        [
            "U4001 t.una:5:6 the result of `classic` is `public`, and which value it returns depends on this `secret` condition: a flow from `secret` to `public`",
            "U4001 t.una:10:39 the result of `line` is `public`, and which value it returns depends on this `secret` condition: a flow from `secret` to `public`",
            "U4001 t.una:15:3 the result of `through_let` is `Int`, and this value is `secret Int`: a flow from `secret` to `public`",
            "U4001 t.una:21:8 the result of `arms` is `public`, and which value it returns depends on this `secret` condition: a flow from `secret` to `public`",
            "U4001 t.una:30:3 the result of `chosen` is `Int`, and this value is `secret Int`: a flow from `secret` to `public`",
        ]
    );
}

// ----- the labels of values (D-090) -----

#[test]
fn operators_fields_and_patterns_take_the_labels_they_read() {
    assert_eq!(
        single(
            "kind Person\n  name: Text\n  code: secret Text\n\
             \n\
             kind Shape = Circle(radius: secret Int) | Dot\n\
             \n\
             pub shown(pin: secret Int) -> Text = \"pin {pin}\"\n\
             \n\
             pub same(xs: List[secret Int], ys: List[secret Int]) -> Bool = xs == ys\n\
             \n\
             pub size(xs: List[secret Int]) -> Int = xs.len\n\
             \n\
             pub present(o: Option[secret Int]) -> Bool = o is Some\n\
             \n\
             pub name_of(p: Person) -> Text = p.name\n\
             \n\
             pub code_of(p: Person) -> Text = p.code\n\
             \n\
             pub radius(shape: Shape) -> Int\n=\n  when shape\n    is Circle(radius) -> radius\n    is Dot -> 0\n\
             \n\
             pub otherwise(o: secret Option[Int]) -> Int\n=\n  o\n    if none use 0\n"
        ),
        [
            "U4001 t.una:9:38 the result of `shown` is `Text`, and this value is `secret Text`: a flow from `secret` to `public`",
            "U4001 t.una:11:64 the result of `same` is `Bool`, and this value is `secret Bool`: a flow from `secret` to `public`",
            "U4001 t.una:19:34 the result of `code_of` is `Text`, and this value is `secret Text`: a flow from `secret` to `public`",
            "U4001 t.una:24:26 the result of `radius` is `Int`, and this value is `secret Int`: a flow from `secret` to `public`",
            "U4001 t.una:29:3 the result of `otherwise` is `Int`, and this value is `secret Int`: a flow from `secret` to `public`",
        ]
    );
}

// ----- failures and calls under the program counter (U4002; `spec/07 §3` rules 3, 4) -----

#[test]
fn failures_and_calls_under_a_raised_program_counter_are_implicit_flows() {
    assert_eq!(
        single(
            "kind PinError = | Rejected\n\
             \n\
             capability Log\n  op write(line: Text) -> Unit\n\
             \n\
             handler forwarding() for Log\n  uses @out: Log[write]\n  op write(line) = @out.write(line)\n\
             \n\
             pub risky(x: Int) -> Int\n  fails PinError\n=\n  if x > 0 then x else fail Rejected\n\
             \n\
             pub note(line: Text) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(line)\n\
             \n\
             pub failing(pin: secret Int) -> secret Int\n  fails PinError\n=\n  if pin == 0\n    fail Rejected\n  else\n    1\n\
             \n\
             pub passing(pin: secret Int) -> secret Int\n  fails PinError\n=\n  if pin == 0\n    let v = risky(pin)\n      if error pass\n    v\n  else\n    1\n\
             \n\
             pub unwrapped(o: secret Option[Int]) -> secret Int\n  fails PinError\n=\n  o\n    if none fail Rejected\n\
             \n\
             pub pure_under(pin: secret Int) -> secret Int = if pin > 0 then abs(pin) else 0\n\
             \n\
             pub installed(c: secret Bool) -> Unit\n  uses @out: Log[write]\n=\n  if c\n    note(\"x\") with @log = forwarding()\n  else\n    unit\n"
        ),
        [
            "U4002 t.una:26:5 this `fail` depends on `secret` data, so its failure would reveal it; return a labeled value instead of failing",
            "U4002 t.una:34:13 `risky` can fail, and it is called under a `secret` condition, which would be revealed",
            "U4001 t.una:34:19 parameter `x` of `risky` is `Int`, and this value is `secret Int`: a flow from `secret` to `public`",
            "U4002 t.una:35:7 `if error pass` depends on `secret` data, so its failure would reveal it; return a labeled value instead of failing",
            "U4002 t.una:44:5 `if none fail` depends on `secret` data, so its failure would reveal it; return a labeled value instead of failing",
            "U4002 t.una:52:5 `note` has effects that accept at most `public` data, and it is called under a `secret` condition, which would be revealed",
            "U4002 t.una:52:27 handler `forwarding` has effects that accept at most `public` data, and it is installed under a `secret` condition, which would be revealed",
        ]
    );
}

#[test]
fn checked_reveals_its_value_through_its_failure() {
    assert_eq!(
        package(
            "\n[escape_budget]\nchecked_per_module = 2\n",
            &[(
                "src/app/parse.una",
                "module app.parse\n\
                 \n\
                 kind ParseError = | Empty\n\
                 \n\
                 pub public_value(raw: Text) -> Text\n  fails ParseError\n=\n  checked(raw, raw.len > 0) else fail Empty\n\
                 \n\
                 pub secret_value(raw: secret Text) -> secret Text\n  fails ParseError\n=\n  checked(raw, raw.len > 0) else fail Empty\n",
            )]
        ),
        [
            "U4002 src/app/parse.una:13:3 `checked` depends on `secret` data, so its failure would reveal it; return a labeled value instead of failing",
        ]
    );
}

// ----- sinks (U4003; `spec/07 §4`, `spec/14 §3`) -----

#[test]
fn operation_arguments_are_sinks_bounded_by_accepts() {
    assert_eq!(
        single(
            "capability Vault\n  op store(value: Text) -> Unit\n    accepts secret\n\
             \n\
             capability Log\n  op write(line: Text) -> Unit\n\
             \n\
             handler leaky() for Vault\n  uses @log: Log[write]\n  op store(value) = @log.write(value)\n\
             \n\
             pub print(name: secret Text) -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"name: {name}\")\n\
             \n\
             pub kept(name: secret Text) -> Unit\n  uses @vault: Vault[store]\n=\n  @vault.store(name)\n\
             \n\
             pub under(c: secret Bool) -> Unit\n  uses @log: Log[write]\n=\n  if c\n    @log.write(\"yes\")\n  else\n    unit\n\
             \n\
             pub inner(c: internal Bool) -> Unit\n  uses @log: Log[write]\n=\n  if c\n    @log.write(\"yes\")\n  else\n    unit\n\
             \n\
             pub hashed(name: secret Text) -> secret Hashed\n  uses @keys: Keys[hmac]\n  fails KeyError\n=\n  @keys.hmac(key_name: \"k\", data: name)\n    if error pass\n"
        ),
        [
            "U4003 t.una:12:21 `@log.write` accepts `public` data, and it runs in an operation body whose callers may pass `secret` data, which it would reveal",
            "U4003 t.una:17:18 `@console.print` accepts `public` data, and this argument is `secret`; declassify it through a policy first",
            "U4003 t.una:28:5 `@log.write` accepts `public` data, and it runs under a `secret` condition, which it would reveal",
            // R-034: the article follows the label.
            "U4003 t.una:36:5 `@log.write` accepts `public` data, and it runs under an `internal` condition, which it would reveal",
        ]
    );
}

// ----- lambdas and function values (D-088, D-093) -----

#[test]
fn lambdas_and_function_values_carry_labels() {
    assert_eq!(
        single(
            "pub apply(f: fn(Int) -> Int, x: Int) -> Int = f(x)\n\
             \n\
             pub mapped(xs: List[secret Int]) -> List[Int] = xs.map(each: fn(x) -> x + 1)\n\
             \n\
             pub mapped_kept(xs: List[secret Int]) -> secret List[secret Int] = xs.map(each: fn(x) -> x + 1)\n\
             \n\
             pub captured(xs: List[Int], s: secret Int) -> List[Int] = xs.map(each: fn(x) -> x + s)\n\
             \n\
             pub returned(s: secret Int) -> Int = apply(fn(x) -> s, x: 1)\n\
             \n\
             pub accepts_secret(f: fn(secret Int) -> Int) -> Int = 0\n\
             \n\
             pub public_only(x: Int) -> Int = x\n\
             \n\
             pub passed() -> Int = accepts_secret(public_only)\n"
        ),
        [
            "U4001 t.una:5:49 the result of `mapped` is `List[Int]`, and this value is `List[secret Int]`: a flow from `secret` to `public`",
            "U4001 t.una:9:59 the result of `captured` is `List[Int]`, and this value is `List[secret Int]`: a flow from `secret` to `public`",
            "U4001 t.una:11:53 the result of this lambda is `Int`, and this value is `secret Int`: a flow from `secret` to `public`",
            "U4001 t.una:17:38 parameter `f` of `accepts_secret` is `fn(secret Int) -> Int`, and this value is `fn(Int) -> Int`: a flow from `secret` to `public`",
        ]
    );
}

// ----- policies and trusted modules (U4004, U4005; `spec/07 §5, §6`) -----

#[test]
fn policies_live_in_trusted_modules_and_own_their_via_functions() {
    let keys = "trusted module app.keys\n\
                \n\
                pub policy mask\n  from secret[PII] Text\n  to internal Text\n  via masked\n  purpose \"show the last digits\"\n  owner privacy\n\
                \n\
                pub policy wrong_type\n  from secret[PII] Text\n  to internal Int\n  via masked\n  purpose \"test\"\n  owner privacy\n\
                \n\
                pub policy public_via\n  from secret Text\n  to Text\n  via exposed\n  purpose \"test\"\n  owner privacy\n\
                \n\
                masked(t: secret[PII] Text) -> internal Text = t\n\
                \n\
                pub exposed(t: secret Text) -> Text = t\n\
                \n\
                direct(t: secret[PII] Text) -> internal Text = masked(t)\n\
                \n\
                pub policy generic_via\n  from secret Text\n  to internal Text\n  via pass_any\n  purpose \"test\"\n  owner privacy\n\
                \n\
                pass_any[T](n: T) -> T = n\n";
    let main = "module app.main\n\
                \n\
                import app.keys (mask)\n\
                \n\
                pub policy outside\n  from secret Text\n  to Text\n  via helper\n  purpose \"test\"\n  owner team\n\
                \n\
                helper(t: secret Text) -> Text = t\n\
                \n\
                pub shown(t: secret[PII] Text) -> internal Text = mask(t)\n\
                \n\
                pub under(c: secret Bool, t: secret[PII] Text) -> secret Text\n=\n  if c\n    mask(t)\n  else\n    \"none\"\n";
    assert_eq!(
        package(
            "\n[labels]\ncategories = [\"PII\"]\n\n[trusted]\nmodules = [\"app.keys\"]\n",
            &[("src/app/keys.una", keys), ("src/app/main.una", main)]
        ),
        [
            "U4005 src/app/keys.una:13:7 `via` function `masked` must have type `fn(Text) -> Int`, except for labels; it is `fn(Text) -> Text`",
            "U4005 src/app/keys.una:20:7 the `via` function of a policy is private to its trusted module, and `exposed` is not; remove its `pub`, or declare it in this module",
            "U4005 src/app/keys.una:28:48 `masked` is the `via` function of policy `mask`, which is its only entry point; call `mask` instead",
            "U4005 src/app/keys.una:33:7 `via` function `pass_any` must have type `fn(Text) -> Text`, except for labels; it is `fn(T) -> T`",
            "U4004 src/app/main.una:5:12 policy `outside` is declared in module `app.main`, which is not trusted; declare `trusted module app.main` and list it under `[trusted] modules` in `una.toml`",
            "U4002 src/app/main.una:19:5 policy `mask` lowers data to `internal`, and it is called under a `secret` condition, which its result would reveal",
        ]
    );
}

// ----- categories (U4007; `spec/07 §2`) -----

#[test]
fn label_categories_are_declared_in_una_toml() {
    assert_eq!(
        single("pub f(t: secret[PII] Text) -> secret[PII] Text = t\n"),
        [
            "U4007 t.una:3:17 label category `PII` is not declared in `[labels] categories` of `una.toml`",
            "U4007 t.una:3:38 label category `PII` is not declared in `[labels] categories` of `una.toml`",
        ]
    );
    assert_eq!(
        package(
            "\n[labels]\ncategories = [\"PII\", \"Location\"]\n",
            &[(
                "src/app/a.una",
                "module app.a\n\
                 \n\
                 capability Vault\n  op store(value: Text) -> Unit\n    accepts secret[PIX]\n\
                 \n\
                 pub f(t: secret[PII, Location] Text) -> confidential[Locaton] Text = t\n",
            )]
        ),
        [
            "U4007 src/app/a.una:5:20 label category `PIX` is not declared in `[labels] categories` of `una.toml`",
            "U4007 src/app/a.una:7:54 label category `Locaton` is not declared in `[labels] categories` of `una.toml`",
            "U4001 src/app/a.una:7:70 the result of `f` is `confidential Text`, and this value is `secret[Location, PII] Text`: a flow from `secret[Location, PII]` to `confidential`",
        ]
    );
}

// ----- what is not checked (D-095) -----

#[test]
fn expect_lines_clauses_and_via_bodies_are_not_checked_for_labels() {
    assert_eq!(
        single(
            "pub secret_value() -> secret Int = 42\n\
             \n\
             pub guarded(pin: secret Int) -> secret Int\n  requires pin > 0\n  ensures result == pin\n=\n  pin\n\
             \n\
             example \"expect lines see labeled values\"\n  do secret_value()\n  expect result == 42\n\
             \n\
             example \"do lines are checked\"\n  do plain(secret_value())\n  expect true\n\
             \n\
             pub plain(x: Int) -> Int = x\n"
        ),
        [
            "U4001 t.una:16:12 parameter `x` of `plain` is `Int`, and this value is `secret Int`: a flow from `secret` to `public`",
        ]
    );
}

#[test]
fn predicate_results_are_checked() {
    assert_eq!(
        single(
            "predicate reflect big(x: secret Int) -> Bool = x > 10\n\
             \n\
             predicate reflect big_kept(x: secret Int) -> secret Bool = x > 10\n"
        ),
        [
            "U4001 t.una:3:48 the result of `big` is `Bool`, and this value is `secret Bool`: a flow from `secret` to `public`",
        ]
    );
}

#[test]
fn lambda_bodies_keep_the_labels_they_assume_for_their_parameters() {
    // `fold`'s accumulator starts `public` here and becomes `secret`, so the
    // body, checked with a `public` accumulator, could branch on a secret.
    assert_eq!(
        single(
            "pub summed(xs: List[secret Int]) -> secret Int = xs.fold(initial: 0, step: fn(total, x) -> total + x)\n\
             \n\
             pub started(xs: List[secret Int]) -> secret Int\n=\n  let zero: secret Int = 0\n  xs.fold(initial: zero, step: fn(total, x) -> total + x)\n"
        ),
        [
            "U4001 t.una:3:76 parameter `step` of `fold` is `fn(secret Int, secret Int) -> secret Int`, and this value is `fn(Int, secret Int) -> secret Int`: a flow from `secret` to `public`",
        ]
    );
}
