//! Names (ADR 0023): resolution, duplicates, imports, cycles, conventions, requirements,
//! module paths and trusted modules.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use pretty_assertions::assert_eq;
use una_check::{ModuleInput, PackageInput, check_names};
use una_project::{Manifest, parse_manifest};
use una_syntax::{Interner, SourceDb, lex, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n";

/// The name diagnostics of `files`, as "code file:line:col message" in output order.
fn check(files: &[(&str, &str)], single: bool) -> Vec<String> {
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
    let mut diagnostics = check_names(&input);
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

/// A package with `una.toml` and the given modules.
fn package(modules: &[(&str, &str)]) -> Vec<String> {
    let mut files = vec![("una.toml", MANIFEST)];
    files.extend_from_slice(modules);
    check(&files, false)
}

#[test]
fn names_resolve_through_locals_declarations_imports_and_the_prelude() {
    let body = "kind Shape = Circle(radius: Int) | Dot\n\n\
        kind Point\n  x: Int\n  y: Int\n\n\
        predicate reflect small(n: Int) -> Bool = n < 10\n\n\
        capability Log\n  op write(line: Text) -> Unit\n\n\
        handler silent() for Log\n  op write(line) = unit\n\n\
        area(shape: Shape) -> Int\n=\n  when shape\n    is Circle(radius: r) -> r * r\n    else -> 0\n\n\
        first_x[T](xs: List[T], f: fn(T) -> Int) -> Option[Int]\n=\n  let y = map(xs, fn(v: T) -> f(v))\n  first(y)\n\n\
        origin() -> Point = Point(x: abs(0), y: 0)\n\n\
        logged() -> Unit\n  uses @log: Log[write]\n=\n  @log.write(\"a\") with @log = silent()\n\n\
        example \"small numbers\"\n  given @log = silent()\n  do small(3)\n  expect result == true\n";
    assert_eq!(single(body), Vec::<String>::new());
}

#[test]
fn a_name_no_scope_holds_is_t1001() {
    // Type names too.
    assert_eq!(
        single("f(x: Amount) -> Int = g(x)\n"),
        [
            "U1001 t.una:3:6 cannot find type `Amount` in this module, its imports or the prelude",
            "U1001 t.una:3:23 cannot find name `g` in this module, its imports or the prelude",
        ]
    );
    assert_eq!(
        single("f(x: Int) -> Int\n=\n  if x > 0\n    let y = x\n    y\n  else\n    y\n"),
        ["U1001 t.una:9:5 cannot find name `y` in this module, its imports or the prelude"]
    );
    assert_eq!(
        single(
            "f() -> Unit\n  uses @log: Journal\n  fails Oops\n=\n  unit\n\nhandler h() for Journal\n  op write(line) = unit\n\ng() -> Unit\n=\n  f() with @log = missing()\n\npolicy p\n  from Int\n  to Int\n  via declassify\n  purpose \"x\"\n  owner team\n"
        ),
        [
            "U1001 t.una:4:14 cannot find capability `Journal` in this module, its imports or the prelude",
            "U1001 t.una:5:9 cannot find error kind `Oops` in this module, its imports or the prelude",
            "U1001 t.una:9:17 cannot find capability `Journal` in this module, its imports or the prelude",
            "U1001 t.una:14:19 cannot find handler `missing` in this module, its imports or the prelude",
            "U1001 t.una:19:7 cannot find function `declassify` in this module, its imports or the prelude",
        ]
    );
    assert_eq!(
        single("kind Box[T]\n  item: T\n\nf(x: T) -> Int = 1\n"),
        ["U1001 t.una:6:6 cannot find type `T` in this module, its imports or the prelude"]
    );
}

#[test]
fn variants_in_context_wait_for_their_kind() {
    // Patterns, `is`, `fail` and `expect fails` take the kind from context.
    let body = "f(x: Int) -> Int\n  fails Oops\n=\n  when x\n    is Anything -> 1\n    else -> fail Whatever\n\ng(x: Int) -> Bool = x is Something\n\nexample \"e\"\n  do f(1)\n  expect fails Whatever\n";
    assert_eq!(
        single(body),
        [
            "U1001 t.una:4:9 cannot find error kind `Oops` in this module, its imports or the prelude"
        ]
    );
    // A bare constructor is resolved through scopes.
    assert_eq!(
        single("f() -> Int = Nothing\n"),
        ["U1001 t.una:3:14 cannot find name `Nothing` in this module, its imports or the prelude"]
    );
}

#[test]
fn declarations_may_reuse_prelude_names() {
    assert_eq!(
        single(
            "kind LookupError = NotFound | Unavailable\n\nsum(xs: List[Int]) -> Int = 0\n\nf() -> LookupError = NotFound\n"
        ),
        Vec::<String>::new()
    );
}

#[test]
fn duplicates_are_t1002() {
    assert_eq!(
        single(
            "f() -> Int = 1\n\npredicate reflect f() -> Bool = true\n\nkind A = X | Y\n\nkind B = X | Z\n\ncapability A\n  op go() -> Unit\n"
        ),
        [
            "U1002 t.una:5:19 `f` is defined twice in this module: as a function and as a predicate",
            "U1002 t.una:9:10 `X` is defined twice in this module, both times as a variant",
            "U1002 t.una:11:12 `A` is defined twice in this module: as a kind and as a capability",
        ]
    );
    assert_eq!(
        single(
            "f(x: Int, x: Int) -> Int = x\n\ng[T, T](x: T) -> Int = 1\n\nkind P\n  a: Int\n  a: Int\n\nkind V = W(b: Int, b: Int) | U\n\ncapability C\n  op go() -> Unit\n  op go() -> Unit\n\nhandler h() for C\n  op go() = unit\n  op go() = unit\n"
        ),
        [
            "U1002 t.una:3:11 `x` is already bound in this scope; a local cannot rebind another",
            "U1002 t.una:5:6 type parameter `T` is declared twice",
            "U1002 t.una:9:3 field `a` is declared twice",
            "U1002 t.una:11:20 field `b` is declared twice",
            "U1002 t.una:15:6 operation `go` is declared twice",
            "U1002 t.una:19:6 operation `go` is declared twice",
        ]
    );
    assert_eq!(
        single(
            "example \"one\"\n  do 1\n  expect true\n\nexample \"one\"\n  do 2\n  expect true\n"
        ),
        ["U1002 t.una:7:9 example \"one\" is defined twice in this module"]
    );
}

#[test]
fn a_local_cannot_rebind_another() {
    assert_eq!(
        single(
            "f(x: Int) -> Int\n=\n  let y = x\n  if y > 0\n    let y = 1\n    y\n  else\n    let x = 2\n    fold([1], 0, fn(acc, x) -> acc + x)\n"
        ),
        [
            "U1002 t.una:7:9 `y` is already bound in this scope; a local cannot rebind another",
            "U1002 t.una:10:9 `x` is already bound in this scope; a local cannot rebind another",
            "U1002 t.una:11:26 `x` is already bound in this scope; a local cannot rebind another",
        ]
    );
    // Sibling scopes may reuse a name, and locals may reuse global names.
    assert_eq!(
        single(
            "f(x: Int) -> Int\n=\n  when x\n    in 0..9 ->\n      let abs = 1\n      abs\n    else ->\n      let abs = 2\n      abs\n"
        ),
        Vec::<String>::new()
    );
}

#[test]
fn naming_conventions_are_t1005() {
    // spec/01 §4;
    assert_eq!(
        single(
            "kind Seat_Count = Int\n\ntotal_(a__b: Int) -> Int = a__b\n\ng[My_T](x: My_T) -> Int = 1\n\nrequirement EVENTS3\n  text \"a\"\n  source human\n  owner box_office_\n  status active\n"
        ),
        [
            "U1005 t.una:3:6 kind `Seat_Count` must be UpperCamelCase, as in `SeatCount`",
            "U1005 t.una:5:1 function `total_` must not end with `_`, as in `total`",
            "U1005 t.una:5:8 parameter `a__b` must not contain `__`, as in `a_b`",
            "U1005 t.una:7:3 type parameter `My_T` must be UpperCamelCase, as in `MyT`",
            "U1005 t.una:9:13 requirement identifier `EVENTS3` must be upper snake case ending with `_` and a number, as in `EVENTS_3`",
            "U1005 t.una:12:9 owner `box_office_` must not end with `_`, as in `box_office`",
        ]
    );
}

#[test]
fn requirements_are_unique_declared_and_generated_ones_proposed() {
    let requirement = |id: &str, source: &str, status: &str| {
        format!(
            "requirement {id}\n  text \"a\"\n  source {source}\n  owner team\n  status {status}\n"
        )
    };
    let a = format!(
        "module a\n\n{}\n{}\nf() -> Int\n  derived from R_1, R_2, R_9\n=\n  1\n",
        requirement("R_1", "human", "active"),
        requirement("R_3", "generated \"task 7\"", "active")
    );
    let b = format!(
        "module b\n\n{}",
        requirement("R_1", "policy \"P\"", "proposed")
    );
    let c = format!(
        "module c\n\n{}",
        requirement("R_2", "generated \"task 8\"", "proposed")
    );
    assert_eq!(
        package(&[("src/a.una", &a), ("src/b.una", &b), ("src/c.una", &c)]),
        [
            "U1007 src/a.una:13:10 requirement `R_3` has source `generated`, so its status must be `proposed`",
            "U1006 src/a.una:16:26 requirement `R_9` is not declared in this package",
            "U1002 src/b.una:3:13 requirement `R_1` is defined twice in this package",
        ]
    );
}

#[test]
fn imports_bring_public_names_and_the_variants_of_kinds() {
    let a = "module a\n\npub kind Availability = SoldOut | Open\n\npub availability(x: Int) -> Availability = if x > 0 then Open else SoldOut\n\nhelper() -> Int = 1\n";
    let b = "module b\n\nimport a (Availability, availability)\nimport std.testing (fake_hash)\n\nf() -> Availability = SoldOut\n\ng() -> Availability = availability(1)\n\nhandler h() for Keys\n  op hmac(key_name, data) = fake_hash(1)\n";
    assert_eq!(
        package(&[("src/a.una", a), ("src/b.una", b)]),
        Vec::<String>::new()
    );
    let c = "module c\n\nimport a (helper, missing)\nimport nowhere (x)\nimport std.testing (fake_key)\n";
    assert_eq!(
        package(&[("src/a.una", a), ("src/c.una", c)]),
        [
            "U1004 src/c.una:3:11 function `helper` is private to module `a`",
            "U1001 src/c.una:3:19 module `a` declares no `missing`",
            "U1001 src/c.una:4:8 module `nowhere` is not a module of this package; the standard modules that can be imported are `std.host` and `std.testing`",
            "U1001 src/c.una:5:21 module `std.testing` declares no `fake_key`",
        ]
    );
}

#[test]
fn imported_names_must_not_repeat_declarations_or_imports() {
    let a = "module a\n\npub one() -> Int = 1\n\npub two() -> Int = 2\n";
    let b = "module b\n\nimport a (one, two)\nimport a (two)\n\none() -> Int = 3\n";
    assert_eq!(
        package(&[("src/a.una", a), ("src/b.una", b)]),
        [
            "U1002 src/b.una:3:11 `one` is already defined in this module",
            "U1002 src/b.una:4:11 `two` is already defined in this module",
        ]
    );
}

#[test]
fn import_cycles_are_one_t1003_each() {
    let a = "module a\n\nimport b (y)\n\npub x() -> Int = 1\n";
    let b = "module b\n\nimport c (z)\n\npub y() -> Int = 2\n";
    let c = "module c\n\nimport a (x)\n\npub z() -> Int = 3\n";
    let d = "module d\n\nimport d (w)\n\npub w() -> Int = 4\n";
    assert_eq!(
        package(&[
            ("src/a.una", a),
            ("src/b.una", b),
            ("src/c.una", c),
            ("src/d.una", d)
        ]),
        [
            "U1003 src/a.una:3:8 import cycle between modules `a`, `b` and `c`",
            "U1003 src/d.una:3:8 module `d` imports itself",
            "U1002 src/d.una:3:11 `w` is already defined in this module",
        ]
    );
}

#[test]
fn module_paths_follow_file_paths() {
    let wrong = "module a.c\n\nf() -> Int = 1\n";
    assert_eq!(
        package(&[("src/a/b.una", wrong)]),
        ["U1008 src/a/b.una:1:8 module `a.c` is in `src/a/b.una`, which must hold module `a.b`"]
    );
    // A single file has no module path check.
    assert_eq!(check(&[("b.una", wrong)], true), Vec::<String>::new());
}

#[test]
fn trusted_modules_are_declared_twice() {
    // spec/07 §6;
    let keys = "trusted module keys\n\nf() -> Int = 1\n";
    assert_eq!(
        check(&[("keys.una", keys)], true),
        [
            "U7005 keys.una:1:1 module `keys` is `trusted`, but a file checked on its own has no una.toml to list it in [trusted] modules"
        ]
    );
    let manifest = format!(
        "{MANIFEST}\n[trusted]\nmodules = [\"keys\", \"plain\", \"ghost\"]\n\n[run]\nentry = \"main\"\n"
    );
    let plain = "module plain\n\ng() -> Int = 2\n";
    let other = "trusted module other\n\nh() -> Int = 3\n";
    assert_eq!(
        check(
            &[
                ("una.toml", &manifest),
                ("src/keys.una", keys),
                ("src/other.una", other),
                ("src/plain.una", plain),
            ],
            false
        ),
        [
            "U7005 src/other.una:1:1 module `other` is `trusted` in its header but not listed in una.toml [trusted] modules",
            "U7005 una.toml:7:20 [trusted] modules lists `plain`, whose header does not say `trusted module`",
            "U7005 una.toml:7:29 [trusted] modules lists `ghost`, which is not a module of this package",
            "U7002 una.toml:10:9 [run] entry `main` is not a module of this package",
        ]
    );
}

#[test]
fn modules_with_syntax_errors_are_not_resolved() {
    // Declarations that parsed are still checked for U1002 and U1005.
    let broken = "module a\n\npub f() -> Int = missing(1)\n\npub g() Int = 2\n\npub h() -> Int = 3\n\npub h() -> Int = 4\n";
    let user = "module b\n\nimport a (g, f)\n\nuse_it() -> Int = g()\n";
    assert_eq!(
        package(&[("src/a.una", broken), ("src/b.una", user)]),
        ["U1002 src/a.una:9:5 `h` is defined twice in this module, both times as a function"]
    );
}
