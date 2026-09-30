//! Types (M4): kinds and written types, expressions, calls, constructors, `when`, statements,
//! the positions of `result`, `it` and `error`, the refinement language, predicates, lambdas,
//! generics, and pending obligations (`spec/03`, `spec/04`, `spec/06`, `spec/10`, ADR 0024).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use pretty_assertions::assert_eq;
use una_check::{ModuleInput, PackageInput, TypeCheck, check_names, check_types};
use una_diag::{Code, Diagnostic, Domain};
use una_project::{Manifest, parse_manifest};
use una_syntax::ast::DeclKind;
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

/// Whether `code` belongs to a later milestone than M4: the authority codes and the escape
/// budget of M5, which `authority.rs` tests, and the security codes of M6, which `flow.rs`
/// tests.
fn later(code: Code) -> bool {
    matches!(code.domain(), Domain::Authority | Domain::Security) || code == Code::U7003
}

/// The name and type diagnostics of `files`, as "code file:line:col message" in output order,
/// without the codes of later milestones.
fn check(files: &[(&str, &str)], single: bool) -> Vec<String> {
    let (names, types) = run(files, single);
    let mut diagnostics: Vec<Diagnostic> = names
        .into_iter()
        .chain(types.diagnostics)
        .filter(|d| !later(d.code))
        .collect();
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
fn types(body: &str) -> Vec<String> {
    check(&[("t.una", &format!("module t\n\n{body}"))], true)
}

/// A package with `una.toml` and the given modules.
fn package(modules: &[(&str, &str)]) -> Vec<String> {
    let mut files = vec![("una.toml", MANIFEST)];
    files.extend_from_slice(modules);
    check(&files, false)
}

/// The repairs of the diagnostics of a single file, as "code: description [edit]".
fn repairs(body: &str) -> Vec<String> {
    let (_, types) = run(&[("t.una", &format!("module t\n\n{body}"))], true);
    types
        .diagnostics
        .iter()
        .flat_map(|d| {
            d.repairs.iter().map(move |repair| match &repair.edit {
                Some(edit) => format!(
                    "{}: {} [{}:{} -> {:?}]",
                    d.code,
                    repair.description,
                    edit.location.start.line,
                    edit.location.start.col,
                    edit.replacement
                ),
                None => format!("{}: {}", d.code, repair.description),
            })
        })
        .collect()
}

/// The pending obligations of a single file, as "kind line:col", and where selfified `let`s
/// bind their names; the file must type-check.
fn obligations(body: &str) -> (Vec<String>, Vec<String>) {
    let (names, types) = run(&[("t.una", &format!("module t\n\n{body}"))], true);
    assert_eq!(names, Vec::new(), "name diagnostics");
    assert_eq!(types.diagnostics, Vec::new(), "type diagnostics");
    let obligations = types
        .obligations
        .iter()
        .map(|o| {
            format!(
                "{} {}:{}",
                o.kind.as_str(),
                o.location.start.line,
                o.location.start.col
            )
        })
        .collect();
    let selfified = types
        .selfified
        .iter()
        .map(|l| format!("{}:{}", l.start.line, l.start.col))
        .collect();
    (obligations, selfified)
}

fn std_source(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/std")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

// ----- the prelude and std.testing (spec/14) -----

#[test]
fn the_standard_modules_type_check_as_ordinary_modules() {
    for name in ["prelude.una", "testing.una"] {
        let text = std_source(name);
        // ADR 0038 D7: checked as a user module, a standard module's path is reserved
        // (U1009), and nothing else is reported.
        let found = check(&[(name, &text)], true);
        assert!(
            found.len() == 1 && found[0].starts_with("U1009 "),
            "{name}: {found:?}"
        );
    }
}

#[test]
fn the_prelude_declares_exactly_the_names_of_spec_14() {
    let spec = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/spec/14-standard-library.md"),
    )
    .unwrap();
    let section = |heading: &str| {
        spec.split(heading)
            .nth(1)
            .unwrap()
            .split("\n## ")
            .next()
            .unwrap()
            .to_owned()
    };
    // Functions: the first column of the table of §1, up to `(` or `[`.
    let functions: BTreeSet<String> = section("## 1. Prelude")
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .map(|rest| rest.split(['(', '[']).next().unwrap().to_owned())
        .collect();
    // Capabilities: the first column of the table of §3.
    let capabilities: BTreeSet<String> = section("## 3. Host capabilities")
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .map(|rest| rest.split('`').next().unwrap().to_owned())
        .collect();
    let kinds: BTreeSet<String> = ["Nat", "Option", "Result", "FileError", "KeyError"]
        .into_iter()
        .map(str::to_owned)
        .collect();

    let text = std_source("prelude.una");
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let id = db.add("prelude.una", text.as_bytes());
    let lexed = lex(db.file(id), id, &mut interner);
    let parsed = parse(&lexed, id, &interner);
    let (mut declared_functions, mut declared_capabilities, mut declared_kinds) =
        (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for decl in &parsed.file.decls {
        match &decl.kind {
            DeclKind::Function(f) => {
                assert!(f.public);
                declared_functions.insert(interner.resolve(f.name.name).to_owned());
            }
            DeclKind::Capability(c) => {
                assert!(c.public);
                declared_capabilities.insert(interner.resolve(c.name.name).to_owned());
            }
            DeclKind::Kind(k) => {
                assert!(k.public);
                declared_kinds.insert(interner.resolve(k.name.name).to_owned());
            }
            other => panic!("unexpected declaration {other:?}"),
        }
    }
    assert_eq!(declared_functions, functions);
    assert_eq!(declared_capabilities, capabilities);
    assert_eq!(declared_kinds, kinds);
    assert_eq!(functions.len(), 20);
}

// ----- kinds and written types (U2002, U2003) -----

#[test]
fn a_refined_type_argument_is_named_as_written_n010() {
    // N-010: a type argument names the refined kind written, not the kind or range its alias
    // goes through; range sugar on the argument itself names the refined type.
    assert_eq!(
        types(
            "kind Small = Int in 0..5\n\
             \n\
             kind S2 = Small\n\
             \n\
             kind Pos = Int where it > 0\n\
             \n\
             kind P2 = Pos\n\
             \n\
             f(a: List[S2], b: List[P2], c: List[Int in 0..5], d: List[Small]) -> Int = 0\n"
        ),
        [
            "U2002 t.una:11:11 this type argument is the refined kind `S2`, and a refined type cannot instantiate a type parameter in v0.1; write its base type `Int` and check the refinement where the value is used",
            "U2002 t.una:11:24 this type argument is the refined kind `P2`, and a refined type cannot instantiate a type parameter in v0.1; write its base type `Int` and check the refinement where the value is used",
            "U2002 t.una:11:37 this type argument is the refined type `Int in 0..5`, and a refined type cannot instantiate a type parameter in v0.1; write its base type `Int` and check the refinement where the value is used",
            "U2002 t.una:11:59 this type argument is the refined kind `Small`, and a refined type cannot instantiate a type parameter in v0.1; write its base type `Int` and check the refinement where the value is used",
        ]
    );
}

#[test]
fn written_types_name_types_with_their_arguments() {
    assert_eq!(
        types(
            "kind Shape = Circle(radius: Int) | Dot\n\
             \n\
             kind Point\n  x: Int\n  y: Int\n\
             \n\
             f(a: Circle) -> Int = 0\n\
             \n\
             g(a: Console) -> Int = 0\n\
             \n\
             h(a: Text in 0..9) -> Int = 0\n\
             \n\
             i(a: List) -> Int = 0\n\
             \n\
             j(a: Point[Int], b: Option[Int, Int], c: Int[Text]) -> Int = 0\n\
             \n\
             k() -> Int\n  fails Point\n=\n  0\n\
             \n\
             m() -> Int\n  fails Option\n=\n  0\n"
        ),
        [
            "U2002 t.una:9:6 `Circle` is a variant of `Shape`, not a type",
            "U2002 t.una:11:6 `Console` is a capability, not a type; capabilities are not values",
            "U2002 t.una:13:14 range sugar applies only to `Int`",
            "U2003 t.una:15:6 type `List` takes 1 type argument, but none is given",
            "U2003 t.una:17:6 type `Point` takes no type arguments, but 1 is given",
            "U2003 t.una:17:21 type `Option` takes 1 type argument, but 2 are given",
            "U2003 t.una:17:42 type `Int` takes no type arguments, but 1 is given",
            "U2002 t.una:20:9 `fails` names a sum kind, and `Point` is a record",
            "U2002 t.una:25:9 error kind `Option` has type parameters, which `fails` cannot give",
        ]
    );
}

#[test]
fn a_kind_contains_itself_only_through_list_or_option() {
    // Through any other kind, a type argument or a function type, it is U2002.
    assert_eq!(
        types(
            "kind Node\n  next: Node\n\
             \n\
             kind Tree = Leaf | Branch(left: Tree, right: Tree)\n\
             \n\
             kind A\n  b: B\n\
             \n\
             kind B\n  a: A\n\
             \n\
             kind Pair[X, Y]\n  first: X\n  second: Y\n\
             \n\
             kind Wrapped\n  inner: Pair[Wrapped, Int]\n\
             \n\
             kind Deferred\n  make: fn() -> Deferred\n\
             \n\
             kind Fine\n  children: List[Fine]\n  parent: Option[Fine]\n  pairs: List[Pair[Fine, Fine]]\n\
             \n\
             kind Alias1 = Alias2\n\
             \n\
             kind Alias2 = List[Alias1]\n"
        ),
        [
            "U2002 t.una:4:9 kind `Node` contains itself through field `next`; a kind may contain itself only through `List` or `Option`",
            "U2002 t.una:6:33 kind `Tree` contains itself through field `left`; a kind may contain itself only through `List` or `Option`",
            "U2002 t.una:9:6 kind `A` contains itself through field `b` (through `B`); a kind may contain itself only through `List` or `Option`",
            "U2002 t.una:19:10 kind `Wrapped` contains itself through field `inner`; a kind may contain itself only through `List` or `Option`",
            "U2002 t.una:22:9 kind `Deferred` contains itself through field `make`; a kind may contain itself only through `List` or `Option`",
            "U2002 t.una:31:20 kind `Alias1` is defined in terms of itself",
        ]
    );
    assert_eq!(
        repairs("kind Node\n  next: Node\n"),
        ["U2002: make the field optional: `Option[Node]` [4:9 -> \"Option[Node]\"]"]
    );
}

// ----- expressions (U2001) -----

#[test]
fn mismatches_name_the_expected_and_found_types() {
    assert_eq!(
        types(
            "kind Point\n  x: Int\n  y: Int\n\
             \n\
             f(a: Int) -> Text = a\n\
             \n\
             g(p: Point) -> Int = if p.x > 0 then \"x\" else p.y\n\
             \n\
             h(a: Int) -> Int\n=\n  let b: Text = a\n  a\n\
             \n\
             i(a: Int) -> Bool = a and true\n\
             \n\
             j(a: Text) -> Int = a + 1\n\
             \n\
             k(a: Int, b: Text) -> Bool = a == b\n\
             \n\
             m(a: Int) -> Int = -true\n"
        ),
        [
            "U2001 t.una:7:21 `f` returns `Text`, found `Int`",
            "U2001 t.una:9:38 `g` returns `Int`, found `Text`",
            "U2001 t.una:13:17 `b` is declared as `Text`, found `Int`",
            "U2001 t.una:16:21 an operand of `and` must be `Bool`, found `Int`",
            "U2001 t.una:18:21 an operand of `+` must be `Int`, found `Text`",
            "U2001 t.una:20:30 `==` needs operands of one type, and these are `Int` and `Text`",
            "U2001 t.una:22:21 the operand of `-` must be `Int`, found `Bool`",
        ]
    );
}

#[test]
fn equality_ordering_and_interpolation_follow_spec_03() {
    assert_eq!(
        types(
            "kind Shape = Circle(radius: Int) | Dot\n\
             \n\
             kind Level = Low | High\n\
             \n\
             a(f: fn(Int) -> Int) -> Bool = f == f\n\
             \n\
             b(x: Shape, y: Shape) -> Bool = x < y\n\
             \n\
             c(x: Level, y: Level, s: Text, t: Text) -> Bool = x < y and s < t\n\
             \n\
             d(h: Hashed) -> Text = \"h {h}\"\n\
             \n\
             e(h: Hashed, g: Hashed) -> Bool = h == g\n\
             \n\
             g(h: Hashed, g: Hashed) -> Bool = h < g\n\
             \n\
             i(n: Int, b: Bool, t: Text) -> Text = \"{n} {b} {t}\"\n"
        ),
        [
            "U2001 t.una:7:32 `==` cannot compare values of `fn(Int) -> Int`, which hold functions",
            "U2001 t.una:9:33 `<` orders `Int`, `Text` and variants without fields, not `Shape`",
            "U2001 t.una:13:28 interpolation converts `Int`, `Bool` and `Text` to text, and this value is `Hashed`",
            "U2001 t.una:17:35 `<` orders `Int`, `Text` and variants without fields, not `Hashed`",
        ]
    );
}

#[test]
fn a_type_that_inference_leaves_unknown_is_reported() {
    assert_eq!(
        types(
            "a() -> Int\n=\n  let xs = []\n  0\n\
             \n\
             b() -> Bool = is_empty([])\n\
             \n\
             c() -> Int\n=\n  let o = None\n  0\n\
             \n\
             d() -> Int\n=\n  let xs: List[Int] = []\n  let o: Option[Text] = None\n  0\n"
        ),
        [
            "U2001 t.una:5:12 the element type of this empty list cannot be inferred; give it a type, as in `let xs: List[Int] = []`",
            "U2001 t.una:8:15 type parameter `T` of `is_empty` cannot be inferred here; give the value a type, for example with an annotated `let`",
            "U2001 t.una:12:11 type parameter `T` of `Option` cannot be inferred here; give the value a type, for example with an annotated `let`",
        ]
    );
}

// ----- calls (U2004 to U2006, `spec/04 §3`) -----

#[test]
fn calls_label_every_argument_after_the_first() {
    assert_eq!(
        types(
            "a(x: Int) -> Int = max(x, 1)\n\
             \n\
             b(x: Int) -> Int = max(x, c: 1)\n\
             \n\
             c(x: Int) -> Int = max(a: x, a: 1)\n\
             \n\
             d(x: Int) -> Int = max(x)\n\
             \n\
             e(x: Int) -> Int = max(x, b: 1, 2)\n\
             \n\
             f(a: Int, b: Int) -> Int = max(b, a)\n\
             \n\
             g(x: Int) -> Int = max(b: 1, a: x)\n\
             \n\
             h(x: Int) -> Int = abs(x: x)\n"
        ),
        [
            "U2004 t.una:3:27 argument 2 of `max` needs a label; only the first argument may go without one",
            "U2005 t.una:5:27 `max` has no parameter `c`",
            "U2006 t.una:7:30 parameter `a` of `max` is given twice",
            "U2001 t.una:9:20 `max` is missing argument `b`",
            "U2001 t.una:11:20 `max` takes 2 arguments, but 3 are given",
        ]
    );
    assert_eq!(
        repairs("a(x: Int) -> Int = max(x, 1)\n\nb(x: Int) -> Int = max(x, bb: 1)\n"),
        [
            "U2004: label it `b` [3:27 -> \"b: 1\"]",
            "U2005: write `b` [5:27 -> \"b\"]",
        ]
    );
}

#[test]
fn function_values_take_positional_arguments() {
    assert_eq!(
        types(
            "a(f: fn(Int, Int) -> Int) -> Int = f(1, 2)\n\
             \n\
             b(f: fn(Int) -> Int) -> Int = f(x: 1)\n\
             \n\
             c(f: fn(Int) -> Int) -> Int = f(1, 2)\n\
             \n\
             d(n: Int) -> Int = n(1)\n\
             \n\
             e(f: fn(Int) -> Int) -> Text = f(1)\n"
        ),
        [
            "U2005 t.una:5:33 `f` is a function value, which takes its arguments without labels",
            "U2001 t.una:7:31 `f` takes 1 argument, but 2 are given",
            "U2001 t.una:9:20 `n` is not a function; it has type `Int`",
            "U2001 t.una:11:32 `e` returns `Text`, found `Int`",
        ]
    );
}

#[test]
fn fields_and_methods_resolve_by_type() {
    assert_eq!(
        types(
            "kind Point\n  x: Int\n  y: Int\n\
             \n\
             kind Box[T]\n  item: T\n  label: Text\n\
             \n\
             kind Tool\n  run: fn(Int) -> Int\n\
             \n\
             norm(p: Point) -> Int = abs(p.x) + p.y.abs\n\
             \n\
             unwrap[T](b: Box[T]) -> T = b.item\n\
             \n\
             size(b: Box[Int]) -> Int = b.item + b.label.len + b.unwrap()\n\
             \n\
             apply(t: Tool) -> Int = t.run(2)\n\
             \n\
             bigger(xs: List[Int]) -> List[Int] = xs.map(each: fn(x) -> x * 2).filter(keep: fn(x) -> x > 2)\n\
             \n\
             wrong(p: Point) -> Int = p.z\n\
             \n\
             method(p: Point) -> Int = p.zz()\n\
             \n\
             arity(p: Point) -> Int = p.max\n"
        ),
        [
            "U1001 t.una:24:28 `Point` has no field `z`, and no function `z` in scope applies",
            "U1001 t.una:26:29 `Point` has no field or function `zz`, and no function `zz` in scope applies",
            "U2001 t.una:28:28 `max` takes 2 arguments, so `.max` needs them, as in `.max(...)`",
        ]
    );
}

#[test]
fn policies_take_one_argument_and_check_their_via_function() {
    assert_eq!(
        package(&[(
            "src/k.una",
            "trusted module k\n\
             \n\
             pub policy digest\n  from secret Text\n  to internal Hashed\n  via hash_it\n  purpose \"test\"\n  owner team\n\
             \n\
             pub policy wrong\n  from Text\n  to Int\n  via hash_it\n  purpose \"test\"\n  owner team\n\
             \n\
             hash_it(t: secret Text) -> internal Hashed = hash_it(t)\n\
             \n\
             use_it(t: Text) -> Hashed = digest(t)\n\
             \n\
             labeled(t: Text) -> Hashed = digest(v: t)\n\
             \n\
             value() -> Int = digest\n",
        )]),
        [
            "U7005 src/k.una:1:1 module `k` is `trusted` in its header but not listed in una.toml [trusted] modules",
            "U2005 src/k.una:21:37 policy `digest` takes one argument without a label",
            "U2001 src/k.una:23:18 policy `digest` is not a value; call it as `digest(value)`",
        ]
    );
}

// ----- constructors (U2009, `spec/04 §3` rule 4) -----

#[test]
fn constructors_label_their_fields() {
    // The unlabeled argument of a constructor with several fields.
    assert_eq!(
        types(
            "kind Point\n  x: Int\n  y: Int\n\
             \n\
             kind Shape = Circle(radius: Int) | Dot\n\
             \n\
             a() -> Point = Point(y: 2, x: 1)\n\
             \n\
             b(x: Int, y: Int) -> Point = Point(x, y)\n\
             \n\
             c() -> Point = Point(x: 1, z: 2)\n\
             \n\
             d() -> Point = Point(x: 1)\n\
             \n\
             e() -> Point = Point(1, 2)\n\
             \n\
             f() -> Shape = Circle(3)\n\
             \n\
             g() -> Shape = Circle(radius: 1, radius: 2)\n\
             \n\
             h() -> Shape = Dot()\n\
             \n\
             i() -> Shape = Circle\n\
             \n\
             j() -> Point = Point\n\
             \n\
             k() -> Option[Int] = Some(1, 2)\n"
        ),
        [
            "U2009 t.una:13:28 `Point` has no field `z`",
            "U2009 t.una:15:16 `Point` is missing field `y`",
            "U2004 t.una:17:22 `Point` has 2 fields, so every argument needs its field's label",
            "U2004 t.una:17:25 `Point` has 2 fields, so every argument needs its field's label",
            "U2006 t.una:21:34 field `radius` of `Circle` is given twice",
            "U2009 t.una:23:16 variant `Dot` has no fields; write `Dot`",
            "U2009 t.una:25:16 variant `Circle` needs its fields: `radius`",
            "U2009 t.una:27:16 record `Point` needs its fields: `x`, `y`",
            "U2009 t.una:29:30 `Some` has 1 field, but 2 are given",
        ]
    );
}

#[test]
fn unknown_label_repairs_suggest_only_what_is_not_given() {
    // R-030: an unknown label close to a parameter or field that another argument already
    // gives suggests one that is still missing, so applying the repair cannot give it twice
    // (U2006); when nothing is missing, the advice says so.
    assert_eq!(
        repairs(
            "kind Point\n  value: Int\n  values: Int\n\
             \n\
             kind Shape = Rect(width: Int, widths: Int) | Dot\n\
             \n\
             pick(value: Int, values: Int) -> Int = value\n\
             \n\
             a(x: Int) -> Int = pick(value: x, valuex: 1)\n\
             \n\
             b(x: Int) -> Point = Point(value: 1, valuex: 2)\n\
             \n\
             c(s: Shape) -> Int\n=\n  when s\n    is Rect(width, widthx: h) -> h\n    else -> 0\n\
             \n\
             d() -> Point = Point(value: 1, values: 2, valuex: 3)\n"
        ),
        [
            "U2005: write `values` [11:35 -> \"values\"]",
            "U2009: write `values` [13:38 -> \"values\"]",
            "U2009: write `widths` [18:20 -> \"widths\"]",
            "U2009: every field is given by another argument; remove this one",
        ]
    );
}

// ----- when and patterns (U2007, U2008, `spec/10 §4`) -----

#[test]
fn when_over_a_sum_is_exhaustive_and_every_arm_is_reachable() {
    assert_eq!(
        types(
            "kind Level = Low | Mid | High\n\
             \n\
             a(l: Level) -> Int\n=\n  when l\n    is Low -> 0\n\
             \n\
             b(l: Level) -> Int\n=\n  when l\n    else -> 0\n    is Low -> 1\n\
             \n\
             c(l: Level) -> Int\n=\n  when l\n    is Low -> 0\n    is Low -> 1\n    else -> 2\n\
             \n\
             d(l: Level) -> Int\n=\n  when l\n    is Low -> 0\n    is Mid -> 1\n    is High -> 2\n    else -> 3\n\
             \n\
             e(b: Bool) -> Int\n=\n  when b\n    is true -> 0\n    is true -> 1\n    is false -> 2\n\
             \n\
             f(l: Level) -> Int\n=\n  when l\n    is Low -> 0\n    else -> 1\n"
        ),
        [
            "U2007 t.una:7:3 `when` does not match variants `Mid`, `High` of `Level`",
            "U2008 t.una:14:5 this arm follows the `else` arm, so no value reaches it; `else` must be the last arm",
            "U2008 t.una:20:5 variant `Low` is already matched by an earlier arm",
            "U2008 t.una:29:5 no value reaches this `else`: the arms above match every variant of `Level`",
            "U2008 t.una:35:5 this literal is already matched by an earlier arm",
        ]
    );
}

#[test]
fn patterns_bind_the_fields_of_their_variant() {
    assert_eq!(
        types(
            "kind Shape = Circle(radius: Int) | Rect(width: Int, height: Int) | Dot\n\
             \n\
             a(s: Shape) -> Int\n=\n  when s\n    is Circle(r) -> r\n    is Rect(width, height: h) -> width * h\n    is Dot -> 0\n\
             \n\
             b(s: Shape) -> Int\n=\n  when s\n    is Rect(depth) -> 0\n    is Dot(x) -> 1\n    else -> 2\n\
             \n\
             c(s: Shape) -> Int\n=\n  when s\n    is Rect(width, width: w) -> w\n    else -> 0\n\
             \n\
             d(n: Int) -> Int\n=\n  when n\n    is Dot -> 0\n    is Circle -> 1\n\
             \n\
             e(s: Shape) -> Int\n=\n  when s\n    in 0..9 -> 0\n    else -> 1\n"
        ),
        [
            "U2009 t.una:15:13 variant `Rect` has no field `depth`",
            "U2009 t.una:16:12 variant `Dot` has no fields to bind",
            "U2006 t.una:22:20 field `width` of `Rect` is bound twice",
            "U2001 t.una:27:8 `when` with variant arms examines a value of a sum kind, and this value is `Int`",
            "U2001 t.una:34:5 a range arm applies to `Int`, and `when` examines a `Shape` value",
        ]
    );
}

// ----- statements and handling clauses (U2011, `spec/06 §3`) -----

#[test]
fn statements_before_the_last_are_unit() {
    assert_eq!(
        types(
            "a(x: Int) -> Int\n=\n  x + 1\n  x\n\
             \n\
             b(x: Int) -> Int\n=\n  let y = x\n  y\n\
             \n\
             c(x: Int) -> Unit\n=\n  fail Oops\n  unit\n"
        ),
        [
            "U2011 t.una:5:3 this statement has type `Int`, and only `Unit` values can be discarded; bind the value with `let`",
            "U1001 t.una:15:8 cannot find variant `Oops` in this module, its imports or the prelude",
        ]
    );
}

#[test]
fn handling_clauses_type_their_values() {
    assert_eq!(
        types(
            "kind StoreError = Timeout | Corrupted\n\
             \n\
             kind CheckInError = Missing | Unavailable(reason: Text)\n\
             \n\
             capability Store\n  op find(key: Int) -> Option[Int]\n    fails StoreError\n  op put(key: Int) -> Unit\n    fails StoreError\n\
             \n\
             a(key: Int) -> Int\n  uses @store: Store\n  fails CheckInError\n=\n  let v = @store.find(key)\n    if error fail Unavailable(reason: \"store\")\n    if none fail Missing\n  v\n\
             \n\
             b(key: Int) -> Int\n  uses @store: Store\n=\n  let v = @store.find(key)\n    if error use None\n    if none use 0\n  v\n\
             \n\
             c(key: Int) -> Int\n  uses @store: Store\n  fails CheckInError\n=\n  @store.put(key)\n    if error fail Unavailable(reason: error)\n  0\n\
             \n\
             d(key: Int) -> Int\n  uses @store: Store\n=\n  let v = @store.put(key)\n    if error use unit\n    if none use 0\n  v\n\
             \n\
             e(key: Int) -> Bool\n  uses @store: Store\n=\n  let r = attempt @store.find(key)\n  when r\n    is Ok(found) -> found.is_some()\n    is Err(failure) -> failure == Timeout\n\
             \n\
             f(key: Int) -> Int\n=\n  let r = attempt (key + 1)\n  0\n\
             \n\
             g(raw: Text) -> Text\n  fails CheckInError\n=\n  checked(raw, trim(it) == it) else fail Missing\n\
             \n\
             h(raw: Text) -> Text\n  fails CheckInError\n=\n  checked(raw, it.len > 0) else fail Missing\n"
        ),
        [
            "U2001 t.una:35:39 field `reason` of `Unavailable` must be `Text`, found `StoreError`",
            "U2001 t.una:41:11 `if none` applies to an `Option` value, and this one is `Unit`",
            "U2001 t.una:56:19 `attempt` applies to one call of a function or capability operation",
            "U2014 t.una:62:16 the call of function `trim` is outside the refinement language; the refinement language calls only predicates",
        ]
    );
}

// ----- positions of result, it and error (U2012, U2013) -----

#[test]
fn result_it_and_error_appear_only_where_allowed() {
    assert_eq!(
        types(
            "kind Small = Int where it < 10\n\
             \n\
             a(x: Int) -> Int\n  requires result > 0\n  ensures result > x\n=\n  result\n\
             \n\
             b(x: Int) -> Int = it\n\
             \n\
             c(x: Int) -> Int = error\n\
             \n\
             example \"result in expect\"\n  do a(1)\n  expect result > 1\n\
             \n\
             example \"result in do\"\n  do result\n  expect true\n"
        ),
        [
            "U2012 t.una:6:12 `result` is allowed only in `ensures` clauses and `expect` lines",
            "U2012 t.una:9:3 `result` is allowed only in `ensures` clauses and `expect` lines",
            "U2013 t.una:11:20 `it` is allowed only in `where` clauses and the predicate of `checked`",
            "U2013 t.una:13:20 `error` is allowed only in the `fail` or `use` part of an `if error` clause",
            "U2012 t.una:20:6 `result` is allowed only in `ensures` clauses and `expect` lines",
        ]
    );
}

// ----- the refinement language (U2014, `spec/03 §3`) -----

#[test]
fn clauses_are_in_the_refinement_language() {
    assert_eq!(
        types(
            "kind Level = Low | High\n\
             \n\
             kind Record\n  count: Int\n  name: Text\n  items: List[Int]\n\
             \n\
             predicate reflect positive(x: Int) -> Bool = x > 0\n\
             \n\
             good(r: Record, l: Level, t: Text) -> Int\n  requires positive(r.count) and r.items.len > 0 and 2 * r.count - 1 >= r.count / 2\n  requires l is Low implies r.name != \"\" and r.count mod 3 == 0 and r.count in 0..9\n  requires not (t == \"x\") and -r.count < 0\n  ensures result >= 0\n=\n  1\n\
             \n\
             bad(a: Int, b: Int, t: Text, u: Text, l: Level) -> Int\n  requires abs(a) > 0\n  requires a * b > 0 and a / b > 0\n  requires (if a > 0 then b else a) > 0\n  requires t < u and \"{a}\" == t\n  requires l == Low\n  requires a is 1\n=\n  0\n"
        ),
        [
            "U2014 t.una:21:12 the call of function `abs` is outside the refinement language; the refinement language calls only predicates",
            "U2014 t.una:22:12 `*` of two operands that are not literals is outside the refinement language",
            "U2014 t.una:22:26 `/` by a divisor that is not a nonzero literal is outside the refinement language",
            "U2014 t.una:23:13 `if ... then ... else` is outside the refinement language",
            "U2014 t.una:24:12 ordering of `Text` is outside the refinement language",
            "U2014 t.una:24:22 text interpolation is outside the refinement language",
            "U2014 t.una:25:12 comparing with variant `Low` is outside the refinement language",
            "U2014 t.una:26:12 `is` with a literal is outside the refinement language",
        ]
    );
    assert_eq!(
        repairs(
            "kind Level = Low | High\n\nf(l: Level, a: Int) -> Int\n  requires l == Low\n  requires a is 1\n=\n  0\n"
        ),
        [
            "U2014: write `l is Low` [6:12 -> \"l is Low\"]",
            "U2014: write `a == 1` [7:12 -> \"a == 1\"]",
        ]
    );
}

// ----- predicates (U2015, `spec/03 §4`) -----

#[test]
fn predicates_are_pure_total_and_not_recursive() {
    // `helper` calls an opaque predicate outside `checked` (ADR 0042).
    assert_eq!(
        types(
            "kind Failure = | Oops\n\
             \n\
             capability Log\n  op write(line: Text) -> Unit\n\
             \n\
             predicate reflect small(x: Int) -> Bool = abs(x) < 10\n\
             \n\
             predicate reflect again(x: Int) -> Bool = again(x)\n\
             \n\
             predicate reflect ping(x: Int) -> Bool = pong(x)\n\
             \n\
             predicate reflect pong(x: Int) -> Bool = ping(x)\n\
             \n\
             predicate opaque via_function(x: Int) -> Bool = helper(x)\n\
             \n\
             helper(x: Int) -> Bool = via_function(x)\n\
             \n\
             predicate opaque failing(x: Int) -> Bool = fail Oops\n\
             \n\
             predicate opaque checking(t: Text) -> Bool =\n  let c = checked(t, it.len > 0) else fail Oops\n  c == t\n\
             \n\
             predicate opaque calls_failing(x: Int) -> Bool = risky(x)\n\
             \n\
             predicate opaque handled(x: Int) -> Bool =\n  let r = attempt risky(x)\n  r is Ok\n\
             \n\
             risky(x: Int) -> Bool\n  fails Failure\n=\n  true\n\
             \n\
             predicate opaque number(x: Int) -> Int = x\n"
        ),
        [
            "U2015 t.una:8:43 the call of function `abs` is outside the refinement language, to which `reflect` predicates are limited; the refinement language calls only predicates",
            "U2015 t.una:10:19 reflect predicate `again` calls itself; predicates are not recursive",
            "U2015 t.una:12:19 reflect predicate `ping` calls itself through `pong`; predicates are not recursive",
            "U2015 t.una:16:18 opaque predicate `via_function` calls itself through `helper`; predicates are not recursive",
            "U2016 t.una:18:26 opaque predicate `via_function` is called outside `checked`, so its fact would bypass the escape budget",
            "U2015 t.una:20:44 `fail` in the body of a predicate, which must be pure and cannot fail",
            "U2015 t.una:23:11 `checked`, which can fail, in the body of a predicate, which must be pure and cannot fail",
            "U2015 t.una:26:50 call of `risky`, which can fail, in the body of a predicate, which must be pure and cannot fail",
            "U2001 t.una:37:36 a predicate returns `Bool`, and `number` returns `Int`",
        ]
    );
}

// ----- lambdas (U2010, `spec/03 §7`, `spec/04 §6`) -----

#[test]
fn lambda_parameters_take_their_types_from_the_expected_function_type() {
    assert_eq!(
        types(
            "a(xs: List[Int]) -> Int = fold(xs, initial: 0, step: fn(acc, x) -> acc + x)\n\
             \n\
             b() -> Int\n=\n  let f = fn(x) -> x + 1\n  1\n\
             \n\
             c() -> Int\n=\n  let f = fn(x: Int) -> x + 1\n  f(2)\n\
             \n\
             d(xs: List[Int]) -> List[Int] = xs.map(each: fn(a, b) -> a)\n\
             \n\
             e(xs: List[Int]) -> List[Text] = xs.map(each: fn(x: Text) -> x)\n\
             \n\
             f(names: List[Text]) -> List[Int] = map(each: fn(x) -> x.len, xs: names)\n\
             \n\
             g() -> Bool = is_some(first_where([], keep: fn(x) -> x.len > 0))\n"
        ),
        [
            "U2010 t.una:7:14 cannot infer the type of lambda parameter `x`; annotate it, as in `x: Int`",
            "U2001 t.una:15:46 expected a function of 1 parameter, found a lambda of 2 parameters",
            "U2001 t.una:17:53 lambda parameter `x` is declared as `Text`, and the expected function type gives it `Int`",
            "U2001 t.una:21:54 the type of this value is not known here; annotate it, for example the lambda parameter or the `let` it comes from",
        ]
    );
}

// ----- generics (`spec/03 §2` rule 4) -----

#[test]
fn type_parameters_are_rigid_and_compared_values_hold_no_functions() {
    assert_eq!(
        types(
            "kind Box[T]\n  item: T\n\
             \n\
             a[T](x: T) -> Int = x\n\
             \n\
             same[T](x: T, y: T) -> Bool = x == y\n\
             \n\
             indirect[U](x: U) -> Bool = same(x, y: x)\n\
             \n\
             b(f: fn(Int) -> Int) -> Bool = indirect(f)\n\
             \n\
             c(fs: List[fn(Int) -> Int], f: fn(Int) -> Int) -> Bool = contains(fs, item: f)\n\
             \n\
             d(b: Box[fn(Int) -> Int]) -> Bool = b == b\n\
             \n\
             e(b: Box[Int]) -> Bool = b == b and first([1]) == Some(1)\n"
        ),
        [
            "U2001 t.una:6:21 `a` returns `Int`, found `T`",
            "U2001 t.una:12:32 `indirect` compares values of its type parameter `U` with `==`, so `U` cannot be a type that holds functions",
            "U2001 t.una:14:58 `contains` compares values of its type parameter `T` with `==`, so `T` cannot be a type that holds functions",
            "U2001 t.una:16:37 `==` cannot compare values of `Box[fn(Int) -> Int]`, which hold functions",
        ]
    );
}

// ----- variants in context (ADR 0024 rule 1) -----

#[test]
fn variants_take_their_kind_from_the_context() {
    assert_eq!(
        package(&[
            (
                "src/a.una",
                "module a\n\
                 \n\
                 pub kind Level = Low | High\n\
                 \n\
                 pub kind Failure = Broken | Lost\n\
                 \n\
                 pub level(x: Int) -> Level = if x > 0 then High else Low\n\
                 \n\
                 pub risky(x: Int) -> Int\n  fails Failure\n=\n  if x > 0 then x else fail Broken\n",
            ),
            (
                "src/b.una",
                "module b\n\
                 \n\
                 import a (level, risky)\n\
                 \n\
                 describe(x: Int) -> Text\n=\n  when level(x)\n    is Low -> \"low\"\n    is High -> \"high\"\n\
                 \n\
                 low(x: Int) -> Bool = level(x) is Low\n\
                 \n\
                 wrong(x: Int) -> Bool = level(x) is Medium\n\
                 \n\
                 example \"fails\"\n  do risky(0)\n  expect fails Broken\n\
                 \n\
                 example \"fails with another variant\"\n  do risky(0)\n  expect fails Vanished\n",
            ),
        ]),
        [
            "U1001 src/b.una:13:37 `Level` has no variant `Medium`",
            "U1001 src/b.una:21:16 cannot find variant `Vanished` in `Failure`",
        ]
    );
}

#[test]
fn a_variant_that_two_imported_kinds_share_is_chosen_by_type() {
    assert_eq!(
        package(&[
            ("src/a.una", "module a\n\npub kind Color = Red | Green\n"),
            ("src/b.una", "module b\n\npub kind Light = Red | Off\n"),
            (
                "src/c.una",
                "module c\n\
                 \n\
                 import a (Color)\n\
                 import b (Light)\n\
                 \n\
                 pick() -> Color = Red\n\
                 \n\
                 other() -> Light = Red\n\
                 \n\
                 unknown() -> Int\n=\n  let x = Red\n  0\n",
            ),
        ]),
        ["U1001 src/c.una:12:11 `Red` is ambiguous: it is a variant of `Color` and `Light`"]
    );
}

// ----- capability operations -----

#[test]
fn capability_operations_take_their_types_from_the_bound_capability() {
    assert_eq!(
        types(
            "capability Store\n  op put(key: Int, value: Text) -> Unit\n\
             \n\
             handler memory() for Store\n  op put(key, value) = unit\n\
             \n\
             handler wrong() for Store\n  op put(key, value) = key\n\
             \n\
             a() -> Unit\n  uses @store: Store[put]\n=\n  @store.put(1, value: \"x\")\n\
             \n\
             b() -> Unit\n  uses @store: Store[put]\n=\n  @store.put(1, value: 2)\n\
             \n\
             c() -> Unit\n  uses @store: Store[put]\n=\n  @store.pt(1, value: \"x\")\n\
             \n\
             d() -> Unit\n  uses @store: Store[put]\n=\n  a() with @store = memory()\n\
             \n\
             example \"uses a handler\"\n  given @store = memory()\n  do @store.put(1, value: \"x\")\n  expect result == unit\n"
        ),
        [
            "U2001 t.una:10:24 `put` returns `Unit`, found `Int`",
            "U2001 t.una:20:24 argument `value` of operation `Store.put` must be `Text`, found `Int`",
            "U1001 t.una:25:10 capability `Store` has no operation `pt`",
        ]
    );
}

// ----- pending obligations (`spec/12 §3`) and selfified lets (`spec/03 §7`) -----

#[test]
fn subsumption_into_refined_types_records_pending_obligations() {
    let (found, _) = obligations(
        "kind Small = Int in 0..9\n\
         \n\
         kind Point\n  x: Small\n  y: Int\n\
         \n\
         clamp(raw: Int) -> Small = if raw < 0 then 0 else if raw > 9 then 9 else raw\n\
         \n\
         use_it(n: Int) -> Int\n  requires n > 0\n  ensures result > 0\n=\n  let s: Small = 3\n  let p = Point(x: s, y: n)\n  let q = clamp(n / 2)\n  let r = n mod q\n  when n\n    in 1..5 -> n mod 7\n    in 6..<1000 -> use_it(n - 1)\n",
    );
    assert_eq!(
        found,
        [
            "refinement 9:44",
            "refinement 9:67",
            "refinement 9:74",
            "refinement 15:18",
            "refinement 16:20",
            "division 18:11",
            "exhaustive 19:3",
            "postcondition 20:16",
            "precondition 21:20",
            "postcondition 21:20",
        ]
    );
}

#[test]
fn lets_whose_value_is_in_the_refinement_language_are_selfified() {
    let (_, selfified) = obligations(
        "kind Point\n  x: Int\n  y: Int\n\
         \n\
         predicate reflect positive(x: Int) -> Bool = x > 0\n\
         \n\
         f(p: Point, n: Int) -> Int\n=\n  let a = p.x + 1\n  let b = positive(a) and n > 2\n  let c = abs(n)\n  let d = [n]\n  let e = \"x\"\n  let g = a\n  a\n",
    );
    assert_eq!(selfified, ["11:7", "12:7", "15:7", "16:7"]);
}
