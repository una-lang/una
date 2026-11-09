//! Information flow (M6): labels on types and their inference, flows into declared types, the
//! program counter, failures and calls under it, sinks, policies and trusted modules, and
//! label categories (`spec/07`, `spec/10`, `spec/14 §3`, ADR 0031).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use pretty_assertions::assert_eq;
use una_check::{
    ModuleInput, PackageInput, TypeCheck, check_names, check_types, check_types_with_rejected,
};
use una_diag::Diagnostic;
use una_project::{Edition, Manifest, parse_manifest};
use una_syntax::{Interner, KeywordSet, SourceDb, lex_in, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n";

/// Runs the name and type checks over `files`.
fn run(files: &[(&str, &str)], single: bool) -> (Vec<Diagnostic>, TypeCheck) {
    with_input(files, single, |input| {
        (check_names(input), check_types(input))
    })
}

/// Calls `f` with the package input of `files`.
fn with_input<R>(
    files: &[(&str, &str)],
    single: bool,
    f: impl FnOnce(&PackageInput<'_>) -> R,
) -> R {
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
        // The keywords of the package's edition: `una.toml` comes first among `files`.
        let keywords = match settings.edition {
            Edition::V0_1 => KeywordSet::V0_1,
            Edition::V0_2 => KeywordSet::V0_2,
            Edition::V0_3 => KeywordSet::V0_3,
            Edition::V0_4 => KeywordSet::V0_4,
            Edition::V0_5 => KeywordSet::V0_5,
        };
        let lexed = lex_in(db.file(id), id, &mut interner, keywords);
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
    f(&input)
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
            "U4003 t.una:17:18 `@console.print` accepts `public` data, and this argument is `secret`",
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
            "U4004 src/app/main.una:5:12 policy `outside` is declared in module `app.main`, which is not trusted",
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

// ----- decisions on values of a type parameter (#354, #362) -----

#[test]
fn edition_0_1_names_a_decision_in_its_messages_as_edition_0_2_does() {
    // Since #354 edition 0.1 tells `ℓ*_T` from `ℓ_T` in its flows, so its messages name
    // `label(T*)` too: written as `label(T)`, the flow read "from `label(T)` to `label(T)`".
    assert_eq!(
        package(
            "",
            &[(
                "src/t.una",
                "module t\n\npub choose[T](a: T, b: T, p: T, q: T) -> T\n=\n  \
                 let picked: T = if a == b then p else q\n  picked\n"
            )]
        ),
        [
            "U4001 src/t.una:5:19 `picked` is `label(T) T`, and this value is `label(T) + label(T*) T`: a flow from `label(T) + label(T*)` to `label(T)`",
        ]
    );
}

#[test]
fn exporting_rejected_declarations_changes_nothing_else() {
    // ADR 0091 decision 5: the Core of the declarations rejected for information flow is
    // elaborated apart, so the program every engine reads (the interpreter, the native backend,
    // `una core` without the flag), the diagnostics, the headers, the obligations and the
    // file-state label are those of `check_types`, on a package that mixes accepted and
    // rejected declarations, and the rejected one has its Core.
    let files = [
        ("una.toml", MANIFEST),
        (
            "src/m.una",
            "module m\n\npub ok() -> Int = 1\n\npub leak(s: secret Int) -> Int = s\n\npub twice(n: Int) -> Int = n + n\n",
        ),
    ];
    with_input(&files, false, |input| {
        let plain = check_types(input);
        let (with, rejected) = check_types_with_rejected(input);
        assert_eq!(
            format!("{:?}", with.program),
            format!("{:?}", plain.program)
        );
        assert_eq!(
            format!("{:?}", with.diagnostics),
            format!("{:?}", plain.diagnostics)
        );
        assert_eq!(
            format!("{:?}", with.headers),
            format!("{:?}", plain.headers)
        );
        assert_eq!(
            format!("{:?}", with.obligations),
            format!("{:?}", plain.obligations)
        );
        assert_eq!(
            format!("{:?}", with.of_lambdas),
            format!("{:?}", plain.of_lambdas)
        );
        assert_eq!(with.file_state, plain.file_state);
        assert!(!plain.diagnostics.is_empty());
        let names: Vec<String> = rejected
            .iter()
            .map(|r| match &r.decl {
                una_core::Decl::Fun(fun) => fun.name.clone(),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(names, ["leak"]);
        assert!(rejected.iter().all(|r| r.module == "m"));
        // The rejected declaration is not in the program.
        let program = format!("{:?}", plain.program);
        assert!(!program.contains("\"leak\""), "{program}");
    });
}

// ----- labeled answers named in flow reports (ADR 0094 rule 7) -----

/// Every diagnostic of `module`, in an edition `0.3` package that declares the category
/// `Service`, as "code line:col message", each followed by its related locations as
/// "  related line:col message" and its repairs as "  repair [escape] description".
fn answers(module: &str) -> Vec<String> {
    let manifest = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n\n[labels]\ncategories = [\"Service\"]\n";
    let files = [("una.toml", manifest), ("src/m.una", module)];
    let (names, types) = run(&files, false);
    let mut diagnostics: Vec<Diagnostic> = names.into_iter().chain(types.diagnostics).collect();
    diagnostics.sort_by_key(|d| (d.primary.start, d.code));
    let mut lines = Vec::new();
    for d in &diagnostics {
        lines.push(format!(
            "{} {}:{} {}",
            d.code, d.primary.start.line, d.primary.start.col, d.message
        ));
        for related in &d.related {
            lines.push(format!(
                "  related {}:{} {}",
                related.location.start.line, related.location.start.col, related.message
            ));
        }
        let mut repairs = d.repairs.clone();
        repairs.sort_by_key(una_diag::Repair::order_key);
        for repair in &repairs {
            let escape = if repair.escape.is_some() {
                "escape "
            } else {
                ""
            };
            lines.push(format!("  repair {escape}{}", repair.description));
        }
    }
    lines
}

const ANSWER_CAPS: &str = "module m\n\
\n\
pub kind Lookup = | NotFound\n\
\n\
pub capability Files\n  op owner(file: Text) -> Text\n  op role(file: Text) -> Text\n  op find(file: Text) -> Text\n    fails Lookup\n\
\n\
pub capability Mailer\n  op send(to: Text) -> Unit\n";

#[test]
fn flows_of_a_labeled_answer_name_its_binding_and_operation() {
    // ADR 0094 rule 7: the U4001, U4002 and U4003 that a labeled answer causes name the binding
    // and operation whose label the value carries, in the message and the first repair, with
    // the call as a related location; the policy that releases it ranks last (ADR 0095).
    let module = format!(
        "{ANSWER_CAPS}\n\
pub returned(file: Text) -> Text\n  uses @service_files: Files[owner gives internal[Service]]\n=\n  @service_files.owner(file)\n\
\n\
pub sent(file: Text) -> Unit\n  uses @service_files: Files[owner gives internal[Service]]\n  uses @mailer: Mailer[send]\n=\n  let owner = @service_files.owner(file)\n  @mailer.send(to: owner)\n\
\n\
pub branched(file: Text) -> Unit\n  uses @service_files: Files[owner gives internal[Service]]\n  uses @mailer: Mailer[send]\n=\n  let owner = @service_files.owner(file)\n  if owner == \"root\"\n    @mailer.send(to: \"root\")\n  unit\n\
\n\
pub passed_on(file: Text) -> internal[Service] Text\n  uses @service_files: Files[find gives internal[Service]]\n  fails Lookup\n=\n  let found = @service_files.find(file)\n    if error pass\n  found\n"
    );
    assert_eq!(
        answers(&module),
        [
            "U4001 17:3 the result of `returned` is `Text`, and this value is `internal[Service] Text`: a flow from `internal[Service]` to `public`; this value carries the answer of `@service_files.owner`, labeled `internal[Service]`",
            "  repair this value carries the answer of `@service_files.owner`, labeled `internal[Service]`: read `owner` through a binding whose `owner` answers within `public`, or send the value only where `internal[Service]` data may go",
            "  repair escape release the answer of `@service_files.owner` through a policy of a trusted module",
            "U4003 24:20 `@mailer.send` accepts `public` data, and this argument is `internal[Service]`; this argument carries the answer of `@service_files.owner`, labeled `internal[Service]`",
            "  related 23:15 the answer of `@service_files.owner`",
            "  repair this argument carries the answer of `@service_files.owner`, labeled `internal[Service]`: read `owner` through a binding whose `owner` answers within `public`, or send the value only where `internal[Service]` data may go",
            "  repair send `@mailer.send` only data labeled `public` or lower, or send this value to an operation that accepts `internal[Service]`",
            "  repair escape declassify the value through a policy of a trusted module",
            "U4003 32:5 `@mailer.send` accepts `public` data, and it runs under an `internal[Service]` condition, which it would reveal; the condition carries the answer of `@service_files.owner`, labeled `internal[Service]`",
            "  related 31:6 the labeled condition",
            "  related 30:15 the answer of `@service_files.owner`",
            "  repair the condition carries the answer of `@service_files.owner`, labeled `internal[Service]`: read `owner` through a binding whose `owner` answers within `public`, or decide this without branching on it",
            "  repair escape release the answer of `@service_files.owner` through a policy of a trusted module",
            "U4002 40:5 `if error pass` depends on `internal[Service]` data, so its failure would reveal it; return a labeled value instead of failing; the condition carries the answer of `@service_files.find`, labeled `internal[Service]`",
            "  related 39:15 the labeled condition",
            "  repair the condition carries the answer of `@service_files.find`, labeled `internal[Service]`: read `find` through a binding whose `find` answers within `public`, or decide this without branching on it",
            "  repair escape release the answer of `@service_files.find` through a policy of a trusted module",
        ]
    );
}

#[test]
fn flows_name_only_an_answer_whose_label_they_carry_above_the_sink() {
    // ADR 0094 rule 7, boundaries: a flow whose label no labeled answer explains names none,
    // even when the body reads one; an answer within the sink is not named; of two answers,
    // the one above the sink is named, though the other is read first.
    let module = format!(
        "{ANSWER_CAPS}\n\
pub other(file: Text, pin: secret Text) -> Text\n  uses @service_files: Files[owner gives internal[Service]]\n=\n  let owner = @service_files.owner(file)\n  pin\n\
\n\
pub within(file: Text, pin: secret Text) -> internal[Service] Text\n  uses @service_files: Files[owner gives internal[Service]]\n=\n  let owner = @service_files.owner(file)\n  pin\n\
\n\
pub two(file: Text) -> internal[Service] Text\n  uses @service_files: Files[role gives internal[Service], owner gives secret[Service]]\n=\n  let role = @service_files.role(file)\n  let owner = @service_files.owner(file)\n  \"{{role}} {{owner}}\"\n"
    );
    assert_eq!(
        answers(&module),
        [
            "U4001 18:3 the result of `other` is `Text`, and this value is `secret Text`: a flow from `secret` to `public`",
            "U4001 24:3 the result of `within` is `internal[Service] Text`, and this value is `secret Text`: a flow from `secret` to `internal[Service]`",
            "U4001 31:3 the result of `two` is `internal[Service] Text`, and this value is `secret[Service] Text`: a flow from `secret[Service]` to `internal[Service]`; this value carries the answer of `@service_files.owner`, labeled `secret[Service]`",
            "  related 30:15 the answer of `@service_files.owner`",
            "  repair this value carries the answer of `@service_files.owner`, labeled `secret[Service]`: read `owner` through a binding whose `owner` answers within `internal[Service]`, or send the value only where `secret[Service]` data may go",
            "  repair escape release the answer of `@service_files.owner` through a policy of a trusted module",
        ]
    );
}

#[test]
fn a_parameter_run_under_a_labeled_answer_and_a_contravariant_flow_name_it() {
    // ADR 0094 rule 7 (review of #1536, L2 and L3): the U4002 of an effectful parameter that
    // the callee runs under a labeled list names the answer the list is, and the U4001 at a
    // contravariant position says that the flow, not the function value, carries the answer.
    let module = "module m\n\
\n\
pub capability Files\n  op sizes(file: Text) -> List[Int]\n\
\n\
relay(file: Text, f: fn(Int) -> Unit) -> Unit\n  uses of f\n  uses @service_files: Files[sizes gives internal[Service]]\n=\n  let xs = @service_files.sizes(file)\n  let done = xs.map(each: f)\n  unit\n";
    let lines = answers(module);
    let joined = lines.join("\n");
    let u4002 = lines
        .iter()
        .find(|l| l.starts_with("U4002 11:27 "))
        .unwrap_or_else(|| panic!("{joined}"));
    assert!(
        u4002.ends_with(
            "; the condition carries the answer of `@service_files.sizes`, labeled `internal[Service]`"
        ),
        "{joined}"
    );
    let u4001 = lines
        .iter()
        .find(|l| l.starts_with("U4001 11:27 "))
        .unwrap_or_else(|| panic!("{joined}"));
    assert!(
        u4001.ends_with(
            "; this flow carries the answer of `@service_files.sizes`, labeled `internal[Service]`"
        ),
        "{joined}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l
                .contains("or pass a function whose parameter accepts `internal[Service]` data")),
        "{joined}"
    );
    assert!(!joined.contains("this value carries"), "{joined}");
}

/// A package of edition 0.4 whose sites `τ_conc` reaches through each kind of edge of ADR 0116
/// P1 (slice 2 of #1087): a direct call, a function value through `Λ`, a prelude function with a
/// function argument (`map`), a `uses of` parameter through `Λ`, an operation through `K(c)` to
/// a handler, an installation of a handler, a cycle of calls, and a site in the block of a
/// `tasks` statement.
const CONCURRENT: &str = "module app\n\nimport std.time (millisecond)\n\ncapability C\n  op get(k: Int) -> Int\n\nhandler h() for C\n  op get(k) = in_handler(k)\n\nstep(n: Int) -> Int = n + 1\n\nsec(key: secret Int) -> secret Int = key * 3\n\ndirect(n: Int) -> Int\n=\n  let w = step(n) within 10 * millisecond\n    if error use 0\n  w\n\nby_value(n: Int) -> Int\n=\n  let w = step(n) within 10 * millisecond\n    if error use 0\n  w\n\nby_map(n: Int) -> Int\n=\n  let w = step(n) within 10 * millisecond\n    if error use 0\n  w\n\nin_handler(n: Int) -> Int\n=\n  let w = step(n) within 10 * millisecond\n    if error use 0\n  w\n\nafter(n: Int) -> Int\n=\n  let w = step(n) within 10 * millisecond\n    if error use 0\n  w\n\nping(n: Int) -> Int = if n > 0 then pong(n - 1) else 0\n\npong(n: Int) -> Int\n=\n  let w = step(n) within 10 * millisecond\n    if error use 0\n  let v = lone(n)\n  if n > 0 then ping(n - 1) else w + v\n\nlone(n: Int) -> Int\n=\n  let w = step(n) within 10 * millisecond\n    if error use 0\n  w\n\napply(f: fn(Int) -> Int, n: Int) -> Int = f(n)\n\npub run_direct(key: secret Int, n: Int) -> secret Int\n=\n  together\n    let a = direct(n)\n    let b = sec(key)\n  b\n\npub run_value(n: Int) -> Int\n=\n  together\n    let a = apply(by_value, n)\n    let b = step(n)\n  b\n\npub run_map(n: Int) -> List[Int]\n=\n  together\n    let a = [n].map(each: fn(x: Int) -> by_map(x))\n    let b = step(n)\n  a\n\npub run_op(n: Int) -> Int\n  uses @c: C[get]\n=\n  together\n    let a = @c.get(n)\n    let b = step(n)\n  b\n\npub run_cycle(n: Int) -> Int\n=\n  together\n    let a = ping(n)\n    let b = step(n)\n  b\n\npub run_block(key: secret Int, n: Int) -> Unit\n=\n  tasks\n    start ignore(sec(key))\n    let w = step(n) within 10 * millisecond\n      if error use 0\n    unit\n  let z = after(n)\n  unit\n\ncapability D\n  op get(k: Int) -> Int\n\nhandler hd() for D\n  op get(k) = in_install(k)\n\nin_install(n: Int) -> Int\n=\n  let w = step(n) within 10 * millisecond\n    if error use 0\n  w\n\nrun_with(f: fn(Int) -> Int, n: Int) -> Int\n  uses of f\n=\n  f(n)\n\nby_uses_of(n: Int) -> Int\n=\n  let w = step(n) within 10 * millisecond\n    if error use 0\n  w\n\npub run_uses_of(key: secret Int, n: Int) -> secret Int\n=\n  together\n    let a = run_with(by_uses_of, n)\n    let b = sec(key)\n  b\n\npub run_install(key: secret Int, n: Int) -> secret Int\n=\n  together\n    let a = @d.get(n) with @d = hd()\n    let b = sec(key)\n  b\n\nignore(n: secret Int) -> Unit = unit\n";

/// Labels by the function that holds what they label, as their text.
type Labeled = Vec<(String, Option<String>)>;

/// `τ_conc` of the `within` of each function, and `κ` of each construct, by the function that
/// holds it, as the type check records them (ADR 0116 P2 to P4).
fn concurrency(text: &str) -> (Labeled, Labeled) {
    let manifest = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.4\"\n";
    let (_, types) = run(&[("una.toml", manifest), ("src/app.una", text)], false);
    let name = |node: &Option<una_check::TimingNode>| match node {
        Some(una_check::TimingNode::Function(name)) => name.clone(),
        other => format!("{other:?}"),
    };
    let label = |label: &Option<una_core::Label>| label.as_ref().map(ToString::to_string);
    let graph = &types.timing_graph;
    let sites = graph
        .sites
        .iter()
        .filter(|site| site.kind == "within")
        .map(|site| (name(&site.node), label(&site.label)))
        .collect();
    let constructs = graph
        .constructs
        .iter()
        .map(|c| (name(&c.node), label(&c.label)))
        .collect();
    (sites, constructs)
}

#[test]
fn tau_conc_follows_every_kind_of_edge_from_the_parts_of_a_construct() {
    let (sites, constructs) = concurrency(CONCURRENT);
    let some = |text: &str| Some(text.to_owned());
    let at = |function: &str| -> Vec<Option<String>> {
        sites
            .iter()
            .filter(|(name, _)| name == function)
            .map(|(_, label)| label.clone())
            .collect()
    };
    // `κ` of each construct: the parts' own reads and what they reach. Under IF5 item 7 (ADR
    // 0116, amendment of 2026-11-06), `τ⁰` of a part also counts the values the functions it
    // reaches compute. `direct`, `in_handler`, `pong`, `lone` and `in_install` each bind `w`, the
    // outcome of a `within`, which is labeled by that `within`'s `τ`, whose `τ_conc` is now the
    // precise one (`spec/27 §11`): `κ` of the construct beside it. So `κ` and `τ_conc` meet at
    // their least fixpoint, which the probe of pass 2 reaches from below: `secret` beside work on
    // `key`, `public` beside `public` work. `run_value`, `run_map` and `run_uses_of` are `⊤`
    // through `Λ` (IF5 item 4).
    assert_eq!(
        constructs,
        [
            ("app.run_direct".to_owned(), some("secret")),
            ("app.run_value".to_owned(), some("restricted")),
            ("app.run_map".to_owned(), some("restricted")),
            ("app.run_op".to_owned(), some("public")),
            ("app.run_cycle".to_owned(), some("public")),
            ("app.run_block".to_owned(), some("secret")),
            ("app.run_uses_of".to_owned(), some("restricted")),
            ("app.run_install".to_owned(), some("secret")),
        ]
    );
    // A direct call from a branch beside work on `key`.
    assert_eq!(at("app.direct"), [some("secret")]);
    // A function value, through `apply`'s call and `Λ`; a lambda given to `map`, through `Λ`.
    assert_eq!(at("app.by_value"), [some("restricted")]);
    assert_eq!(at("app.by_map"), [some("restricted")]);
    // An operation, through its capability to the handler that serves it: beside a construct
    // whose `κ` is `public`, which is not the same as nothing beside it.
    assert_eq!(at("app.in_handler"), [some("public")]);
    // A `uses of` parameter called in `run_with`, through `Λ`.
    assert_eq!(at("app.by_uses_of"), [some("restricted")]);
    // A handler installed in a branch beside work on `key`, whose operation calls the function.
    assert_eq!(at("app.in_install"), [some("secret")]);
    // A cycle shares one `conc`, and so does what it calls.
    assert_eq!(at("app.pong"), [some("public")]);
    assert_eq!(at("app.lone"), [some("public")]);
    // A site in the block of a `tasks` statement runs beside its tasks (P2).
    assert_eq!(at("app.run_block"), [some("secret")]);
    // Called after the statement has ended, nothing runs beside it.
    assert_eq!(at("app.after"), [None]);
}

/// The loop of `tests/conformance/security/tconc_for_ever_within_tasks`, whose `tasks` statement
/// also raises `L_res`: its U4001 names `τ_conc` at the loop, the work beside it (ADR 0116 C,
/// `spec/27 §11`), and not only the residue, so a loop rule that dropped `τ_conc` would name the
/// residue instead and fail here.
#[test]
fn a_for_ever_within_beside_secret_tasks_names_the_tasks_beside_it() {
    let manifest = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.4\"\n";
    let text = "module poll\n\nimport std.time (millisecond)\n\ncrunch(key: secret Int) -> secret Int = key * 3\n\n--- Work on `key`, which a started task runs.\nchurn(key: secret Int) -> Unit\n=\n  let spent = crunch(key)\n  unit\n\n--- Starts `churn(key)`, and polls beside it, in the block of the `tasks` statement.\npub serve(key: secret Int, url: Text) -> Unit\n  uses @http: Http[get]\n  diverges\n=\n  tasks\n    start churn(key)\n    for ever within 100 * millisecond\n      let reading = attempt @http.get(url, headers: [], key_name: None)\n      stop\n    unit\n  unit\n";
    let (_, types) = run(&[("una.toml", manifest), ("src/poll.una", text)], false);
    let loop_errors: Vec<&str> = types
        .diagnostics
        .iter()
        .filter(|d| d.code.as_str() == "U4001" && d.message.contains("`for ever within`"))
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(loop_errors.len(), 1, "{loop_errors:?}");
    assert!(
        loop_errors[0].contains("depends on tasks that can run beside it"),
        "{loop_errors:?}"
    );
}

// ----- `Json` values that carry labeled data (ADR 0073 J5, Proposed amendment of 2026-11-09, #917) -----

/// The diagnostics of one module `t` in a package of `edition`, at `src/t.una`, whose
/// declarations start on line 3.
fn in_edition(edition: &str, body: &str) -> Vec<String> {
    let manifest =
        format!("[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"{edition}\"\n");
    let module = format!("module t\n\n{body}");
    check(&[("una.toml", &manifest), ("src/t.una", &module)], false)
}

/// Functions that build `Json` values from labeled data: in edition 0.4 each constructor
/// accepts them and labels its result with the join of every label of its arguments; reading
/// the value back, writing it with `to_json` and passing it to a sink are raised by that label.
const JSON_LABELS: &str = "\
pub wrapped(token: secret Text) -> secret Json = Str(token)\n\
\n\
pub written(token: secret Text) -> secret Text = Str(token).to_json()\n\
\n\
pub inner(t: internal Text) -> internal Json = Str(value: t)\n\
\n\
pub nested(token: secret Text) -> secret Json = Array([Null, Str(token)])\n\
\n\
pub keyed(m: Map[secret Text, Json]) -> secret Json = Object(m)\n\
\n\
pub listed(items: List[secret Json]) -> secret Json = Array(items)\n\
\n\
pub empty() -> Json = Array([])\n\
\n\
pub null() -> Json = Null\n\
\n\
pub plain(n: Int, b: Bool) -> Json = Array([Number(n), Boolean(b), Str(\"x\")])\n\
\n\
pub leaked(token: secret Text) -> Json = Str(token)\n\
\n\
pub leaked_keys(m: Map[secret Text, Json]) -> Json = Object(m)\n\
\n\
pub leaked_items(items: List[secret Json]) -> Json = Array(items)\n\
\n\
pub leaked_nested(token: secret Text) -> Json = Array([Null, Str(token)])\n\
\n\
pub printed(token: secret Text) -> Unit\n  uses @console: Console[print]\n=\n  @console.print(Str(token).to_json())\n\
\n\
pub bound(token: secret Text) -> Text\n=\n  when Str(token)\n    is Str(value) -> value\n    else -> \"\"\n\
\n\
pub equal(token: secret Text) -> Bool = Str(token) == Null\n";

#[test]
fn json_constructors_carry_the_labels_of_their_arguments_in_edition_0_4() {
    // Lines 3 to 19 build labeled and public values and are accepted; the labeled value then
    // meets each lower declared type or sink where it reaches one, never at the constructor.
    assert_eq!(
        in_edition("0.4", JSON_LABELS),
        [
            "U4001 src/t.una:21:42 the result of `leaked` is `Json`, and this value is `secret Json`: a flow from `secret` to `public`",
            "U4001 src/t.una:23:54 the result of `leaked_keys` is `Json`, and this value is `secret Json`: a flow from `secret` to `public`",
            "U4001 src/t.una:25:54 the result of `leaked_items` is `Json`, and this value is `secret Json`: a flow from `secret` to `public`",
            "U4001 src/t.una:27:49 the result of `leaked_nested` is `Json`, and this value is `secret Json`: a flow from `secret` to `public`",
            "U4003 src/t.una:32:18 `@console.print` accepts `public` data, and this argument is `secret`",
            "U4001 src/t.una:36:8 the result of `bound` is `public`, and which value it returns depends on this `secret` condition: a flow from `secret` to `public`",
            "U4001 src/t.una:37:22 the result of `bound` is `Text`, and this value is `secret Text`: a flow from `secret` to `public`",
            "U4001 src/t.una:40:41 the result of `equal` is `Bool`, and this value is `secret Bool`: a flow from `secret` to `public`",
        ]
    );
}

#[test]
fn json_constructors_keep_public_positions_before_edition_0_4() {
    // ADR 0073 J5 as written: every labeled argument is U4001 at the constructor.
    assert_eq!(
        in_edition("0.3", JSON_LABELS),
        [
            "U4001 src/t.una:3:54 field `value` of `Str` is `Text`, and this value is `secret Text`: a flow from `secret` to `public`",
            "U4001 src/t.una:5:54 field `value` of `Str` is `Text`, and this value is `secret Text`: a flow from `secret` to `public`",
            "U4001 src/t.una:7:59 field `value` of `Str` is `Text`, and this value is `internal Text`: a flow from `internal` to `public`",
            "U4001 src/t.una:9:66 field `value` of `Str` is `Text`, and this value is `secret Text`: a flow from `secret` to `public`",
            "U4001 src/t.una:11:62 field `fields` of `Object` is `Map[Text, Json]`, and this value is `Map[secret Text, Json]`: a flow from `secret` to `public`",
            "U4001 src/t.una:13:61 field `items` of `Array` is `List[Json]`, and this value is `List[secret Json]`: a flow from `secret` to `public`",
            "U4001 src/t.una:21:46 field `value` of `Str` is `Text`, and this value is `secret Text`: a flow from `secret` to `public`",
            "U4001 src/t.una:23:61 field `fields` of `Object` is `Map[Text, Json]`, and this value is `Map[secret Text, Json]`: a flow from `secret` to `public`",
            "U4001 src/t.una:25:60 field `items` of `Array` is `List[Json]`, and this value is `List[secret Json]`: a flow from `secret` to `public`",
            "U4001 src/t.una:27:66 field `value` of `Str` is `Text`, and this value is `secret Text`: a flow from `secret` to `public`",
            "U4001 src/t.una:32:22 field `value` of `Str` is `Text`, and this value is `secret Text`: a flow from `secret` to `public`",
            "U4001 src/t.una:36:12 field `value` of `Str` is `Text`, and this value is `secret Text`: a flow from `secret` to `public`",
            "U4001 src/t.una:40:45 field `value` of `Str` is `Text`, and this value is `secret Text`: a flow from `secret` to `public`",
        ]
    );
}

#[test]
fn only_json_constructors_take_labeled_arguments() {
    // A user kind keeps its fields' declared labels in edition 0.4 (`spec/07 §7` rule 2), a
    // `Json` built under a secret condition is raised by the branch join (rule 2), not by the
    // constructor rule.
    assert_eq!(
        in_edition(
            "0.4",
            "kind Box\n  value: Text\n\
             \n\
             pub boxed(token: secret Text) -> Box = Box(value: token)\n\
             \n\
             pub branched(c: secret Bool) -> Json = if c then Str(\"a\") else Null\n\
             \n\
             pub branched_ok(c: secret Bool) -> secret Json = if c then Str(\"a\") else Null\n"
        ),
        [
            "U4001 src/t.una:6:51 field `value` of `Box` is `Text`, and this value is `secret Text`: a flow from `secret` to `public`",
            "U4001 src/t.una:8:43 the result of `branched` is `public`, and which value it returns depends on this `secret` condition: a flow from `secret` to `public`",
        ]
    );
}
