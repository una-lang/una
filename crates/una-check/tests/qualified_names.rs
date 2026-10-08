//! Names in messages (#13, #17; reviews R-045 and N-015): a declaration whose bare name another
//! declaration shares, in another module of the package or in the standard library, is printed
//! with its module's path, so that "needs `@e` to be `Env`, and here it is `Env`" says which.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use pretty_assertions::assert_eq;
use una_check::{ModuleInput, PackageInput, TypeCheck, check_names, check_types};
use una_diag::Diagnostic;
use una_project::{Edition, Manifest, parse_manifest};
use una_syntax::{Interner, KeywordSet, SourceDb, lex_in, parse};

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
        // The keywords of the manifest's edition, which comes first in `files`.
        let keywords = match settings.edition {
            Edition::V0_1 => KeywordSet::V0_1,
            Edition::V0_2 => KeywordSet::V0_2,
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
    };
    let names = check_names(&input);
    let types = check_types(&input);
    (names, types)
}

/// Every diagnostic of a package with `una.toml` and `modules`, as "code file:line:col message".
fn package(modules: &[(&str, &str)]) -> Vec<String> {
    let mut files = vec![("una.toml", MANIFEST)];
    files.extend_from_slice(modules);
    let (names, types) = run(&files, false);
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

#[test]
fn a_capability_of_the_module_that_shadows_a_host_one_is_named_as_the_modules() {
    // #13 (R-045): the module's own `Console` hides the host one; the message names the
    // module's, not a `Console` that seems to lack `print`.
    assert_eq!(
        package(&[(
            "src/m.una",
            "module m\n\npub capability Console\n  op shout(text: Text) -> Unit\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"hi\")\n"
        )]),
        [
            "U1001 src/m.una:7:26 capability `m.Console` has no operation `print`",
            "U1001 src/m.una:9:12 capability `m.Console` has no operation `print`",
        ]
    );
}

#[test]
fn the_host_capability_is_named_by_the_module_that_exports_it() {
    // Module `a` declares its own `Console`; module `b` has the host's, which the prelude
    // declares and `std.host` exports (`spec/14 §3`).
    assert_eq!(
        package(&[
            (
                "src/a.una",
                "module a\n\npub capability Console\n  op print(text: Text) -> Unit\n\npub say() -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"a\")\n"
            ),
            (
                "src/b.una",
                "module b\n\nimport a (say)\n\npub f() -> Unit\n  uses @console: Console[print]\n=\n  say()\n"
            ),
        ]),
        [
            "U3001 src/b.una:8:3 `say` needs `@console` to be `a.Console`, and here it is `std.host.Console`"
        ]
    );
}

#[test]
fn two_capabilities_or_kinds_of_one_name_are_printed_with_their_modules() {
    // #17 (N-015): "needs `@e` to be `Env`, and here it is `Env`" said nothing.
    assert_eq!(
        package(&[
            (
                "src/a.una",
                "module a\n\npub capability Env\n  op get() -> Int\n\npub need() -> Int\n  uses @e: Env[get]\n=\n  @e.get()\n"
            ),
            (
                "src/b.una",
                "module b\n\nimport a (need)\n\npub capability Env\n  op get() -> Int\n\npub f() -> Int\n  uses @e: Env[get]\n=\n  need()\n"
            ),
        ]),
        ["U3001 src/b.una:11:3 `need` needs `@e` to be `a.Env`, and here it is `b.Env`"]
    );
    assert_eq!(
        package(&[
            (
                "src/a.una",
                "module a\n\npub kind Item\n  n: Int\n\npub use_item(x: Item) -> Int = x.n\n"
            ),
            (
                "src/b.una",
                "module b\n\nimport a (use_item)\n\npub kind Item\n  n: Int\n\npub f(x: Item) -> Int = use_item(x)\n"
            ),
        ]),
        ["U2001 src/b.una:8:34 argument `x` of `use_item` must be `a.Item`, found `b.Item`"]
    );
}

#[test]
fn a_name_no_other_declaration_shares_stays_bare() {
    assert_eq!(
        package(&[(
            "src/a.una",
            "module a\n\npub kind Thing\n  n: Int\n\npub use_thing(x: Thing) -> Int = x.n\n\npub f() -> Int = use_thing(1)\n"
        )]),
        ["U2001 src/a.una:8:28 argument `x` of `use_thing` must be `Thing`, found `Int`"]
    );
}

/// Each diagnostic of a package with `una.toml` and `modules`, with the replacement text of
/// each of its repairs, as "code message => [replacements]".
fn repairs(modules: &[(&str, &str)]) -> Vec<String> {
    let mut files = vec![("una.toml", MANIFEST)];
    files.extend_from_slice(modules);
    let (names, types) = run(&files, false);
    names
        .iter()
        .chain(&types.diagnostics)
        .map(|d| {
            let edits: Vec<&str> = d
                .repairs
                .iter()
                .filter_map(|r| r.edit.as_ref().map(|e| e.replacement.as_str()))
                .collect();
            format!("{} {} => {edits:?}", d.code, d.message)
        })
        .collect()
}

/// A shared name is qualified in a message only: a repair writes the name as the module's
/// source does, since `uses @console: m.Console[shout]` is not a clause Una can parse.
#[test]
fn a_repair_writes_the_declared_name_where_the_message_qualifies_it() {
    let found = repairs(&[(
        "src/m.una",
        "module m\n\npub capability Console\n  op shout(text: Text) -> Unit\n\npub kind FileError = | Missing\n\npub say() -> Unit\n  uses @console: Console[shout]\n=\n  @console.shout(\"a\")\n\npub caller() -> Unit = say()\n\npub f(n: Int) -> Int\n  fails FileError\n=\n  fail Missing\n\npub g(n: Int) -> Int\n=\n  let r = f(n)\n    if error pass\n  r\n",
    )]);
    let reported = |what: &str| -> (String, String) {
        let line = found
            .iter()
            .find(|d| d.contains(what))
            .unwrap_or_else(|| panic!("{what} is reported: {found:?}"));
        let (message, edits) = line.split_once(" => ").unwrap();
        (message.to_owned(), edits.to_owned())
    };
    let (message, edits) = reported("`say` uses");
    assert!(
        message.contains("`@console: m.Console[shout]`"),
        "{message}"
    );
    assert!(edits.contains("uses @console: Console[shout]"), "{edits}");
    assert!(!edits.contains("m.Console"), "{edits}");
    let (message, edits) = reported("`if error pass` passes");
    assert!(message.contains("`m.FileError` failure"), "{message}");
    assert!(edits.contains("fails FileError"), "{edits}");
    assert!(!edits.contains("m.FileError"), "{edits}");
}

/// Core IR names a kind as it is declared: the module is a separate part of its identity
/// (`spec/12 §4`), so a qualified name there would not resolve. A hypothesis of the verifier
/// was lost that way when two modules declared `K`.
#[test]
fn core_names_a_shared_kind_as_declared() {
    let mut files = vec![("una.toml", MANIFEST)];
    files.extend_from_slice(&[
        ("src/a.una", "module a\n\npub kind K = First | Second\n"),
        (
            "src/b.una",
            "module b\n\npub kind K = One | Two\n\npub pick(k: K) -> Int\n=\n  when k\n    is One -> 1\n    is Two -> 2\n",
        ),
    ]);
    let (names, types) = run(&files, false);
    assert!(
        names.is_empty() && types.diagnostics.is_empty(),
        "{names:?} {:?}",
        types.diagnostics
    );
    let core = format!("{:?}", types.program);
    assert!(core.contains("\"K\""), "the kind is named `K` in Core");
    assert!(
        !core.contains("\"a.K\"") && !core.contains("\"b.K\""),
        "{core}"
    );
}
