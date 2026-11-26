//! The parser against `spec/02`, the precedence table of `spec/01 §7`, U0102 and the recovery
//! rules of the approved M1 plan.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use pretty_assertions::assert_eq;
use una_syntax::ast::{DeclKind, ExprKind, File, Stmt};
use una_syntax::{Interner, KeywordSet, SourceDb, dump, lex, lex_in, parse};

struct Parsed {
    file: File,
    tree: String,
    errors: Vec<String>,
    db: SourceDb,
}

fn parse_source(text: &str) -> Parsed {
    parse_source_in(text, None)
}

/// [`parse_source`] with the keywords of an edition, or of edition `0.1` when `None`.
fn parse_source_in(text: &str, keywords: Option<KeywordSet>) -> Parsed {
    let mut db = SourceDb::new();
    let id = db.add("t.una", text.as_bytes());
    let mut interner = Interner::new();
    let lexed = match keywords {
        Some(keywords) => lex_in(db.file(id), id, &mut interner, keywords),
        None => lex(db.file(id), id, &mut interner),
    };
    let parsed = parse(&lexed, id, &interner);
    let errors = lexed
        .errors
        .iter()
        .chain(&parsed.errors)
        .map(|error| {
            let diagnostic = error.to_diagnostic(&db);
            format!(
                "{} {}:{} {}",
                diagnostic.code,
                diagnostic.primary.start.line,
                diagnostic.primary.start.col,
                diagnostic.message
            )
        })
        .collect();
    let tree = dump(&parsed.file, &interner, db.file(id).text());
    Parsed {
        file: parsed.file,
        tree,
        errors,
        db,
    }
}

/// The dump of the declarations of `body`, placed after `module t` and a blank line.
fn tree(body: &str) -> String {
    let parsed = parse_source(&format!("module t\n\n{body}"));
    assert_eq!(parsed.errors, Vec::<String>::new(), "unexpected errors");
    parsed.tree.strip_prefix("(module t)\n").unwrap().to_owned()
}

/// The errors of `body`, placed after `module t` and a blank line (so it starts on line 3).
fn errors(body: &str) -> Vec<String> {
    parse_source(&format!("module t\n\n{body}")).errors
}

/// The dump of a single expression, as the body of a short-form function.
fn expr(text: &str) -> String {
    let dumped = tree(&format!("f() -> Int = {text}\n"));
    dumped
        .strip_prefix("(fn f () {Int} ")
        .and_then(|rest| rest.strip_suffix(")\n"))
        .unwrap()
        .to_owned()
}

// ----- files and modules (spec/02 §1) -----

#[test]
fn module_header_and_imports() {
    let parsed = parse_source(
        "trusted module events.keys\n\nimport events.types (Phone, parse_phone)\nimport std.testing (fake_hash)\n",
    );
    assert_eq!(parsed.errors, Vec::<String>::new());
    assert_eq!(
        parsed.tree,
        "(module trusted events.keys)\n(import events.types Phone parse_phone)\n(import std.testing fake_hash)\n"
    );
}

#[test]
fn a_missing_module_header_is_reported_once_and_declarations_still_parse() {
    let parsed = parse_source("pub one() -> Int = 1\n");
    assert_eq!(
        parsed.errors,
        ["U0101 1:1 unexpected keyword `pub`, expected `module` or `trusted module`"]
    );
    assert_eq!(parsed.file.decls.len(), 1);
    let empty = parse_source("");
    assert_eq!(
        empty.errors,
        ["U0101 1:1 unexpected end of file, expected `module` or `trusted module`"]
    );
    let junk = parse_source("hello world\npub one() -> Int = 1\n");
    assert_eq!(
        junk.errors,
        ["U0101 1:1 unexpected `hello`, expected `module` or `trusted module`"]
    );
    assert_eq!(junk.file.decls.len(), 1);
}

#[test]
fn imports_come_before_declarations() {
    assert_eq!(
        errors("pub one() -> Int = 1\n\nimport std.testing (fake_hash)\n"),
        ["U0101 5:1 unexpected keyword `import`, expected a declaration"]
    );
}

// ----- kinds and types (spec/02 §2) -----

#[test]
fn kind_bodies() {
    assert_eq!(
        tree(
            "pub kind Seats = Int in 0..5000\n\
             kind Even = Int where it mod 2 == 0\n\
             pub kind Shape = Circle(radius: Nat) | Square(side: Nat, label: Text)\n\
             kind Only = | One\n\
             kind Point\n  x: Int\n  y: Int\n\
             kind Pair[A, B]\n  first: A\n  second: B\n"
        ),
        "(kind pub Seats (alias {Int in 0..5000}))\n\
         (kind Even (alias {Int} (where (== (mod it 2) 0))))\n\
         (kind pub Shape (sum (Circle (radius {Nat})) (Square (side {Nat}) (label {Text}))))\n\
         (kind Only (sum One))\n\
         (kind Point (record (x {Int}) (y {Int})))\n\
         (kind Pair [A, B] (record (first {A}) (second {B})))\n"
    );
}

#[test]
fn a_bar_inside_brackets_does_not_make_a_sum() {
    // The line has `|` only inside brackets, so the body is an alias whose type is malformed.
    assert_eq!(
        errors("kind Odd = List[A | B]\n"),
        ["U0101 3:19 unexpected `|`, expected `,` or `]`"]
    );
}

#[test]
fn a_sum_needs_two_variants_or_a_leading_bar() {
    assert_eq!(
        errors("kind Odd = A(x: Int) Int | B\n"),
        ["U0101 3:22 unexpected `Int`, expected `|` and another variant"]
    );
}

#[test]
fn types() {
    assert_eq!(
        tree(
            "f(a: secret[PII, Location] Phone, b: [Int], c: Result[Int, Text], d: Int in -5..<5) -> internal Hashed = a\n\
             g(h: fn(Int, Text) -> Bool uses @log: Log[write] fails StoreError, k: fn() -> Unit) -> Unit = unit\n\
             m(xs: secret List[Int], ys: secret[PII] [Int]) -> public Int = 0\n"
        ),
        "(fn f\n  \
           ((a {secret[PII, Location] Phone}) (b {[Int]}) (c {Result[Int, Text]}) (d {Int in -5..<5}))\n  \
           {internal Hashed}\n  \
           a)\n\
         (fn g\n  \
           ((h {fn(Int, Text) -> Bool uses @log: Log[write] fails StoreError}) (k {fn() -> Unit}))\n  \
           {Unit}\n  \
           unit)\n\
         (fn m ((xs {secret List[Int]}) (ys {secret[PII] [Int]})) {public Int} 0)\n"
    );
}

#[test]
fn a_bracket_after_a_level_starts_label_categories() {
    // `secret [Int]` reads `[Int]` as the label's categories.
    assert_eq!(
        errors("f(xs: secret [Int]) -> Int = 0\n"),
        [
            "U0101 3:19 unexpected `)`, expected a type after the label; a labeled list type is written `secret List[T]`"
        ]
    );
    assert_eq!(
        errors("f(xs: secret) -> Int = 0\n"),
        ["U0101 3:13 unexpected `)`, expected a type after the label"]
    );
}

// ----- predicates and functions (spec/02 §3) -----

#[test]
fn predicates() {
    assert_eq!(
        tree(
            "pub predicate reflect small(n: Int) -> Bool = n < 10\n\
             predicate opaque valid(t: Text) -> Bool =\n  let n = t.len\n  n == 11\n"
        ),
        "(predicate pub reflect small ((n {Int})) {Bool} (< n 10))\n\
         (predicate opaque valid ((t {Text})) {Bool} (block (let n (. t len)) (== n 11)))\n"
    );
    assert_eq!(
        errors("predicate small(n: Int) -> Bool = n < 10\n"),
        ["U0101 3:11 unexpected `small`, expected `reflect` or `opaque`"]
    );
}

#[test]
fn functions() {
    assert_eq!(
        tree(
            "pub one() -> Int = 1\n\
             two() -> Int\n=\n  2\n\
             first[T](xs: List[T]) -> Option[T]\n=\n  xs.first()\n\
             pub save(value: Int) -> Int\n  requires value >= 0\n  requires value < 100\n  ensures result == value\n  uses @store: Store[put]\n  uses @log: Log\n  fails StoreError\n  derived from AUDIT_1, AUDIT_2\n=\n  value\n"
        ),
        "(fn pub one () {Int} 1)\n\
         (fn two () {Int} (block 2))\n\
         (fn first [T] ((xs {List[T]})) {Option[T]} (block (method xs first)))\n\
         (fn pub save\n  \
           ((value {Int}))\n  \
           {Int}\n  \
           (requires (>= value 0))\n  \
           (requires (< value 100))\n  \
           (ensures (== result value))\n  \
           (uses @store {Store[put]})\n  \
           (uses @log {Log})\n  \
           (fails StoreError)\n  \
           (derived-from AUDIT_1 AUDIT_2)\n  \
           (block value))\n"
    );
}

#[test]
fn function_header_errors() {
    assert_eq!(
        errors("pub one() Int = 1\n"),
        ["U0101 3:11 unexpected `Int`, expected `->` and the result type"]
    );
    assert_eq!(
        errors("pub total(if: Int) -> Int = 1\n"),
        ["U0101 3:11 `if` is a reserved keyword, so it cannot name a binding"]
    );
    assert_eq!(
        errors("pub one() -> Int\n  requires true\n  returns 1\n=\n  1\n"),
        [
            "U0101 5:3 unexpected `returns`, expected a clause (`requires`, `ensures`, `uses`, `fails` or `derived from`)"
        ]
    );
    assert_eq!(
        errors("pub one() -> Int\n  requires true\n  1\n"),
        [
            "U0101 5:3 unexpected `1`, expected a clause (`requires`, `ensures`, `uses`, `fails` or `derived from`)"
        ]
    );
}

#[test]
fn function_clauses_follow_the_canonical_order() {
    assert_eq!(
        errors(
            "f(x: Int) -> Int\n  ensures result == x\n  requires x > 0\n  derived from R\n  fails E\n  fails E\n  derived from S\n  uses @log: Log\n=\n  x\n"
        ),
        [
            "U0102 5:3 `requires` clause after `ensures`; clauses go in the order requires, ensures, uses, fails, derived from",
            "U0102 7:3 `fails` clause after `derived from`; clauses go in the order requires, ensures, uses, fails, derived from",
            "U0102 8:3 repeated `fails` clause; at most one is allowed",
            "U0102 9:3 repeated `derived from` clause; list every requirement in one clause",
            "U0102 10:3 `uses` clause after `derived from`; clauses go in the order requires, ensures, uses, fails, derived from",
        ]
    );
}

#[test]
fn clause_errors_do_not_stop_parsing() {
    let parsed =
        parse_source("module t\n\nf(x: Int) -> Int\n  ensures true\n  requires true\n=\n  x x\n");
    assert_eq!(
        parsed.errors,
        [
            "U0102 5:3 `requires` clause after `ensures`; clauses go in the order requires, ensures, uses, fails, derived from",
            "U0101 7:5 unexpected `x`, expected end of line",
        ]
    );
}

// ----- capabilities, handlers and policies (spec/02 §4) -----

#[test]
fn capabilities_handlers_and_policies() {
    assert_eq!(
        tree(
            "pub capability Store\n  op put(key: Int, value: Text) -> Unit\n    requires key > 0\n    ensures true\n    fails StoreError\n    accepts internal[PII]\n  op size() -> Int\n\
             handler memory(limit: Int) for Store\n  uses @log: Log[write]\n  op put(key, value) = unit\n  op size() =\n    limit\n\
             policy key_policy\n  from secret[PII] Phone\n  to internal Hashed\n  via hmac\n  purpose \"lookup\"\n  owner privacy\n"
        ),
        "(capability pub Store\n  \
           (op put\n    \
             ((key {Int}) (value {Text}))\n    \
             {Unit}\n    \
             (requires (> key 0))\n    \
             (ensures true)\n    \
             (fails StoreError)\n    \
             (accepts internal[PII]))\n  \
           (op size () {Int}))\n\
         (handler memory\n  \
           ((limit {Int}))\n  \
           (for Store)\n  \
           (uses @log {Log[write]})\n  \
           (op put (key value) unit)\n  \
           (op size () (block limit)))\n\
         (policy key_policy\n  \
           (from {secret[PII] Phone})\n  \
           (to {internal Hashed})\n  \
           (via hmac)\n  \
           (purpose \"lookup\")\n  \
           (owner privacy))\n"
    );
}

#[test]
fn operation_clauses_follow_the_canonical_order() {
    assert_eq!(
        errors(
            "capability Store\n  op put(key: Int) -> Unit\n    accepts internal\n    fails E\n    ensures true\n    accepts public\n    fails F\n"
        ),
        [
            "U0102 6:5 `fails` clause after `accepts`; operation clauses go in the order requires, ensures, fails, accepts",
            "U0102 7:5 `ensures` clause after `accepts`; operation clauses go in the order requires, ensures, fails, accepts",
            "U0102 8:5 repeated `accepts` clause; at most one is allowed",
            "U0102 9:5 repeated `fails` clause; at most one is allowed",
        ]
    );
}

#[test]
fn policy_lines_are_fixed() {
    assert_eq!(
        errors("policy p\n  from Phone\n  via hmac\n  to Hashed\n  purpose \"x\"\n  owner o\n"),
        ["U0101 5:3 unexpected `via`, expected `to`"]
    );
}

// ----- requirements and examples (spec/02 §5) -----

#[test]
fn requirements() {
    assert_eq!(
        tree(
            "requirement R_1\n  text \"a\"\n  source human\n  owner team\n  status active\n\
             requirement R_2\n  text \"b\"\n  source policy \"Policy 1\"\n  owner team\n  status proposed\n\
             requirement R_3\n  text \"c\"\n  source regulation \"LGPD art. 6\"\n  owner team\n  status active\n\
             requirement R_4\n  text \"d\"\n  source generated \"task 12\"\n  owner team\n  status proposed\n"
        ),
        "(requirement R_1 (text \"a\") (source human) (owner team) (status active))\n\
         (requirement R_2 (text \"b\") (source policy \"Policy 1\") (owner team) (status proposed))\n\
         (requirement R_3 (text \"c\") (source regulation \"LGPD art. 6\") (owner team) (status active))\n\
         (requirement R_4 (text \"d\") (source generated \"task 12\") (owner team) (status proposed))\n"
    );
    assert_eq!(
        errors("requirement R\n  text \"a\"\n  source human\n  owner team\n  status done\n"),
        ["U0101 7:10 unexpected `done`, expected `proposed` or `active`"]
    );
    assert_eq!(
        errors("pub requirement R\n  text \"a\"\n  source human\n  owner team\n  status active\n"),
        [
            "U0101 3:5 unexpected keyword `requirement`, expected a kind, predicate, function, capability, handler or policy after `pub`"
        ]
    );
}

#[test]
fn examples() {
    assert_eq!(
        tree(
            "example \"all forms\"\n  given @store = memory(limit: 2)\n  given @log = silent()\n  do save(1)\n  expect fails Full\n  expect performed @store.put\n  expect not performed @log.write\n  expect not (result == 1)\n  expect result is Ok\n"
        ),
        "(example \"all forms\"\n  \
           (given @store (call memory (limit: 2)))\n  \
           (given @log (call silent))\n  \
           (do (call save 1))\n  \
           (expect-fails Full)\n  \
           (expect-performed @store.put)\n  \
           (expect-not-performed @log.write)\n  \
           (expect (not (paren (== result 1))))\n  \
           (expect (is result Ok)))\n"
    );
}

// ----- blocks and statements (spec/02 §6) -----

#[test]
fn statements_and_handling_clauses() {
    assert_eq!(
        tree(
            "f(key: Int) -> Int\n=\n  let a: Int = g(key)\n    if error fail Unavailable(reason: \"store\")\n    if none use 0\n  let b = g(key)\n    if error pass\n  let c = g(key)\n    if error use error.code\n  let d = g(key)\n    if none fail NotFound\n  run(key) with @log = @audit, @store = memory(limit: 2)\n  a + b + c + d\n"
        ),
        "(fn f\n  \
           ((key {Int}))\n  \
           {Int}\n  \
           (block\n    \
             (let a {Int}\n      \
               (handled (call g key) (if-error-fail (Unavailable (reason: \"store\"))) (if-none-use 0)))\n    \
             (let b (handled (call g key) (if-error-pass)))\n    \
             (let c (handled (call g key) (if-error-use (. error code))))\n    \
             (let d (handled (call g key) (if-none-fail NotFound)))\n    \
             (handled (call run key) (with (= @log @audit) (= @store (call memory (limit: 2)))))\n    \
             (+ (+ (+ a b) c) d)))\n"
    );
}

#[test]
fn handling_clauses_follow_the_canonical_order() {
    // `spec/06 §3` orders handling clauses, checked by the parser.
    assert_eq!(
        errors(
            "f() -> Int\n=\n  let a = g()\n    if none use 0\n    if error pass\n    if none use 1\n  let b = g()\n    if error pass\n    if error use 0\n  a\n"
        ),
        [
            "U0102 7:5 `if error` clause after `if none`; handling clauses go in the order if error, if none",
            "U0102 8:5 repeated `if none` clause; at most one is allowed",
            "U0102 11:5 repeated `if error` clause; at most one is allowed",
        ]
    );
}

#[test]
fn when_and_patterns() {
    assert_eq!(
        tree(
            "f(s: Shape) -> Int\n=\n  when s\n    is Circle(radius) -> radius\n    is Square(side: length, color) -> length\n    is Empty -> 0\n    is 7 -> 7\n    is \"seven\" -> 7\n    is true -> 1\n    in -10..<0 ->\n      let n = 0\n      n\n    else -> 1\n"
        ),
        "(fn f\n  \
           ((s {Shape}))\n  \
           {Int}\n  \
           (block\n    \
             (when s\n      \
               (arm (is Circle radius) radius)\n      \
               (arm (is Square side:length color) length)\n      \
               (arm (is Empty) 0)\n      \
               (arm (is 7) 7)\n      \
               (arm (is \"seven\") 7)\n      \
               (arm (is true) 1)\n      \
               (arm (in -10..<0) (block (let n 0) n))\n      \
               (arm else 1))))\n"
    );
}

#[test]
fn if_forms() {
    assert_eq!(
        tree(
            "f(x: Int) -> Int\n=\n  let a = if x > 0 then 1 else 2\n  let b = if x > 0\n    1\n  else if x == 0\n    0\n  else\n    -1\n  if a > b then a else b\n\
             g(x: Int) -> Int\n=\n  if x > 0\n    1\n  else\n    2\n"
        ),
        "(fn f\n  \
           ((x {Int}))\n  \
           {Int}\n  \
           (block\n    \
             (let a (if (> x 0) 1 2))\n    \
             (let b (if (> x 0) (block 1) (if (== x 0) (block 0) (block (neg 1)))))\n    \
             (if (> a b) a b)))\n\
         (fn g ((x {Int})) {Int} (block (if (> x 0) (block 1) (block 2))))\n"
    );
}

/// From edition `0.2` a `block_expr` may also be the value of an assignment (`spec/17 §1`),
/// so the message of a misplaced one names it (`spec/19 §1` rule 2, #929).
#[test]
fn misplaced_block_expr_names_assignments_from_edition_0_2() {
    let errors_0_2 =
        |body: &str| parse_source_in(&format!("module t\n\n{body}"), Some(KeywordSet::V0_2)).errors;
    assert_eq!(
        errors_0_2("f(xs: List[Int]) -> Int\n=\n  together for x in xs\n    x\n  0\n"),
        [
            "U0101 5:3 a `together for` must be the last line of its block or the value of a `let` or of an assignment"
        ]
    );
    assert_eq!(
        errors_0_2("f(x: Int) -> Int\n=\n  when x\n    else -> 1\n  x\n"),
        [
            "U0101 5:3 a `when` must be the last line of its block or the value of a `let` or of an assignment"
        ]
    );
}

#[test]
fn block_shape_errors() {
    assert_eq!(
        errors("f(x: Int) -> Int\n=\n  let y = x\n"),
        ["U0101 5:3 a block cannot end with `let`; its last line is its value"]
    );
    assert_eq!(
        errors("f(x: Int) -> Int\n=\n  when x\n    else -> 1\n  x\n"),
        ["U0101 5:3 a `when` must be the last line of its block or the value of a `let`"]
    );
    assert_eq!(
        errors("f(x: Int) -> Int\n=\n  if x > 0\n    1\n  else\n    2\n  x\n"),
        [
            "U0101 5:3 an `if` with blocks must be the last line of its block or the value of a `let`"
        ]
    );
    assert_eq!(
        errors("f(x: Int) -> Int = if x > 0\n"),
        ["U0101 3:28 unexpected end of line, expected `then`"]
    );
    assert_eq!(
        errors("f(x: Int) -> Int\n=\n  if x > 0\n    1\n  else if x < 0 then 2 else 3\n"),
        [
            "U0101 7:17 unexpected keyword `then`, expected end of line; after an `if` with blocks, `else if` has blocks too"
        ]
    );
    assert_eq!(
        errors("f(x: Int) -> Int\n=\n  when x\n    1 -> 1\n"),
        ["U0101 6:5 unexpected `1`, expected a pattern (`is`, `in` or `else`)"]
    );
}

// ----- expressions (spec/02 §7, spec/01 §7) -----

#[test]
fn precedence_and_associativity() {
    let cases = [
        ("a or b and c", "(or a (and b c))"),
        ("a and b or c", "(or (and a b) c)"),
        ("not a == b", "(not (== a b))"),
        ("not not a", "(not (not a))"),
        ("a and not b", "(and a (not b))"),
        ("a + b * c", "(+ a (* b c))"),
        ("a - b - c", "(- (- a b) c)"),
        ("a / b * c mod d", "(mod (* (/ a b) c) d)"),
        ("a implies b implies c", "(implies a (implies b c))"),
        ("a and b implies c or d", "(implies (and a b) (or c d))"),
        ("-a.b", "(neg (. a b))"),
        ("a * -b", "(* a (neg b))"),
        ("- -a", "(neg (neg a))"),
        ("x mod 2 == 0", "(== (mod x 2) 0)"),
        ("a + b < c * d", "(< (+ a b) (* c d))"),
        ("a is Some", "(is a Some)"),
        ("a is 0", "(is a 0)"),
        ("a + 1 in 0..<10", "(in (+ a 1) 0..<10)"),
        ("(a or b) and c", "(and (paren (or a b)) c)"),
        ("f(x).g.h(y)(z)", "(call (method (. (call f x) g) h y) z)"),
    ];
    for (source, expected) in cases {
        assert_eq!(expr(source), expected, "{source}");
    }
}

#[test]
fn comparisons_do_not_chain() {
    let message = "comparisons cannot be chained; join them with `and`, as in `a < b and b < c`";
    for (source, column) in [
        ("0 < x < 10", 21),
        ("a == b is C", 22),
        ("a is B in 0..1", 22),
        ("a in 0..1 == b", 25),
        ("a != b != c", 22),
    ] {
        assert_eq!(
            errors(&format!("f() -> Bool = {source}\n")),
            [format!("U0101 3:{column} {message}")],
            "{source}"
        );
    }
    assert_eq!(
        expr("(0 < x) == (x < 10)"),
        "(== (paren (< 0 x)) (paren (< x 10)))"
    );
}

#[test]
fn expression_forms() {
    let cases = [
        ("if a then b else c", "(if a b c)"),
        ("if a then b else if c then d else e", "(if a b (if c d e))"),
        ("fn(x: Int, y) -> x + y", "(fn ((x {Int}) y) (+ x y))"),
        ("fn() -> 1", "(fn () 1)"),
        (
            "checked(raw, valid(it)) else fail Malformed(at: 1)",
            "(checked raw (call valid it) (fail (Malformed (at: 1))))",
        ),
        ("attempt @store.find(key)", "(attempt (@store.find key))"),
        ("attempt f(x).g", "(attempt (. (call f x) g))"),
        ("fail NotFound", "(fail NotFound)"),
        (
            "fail Unavailable(reason: \"x\")",
            "(fail (Unavailable (reason: \"x\")))",
        ),
        ("[]", "(list)"),
        ("[1, 2, [3]]", "(list 1 2 (list 3))"),
        ("Point(x: 1, y: 2)", "(call Point (x: 1) (y: 2))"),
        ("@log.write(\"a\")", "(@log.write \"a\")"),
        ("@log", "@log"),
        ("result", "result"),
        ("it", "it"),
        ("error", "error"),
        ("true", "true"),
        ("false", "false"),
        ("unit", "unit"),
        ("1_000", "1000"),
        ("\"\"", "\"\""),
        (
            "\"a {b.c} d {e + 1}\"",
            "(text \"a \" (. b c) \" d \" (+ e 1))",
        ),
        ("\"{x}\"", "(text x)"),
        ("xs.map(fn(x) -> x * 2)", "(method xs map (fn (x) (* x 2)))"),
    ];
    for (source, expected) in cases {
        assert_eq!(expr(source), expected, "{source}");
    }
}

#[test]
fn a_brace_no_expression_can_follow_is_one_u0007_403() {
    // #403 (D-1): JSON written in text was a cascade of U0101, U0004, U0001 and U0006; each
    // brace that cannot start or end an interpolation is now one U0007, and `{n}` still
    // interpolates.
    assert_eq!(
        errors("f(n: Int) -> Text = \"{\\\"a\\\": {n}, \\\"b\\\": { \\\"c\\\": 1}}\"\n"),
        [
            "U0007 3:22 unescaped `{` in text, where no interpolation can start; write `\\{`",
            "U0007 3:42 unescaped `{` in text, where no interpolation can start; write `\\{`",
            "U0007 3:52 unescaped `}` in text; write `\\}`",
            "U0007 3:53 unescaped `}` in text; write `\\}`"
        ]
    );
}

#[test]
fn expression_errors() {
    // #403: `{}` holds no expression, so it is a literal brace pair, as in an empty JSON
    // object: each brace is one U0007 with its edit.
    assert_eq!(
        errors("f() -> Int = \"{}\"\n"),
        [
            "U0007 3:15 unescaped `{` in text, where no interpolation can start; write `\\{`",
            "U0007 3:16 unescaped `}` in text; write `\\}`"
        ]
    );
    assert_eq!(
        errors("f() -> Int = 1 +\n"),
        ["U0101 3:17 unexpected end of line, expected an expression"]
    );
    assert_eq!(
        errors("f() -> Int = a + if b then 1 else 2\n"),
        ["U0101 3:18 unexpected keyword `if`, expected an expression"]
    );
    assert_eq!(
        errors("f() -> Int = checked(x, it > 0) else Bad\n"),
        ["U0101 3:38 unexpected `Bad`, expected `fail`"]
    );
    assert_eq!(
        errors("f() -> Int = 12abc\n"),
        ["U0101 3:16 unexpected `abc`, expected end of line"]
    );
}

// ----- literals Una does not have (#12, review R-045) -----

#[test]
fn a_hexadecimal_exponent_or_fractional_literal_is_named_in_the_message() {
    // D-016: a name right after digits stays a name, and U0101 stays at it; the message names
    // the literal it was meant to be.
    let cases = [
        (
            "0x1F",
            "3:15",
            "`0x1F` is a hexadecimal literal; Una's integer literals are decimal",
        ),
        (
            "0b101",
            "3:15",
            "`0b101` is a binary literal; Una's integer literals are decimal",
        ),
        (
            "0o17",
            "3:15",
            "`0o17` is an octal literal; Una's integer literals are decimal",
        ),
        (
            "1e5",
            "3:15",
            "`1e5` is a literal with an exponent; Una's integer literals are decimal",
        ),
        (
            "3.14",
            "3:16",
            "`3.14` is a fractional literal; Una's integer literals are decimal",
        ),
        (
            "1.0e3",
            "3:16",
            "`1.0e3` is a fractional literal; Una's integer literals are decimal",
        ),
    ];
    for (literal, at, message) in cases {
        assert_eq!(
            errors(&format!("f() -> Int = {literal}\n")),
            [format!("U0101 {at} {message}")],
            "{literal}"
        );
    }
    // Names after digits that are no such prefix keep the message they had.
    assert_eq!(
        errors("f() -> Int = 12abc\n"),
        ["U0101 3:16 unexpected `abc`, expected end of line"]
    );
    // A space between the digits and the rest is no literal at all.
    assert_eq!(
        errors("f() -> Int = 0 x1\n"),
        ["U0101 3:16 unexpected `x1`, expected end of line"]
    );
}

// ----- reserved words (spec/01 §5, approved M1 plan) -----

#[test]
fn reserved_words_are_skipped_before_an_expression_and_read_as_names_elsewhere() {
    let skipped = parse_source("module t\n\npub double(x: Int) -> Int = return x * 2\n");
    assert_eq!(
        skipped.errors,
        ["U0103 3:29 `return` is not a keyword in Una; the last expression is the result"]
    );
    assert_eq!(
        skipped.tree,
        "(module t)\n(fn pub double ((x {Int})) {Int} (* x 2))\n"
    );
    let named = parse_source("module t\n\npub table(match: Int) -> Int = match\n");
    assert_eq!(
        named.errors,
        [
            "U0103 3:5 `table` is reserved for a later edition (platform)",
            "U0103 3:11 `match` is not a keyword in Una; use `when`",
            "U0103 3:32 `match` is not a keyword in Una; use `when`",
        ]
    );
    assert_eq!(
        named.tree,
        "(module t)\n(fn pub table ((match {Int})) {Int} match)\n"
    );
}

#[test]
fn contextual_keywords_are_names_outside_their_blocks() {
    assert_eq!(
        tree("route(source: Text, to: Text, via: Int, status: Bool) -> Text = source\n"),
        "(fn route ((source {Text}) (to {Text}) (via {Int}) (status {Bool})) {Text} source)\n"
    );
}

// ----- recovery (compiler/architecture.md §5, approved M1 plan) -----

#[test]
fn one_error_per_declaration_and_parsing_resumes_at_the_next() {
    let parsed = parse_source(
        "module t\n\npub one() Int = 1 1\n\npub two() -> Int = 2\n\npub three(x: Int) -> Int\n  requires x >\n=\n  x x\n\nfour[T](x: T) -> T = x\n",
    );
    assert_eq!(
        parsed.errors,
        [
            "U0101 3:11 unexpected `Int`, expected `->` and the result type",
            "U0101 8:15 unexpected end of line, expected an expression",
        ]
    );
    let names: Vec<String> = parsed
        .tree
        .lines()
        .skip(1)
        .map(|line| line.split(' ').take(3).collect::<Vec<_>>().join(" "))
        .collect();
    assert_eq!(names, ["(fn pub two", "(fn four [T]"]);
}

#[test]
fn recovery_resumes_only_at_a_line_that_starts_a_declaration() {
    // After the error in `f`, recovery stops at `g(`; after the error in `g`, it skips the
    // indented line and `value`, which is not followed by `(` or `[`, and stops at `h(`.
    let parsed = parse_source(
        "module t\n\nf() -> Int\n=\n  x x\ng() -> Int\n  = 1\nvalue\nh() -> Int = 3\n",
    );
    assert_eq!(
        parsed.errors,
        [
            "U0101 5:5 unexpected `x`, expected end of line",
            "U0101 7:3 unexpected `=`, expected a clause (`requires`, `ensures`, `uses`, `fails` or `derived from`)",
        ]
    );
    assert_eq!(parsed.tree, "(module t)\n(fn h () {Int} 3)\n");
}

#[test]
fn a_lexical_error_explains_the_parse_errors_of_its_declaration() {
    // The parser's U0101 after a lexical error in the same declaration is not
    // reported; a later declaration still reports its own.
    assert_eq!(
        errors("f(a: Bool, b: Bool) -> Bool = a & b\n\ng() -> Int = 1 1\n"),
        [
            "U0001 3:33 invalid character `&`; use `and`",
            "U0101 5:16 unexpected `1`, expected end of line",
        ]
    );
    assert_eq!(
        errors("f() -> Int = (1 +\n\ng() -> Int = 2\n"),
        ["U0101 3:14 unclosed `(` before the next declaration"]
    );
    assert_eq!(
        errors("f() -> Int\n=\n\t1 +\n"),
        ["U0002 5:1 tab in indentation; indentation uses spaces, two per level"]
    );
}

// ----- doc comments (spec/01 §3, ADR 0017) -----

#[test]
fn doc_comments_attach_to_the_next_declaration() {
    // Every doc comment between the previous token and the declaration
    // attaches, even across blank lines and plain comments.
    let parsed = parse_source(
        "--- Orphan: before the module header.\nmodule t\n\n--- One.\n--- Two.\n\n-- plain\n--- Three.\npub f() -> Int\n=\n  --- Orphan: inside a body.\n  1\n\npub g() -> Int = 2\n--- Orphan: at the end of the file.\n",
    );
    assert_eq!(parsed.errors, Vec::<String>::new());
    let text = parsed.db.files().next().unwrap().1.text().to_owned();
    let docs: Vec<Vec<&str>> = parsed
        .file
        .decls
        .iter()
        .map(|decl| {
            decl.docs
                .iter()
                .map(|comment| &text[comment.span.start..comment.span.end])
                .collect()
        })
        .collect();
    assert_eq!(docs, [vec!["--- One.", "--- Two.", "--- Three."], vec![]]);
    assert_eq!(parsed.file.comments.len(), 7);
}

#[test]
fn doc_comments_of_a_broken_declaration_do_not_move() {
    let parsed =
        parse_source("module t\n\n--- For f.\npub f() Int = 1\n\n--- For g.\npub g() -> Int = 2\n");
    assert_eq!(parsed.errors.len(), 1);
    let text = parsed.db.files().next().unwrap().1.text().to_owned();
    let docs: Vec<&str> = parsed.file.decls[0]
        .docs
        .iter()
        .map(|comment| &text[comment.span.start..comment.span.end])
        .collect();
    assert_eq!(docs, ["--- For g."]);
}

// ----- spans -----

#[test]
fn spans_cover_their_constructs() {
    let source = "module t\n\npub f(x: Int) -> Int\n  requires x > 0\n=\n  x + 1\n\n";
    let parsed = parse_source(source);
    let text = |span: una_syntax::Span| source[span.start..span.end].to_owned();
    let decl = &parsed.file.decls[0];
    assert_eq!(
        text(decl.span),
        "pub f(x: Int) -> Int\n  requires x > 0\n=\n  x + 1"
    );
    let DeclKind::Function(function) = &decl.kind else {
        panic!("not a function");
    };
    assert_eq!(text(function.clauses[0].keyword), "requires");
    assert_eq!(text(function.clauses[0].span), "requires x > 0");
    assert_eq!(text(function.params[0].span), "x: Int");
    let una_syntax::ast::Body::Block(block) = &function.body else {
        panic!("not a block");
    };
    let Stmt::Handled(handled) = &block.stmts[0] else {
        panic!("not a statement");
    };
    assert_eq!(text(handled.expr.span), "x + 1");
    let ExprKind::Binary { left, right, .. } = &handled.expr.kind else {
        panic!("not a binary expression");
    };
    assert_eq!(
        (text(left.span), text(right.span)),
        ("x".to_owned(), "1".to_owned())
    );
    let module = parsed.file.module.as_ref().unwrap();
    assert_eq!(text(module.span), "module t");
}

#[test]
fn an_error_at_the_end_of_the_file_points_one_past_the_last_character() {
    assert_eq!(
        errors("pub f() -> Int"),
        ["U0101 3:15 unexpected end of file, expected `=` and the body"]
    );
    assert_eq!(
        errors("pub f() -> Int\n"),
        ["U0101 4:1 unexpected end of file, expected `=` and the body"]
    );
    assert_eq!(
        errors("pub f() -> Int\n=\n"),
        ["U0101 5:1 unexpected end of file, expected an indented block"]
    );
}

#[test]
fn nested_patterns_are_not_supported() {
    // Benchmark finding B-001: `spec/02 §6` allows only names in the fields of a variant
    // pattern, and a nested pattern gets a message that says so.
    let message = "nested patterns are not supported in v0.1";
    for (pattern, column) in [
        ("is Some(Pair(x: a, y: b))", 13),
        ("is Ok(Some(v))", 11),
        ("is Ok(value: Some(v))", 18),
        ("is Some(value: is 0)", 20),
    ] {
        assert_eq!(
            errors(&format!(
                "f(v: T) -> Int\n=\n  when v\n    {pattern} -> 1\n"
            )),
            [format!("U0101 6:{column} {message}")],
            "{pattern}"
        );
    }
    let mut db = SourceDb::new();
    let id = db.add(
        "t.una",
        b"module t\n\nf(v: T) -> Int\n=\n  when v\n    is Ok(Some(v)) -> 1\n",
    );
    let mut interner = Interner::new();
    let lexed = lex(db.file(id), id, &mut interner);
    let parsed = parse(&lexed, id, &interner);
    assert_eq!(parsed.errors.len(), 1);
    assert_eq!(parsed.errors[0].repairs.len(), 1);
    let repair = &parsed.errors[0].repairs[0];
    assert_eq!(
        repair.description,
        "bind the field to a name, as in `is Ok(inner)`, then match that name in a second `when` inside the arm"
    );
    assert_eq!(repair.edit, None);
    // The two-step shape the repair describes parses.
    tree(
        "f(v: T) -> Int\n=\n  when v\n    is Ok(inner) ->\n      when inner\n        is Some(w) -> w\n        else -> 0\n    else -> 0\n",
    );
}

/// A repair: its description and the replacement of its edit, if any.
type Shown = (String, Option<String>);

/// The code, message and repairs of every lexical and syntax error of `body`, after `module t`.
fn repairs(body: &str) -> Vec<(String, Vec<Shown>)> {
    let text = format!("module t\n\n{body}");
    let mut db = SourceDb::new();
    let id = db.add("t.una", text.as_bytes());
    let mut interner = Interner::new();
    let lexed = lex(db.file(id), id, &mut interner);
    let parsed = parse(&lexed, id, &interner);
    lexed
        .errors
        .iter()
        .chain(&parsed.errors)
        .map(|error| {
            (
                format!("{} {}", error.code, error.message),
                error
                    .repairs
                    .iter()
                    .map(|r| (r.description.clone(), r.edit.as_ref().map(|e| e.1.clone())))
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn builder_mistakes_get_a_hint() {
    // Issue #75 (UI-006): parse errors that the Gate 1 task builders met with no guidance.
    let advice = |body: &str| {
        let found = repairs(body);
        assert_eq!(found.len(), 1, "{found:?}");
        let (_, repairs) = &found[0];
        assert_eq!(repairs.len(), 1, "{repairs:?}");
        assert_eq!(repairs[0].1, None, "advice has no edit");
        repairs[0].0.clone()
    };
    assert_eq!(
        advice("f(x: Int) -> Int\n  fails Bad\n=\n  let reason = Broke\n  fail reason\n"),
        "`fail` takes a variant of the error kind, as in `fail Broke`, not a stored value; fail where the error is found, or pass a call's failure on with `if error pass`"
    );
    assert_eq!(
        advice("f(o: Option[Int]) -> Int = when o\n  is Some(x) -> x\n  is None -> 0\n"),
        "`when` must start its own line: write the long form, with `=` on its own line and the `when` indented below it, or bind it with `let`"
    );
    // ADR 0096: a guard with the rest of the body after it gets the exact rewrite first, and
    // keeps the advice second.
    let guard = repairs("f(x: Int) -> Int\n  fails Bad\n=\n  if x < 0 then fail Broke\n  x\n");
    assert_eq!(guard.len(), 1, "{guard:?}");
    assert_eq!(
        guard[0].1,
        vec![
            (
                "make the rest of the body the `else` branch of an `if` with blocks".to_owned(),
                Some("  if x < 0\n    fail Broke\n  else\n    x\n".to_owned())
            ),
            (
                "an `if` with `then` needs `else`; to fail early, make the rest of the body its `else` branch, as in `if x < 0 then fail Broke else x`, or use the block form, whose `else` block holds the rest".to_owned(),
                None
            ),
        ]
    );
    // The shapes the hints describe parse.
    tree("f(o: Option[Int]) -> Int\n=\n  when o\n    is Some(x) -> x\n    is None -> 0\n");
    tree("f(x: Int) -> Int\n  fails Bad\n=\n  if x < 0 then fail Broke else x\n");
    tree("f(x: Int) -> Int\n  fails Bad\n=\n  if x < 0\n    fail Broke\n  else\n    x\n");
}

#[test]
fn a_word_reserved_for_a_later_version_gets_a_rename_repair() {
    // Issue #75 (UI-006): `table`, `all` and `service` are natural names; the repair renames
    // each occurrence the same way, so applying them all keeps the program consistent.
    assert_eq!(
        repairs("pub f(table: Int) -> Int = table\n"),
        [
            (
                "U0103 `table` is reserved for a later edition (platform)".to_owned(),
                vec![(
                    "rename it to `the_table`".to_owned(),
                    Some("the_table".to_owned())
                )],
            ),
            (
                "U0103 `table` is reserved for a later edition (platform)".to_owned(),
                vec![(
                    "rename it to `the_table`".to_owned(),
                    Some("the_table".to_owned())
                )],
            ),
        ]
    );
    tree("pub f(the_table: Int) -> Int = the_table\n");
    // Where a kind name is expected, the rename capitalizes the word.
    assert_eq!(
        repairs("pub kind service = A | B\n"),
        [(
            "U0103 `service` is reserved for a later edition (platform)".to_owned(),
            vec![(
                "rename it to `Service`".to_owned(),
                Some("Service".to_owned())
            )],
        )]
    );
    tree("pub kind Service = A | B\n");
    // V12-002: the new name clashes with no name of the file, and is the same for every
    // occurrence of the word, capitalized when any of them names a kind.
    let renamed = |body: &str| -> Vec<Option<String>> {
        repairs(body)
            .into_iter()
            .flat_map(|(_, repairs)| repairs.into_iter().map(|(_, edit)| edit))
            .collect()
    };
    assert_eq!(
        renamed("pub f(table: Int, the_table: Int) -> Int = table + the_table\n"),
        [
            Some("the_table_2".to_owned()),
            Some("the_table_2".to_owned())
        ]
    );
    assert_eq!(
        renamed("pub kind Service = A | B\n\npub kind service = C | D\n"),
        [Some("Service2".to_owned())]
    );
    assert_eq!(
        renamed("pub kind service = A | B\n\npub f(x: service) -> service = x\n"),
        [
            Some("Service".to_owned()),
            Some("Service".to_owned()),
            Some("Service".to_owned())
        ]
    );
    // A module path is not renamed by an edit: its file must move too (U1008).
    for header in ["module app.table", "module table", "import app.table (f)"] {
        let text = format!("{header}\n");
        let mut db = SourceDb::new();
        let id = db.add("t.una", text.as_bytes());
        let mut interner = Interner::new();
        let lexed = lex(db.file(id), id, &mut interner);
        assert_eq!(lexed.errors.len(), 1, "{header}");
        let repairs = &lexed.errors[0].repairs;
        assert_eq!(repairs.len(), 1, "{header}");
        assert_eq!(repairs[0].edit, None, "{header}");
        assert_eq!(
            repairs[0].description,
            "rename the module so that no part of its path is `table`, and move its file to match",
            "{header}"
        );
    }
    // #125 (R-2): a name an `import` line lists is not part of the module path, so it gets the
    // rename edit, the same as its declaration and its uses.
    assert_eq!(
        repairs("import app.events (table)\n\npub f() -> Int = table(1)\n"),
        [
            (
                "U0103 `table` is reserved for a later edition (platform)".to_owned(),
                vec![(
                    "rename it to `the_table`".to_owned(),
                    Some("the_table".to_owned())
                )],
            ),
            (
                "U0103 `table` is reserved for a later edition (platform)".to_owned(),
                vec![(
                    "rename it to `the_table`".to_owned(),
                    Some("the_table".to_owned())
                )],
            ),
        ]
    );
    // A word that names a construct Una writes differently says what to write instead.
    assert_eq!(
        repairs("pub f(x: Int) -> Int = return x\n"),
        [(
            "U0103 `return` is not a keyword in Una; the last expression is the result".to_owned(),
            vec![],
        )]
    );
}

#[test]
fn literal_patterns_are_not_supported() {
    // Issue #73 (UI-001): a literal in a field of a variant pattern is named as such. A second
    // `when` matches it with a literal arm and `else`; a negative number, which a pattern
    // cannot write, gets only the comparison.
    let message = "literal patterns are not supported in v0.1";
    for (pattern, column) in [
        ("is Some(0)", 13),
        ("is Some(-1)", 13),
        ("is Some(true)", 13),
        ("is Pair(x, y: \"a\")", 19),
    ] {
        assert_eq!(
            errors(&format!(
                "f(v: T) -> Int\n=\n  when v\n    {pattern} -> 1\n"
            )),
            [format!("U0101 6:{column} {message}")],
            "{pattern}"
        );
    }
    let shown = |pattern: &str| -> Vec<String> {
        let text = format!("module t\n\nf(v: T) -> Int\n=\n  when v\n    {pattern} -> 1\n");
        let mut db = SourceDb::new();
        let id = db.add("t.una", text.as_bytes());
        let mut interner = Interner::new();
        let lexed = lex(db.file(id), id, &mut interner);
        let parsed = parse(&lexed, id, &interner);
        parsed.errors[0]
            .repairs
            .iter()
            .map(|repair| {
                assert_eq!(repair.edit, None);
                repair.description.clone()
            })
            .collect()
    };
    assert_eq!(
        shown("is Some(0)"),
        [
            "bind the field to a name, as in `is Some(inner)`, then match that name in a second `when` inside the arm, with `is 0 -> ...` and `else -> ...`",
            "bind the field to a name, as in `is Some(inner)`, then compare that name inside the arm, as in `if inner == 0 then ... else ...`",
        ]
    );
    assert_eq!(
        shown("is Some(-1)"),
        [
            "bind the field to a name, as in `is Some(inner)`, then compare that name inside the arm, as in `if inner == -1 then ... else ...`"
        ]
    );
    // The shapes the repairs describe parse, for every literal a pattern can write.
    for literal in ["0", "true", "\"a\""] {
        tree(&format!(
            "f(v: T) -> Int\n=\n  when v\n    is Some(inner) ->\n      when inner\n        is {literal} -> 1\n        else -> 2\n    else -> 0\n"
        ));
    }
    tree(
        "f(v: T) -> Int\n=\n  when v\n    is Some(inner) -> if inner == -1 then 1 else 2\n    else -> 0\n",
    );
}

#[test]
fn a_handling_clause_on_the_line_of_its_expression_says_where_it_goes() {
    // #211: `spec/02 §6` puts each handling clause on its own line, indented under the
    // expression it handles. The same-line form gets a message naming the rule and an edit
    // that moves the clause down.
    let message =
        "U0101 a handling clause goes on its own line, indented under the expression it handles";
    assert_eq!(
        repairs("f(x: Int) -> Int\n  fails Bad\n=\n  let y = g(x) if error pass\n  y\n"),
        [(
            message.to_owned(),
            vec![(
                "move the clause to its own line".to_owned(),
                Some("\n    if error pass".to_owned())
            )]
        )]
    );
    assert_eq!(
        repairs("f(o: Option[Int]) -> Int\n=\n  when o\n    is Some(v) ->\n      g(v) if none use 0\n    else -> 0\n")[0].1,
        [(
            "move the clause to its own line".to_owned(),
            Some("\n        if none use 0".to_owned())
        )]
    );
    // Two clauses on the line: each gets its own line (#257).
    assert_eq!(
        repairs("f(x: Int) -> Int\n=\n  let y = g(x) if error use None if none use 0\n  y\n")[0].1,
        [(
            "move each handling clause to its own line".to_owned(),
            Some("\n    if error use None\n    if none use 0".to_owned())
        )]
    );
    // The repaired forms parse.
    tree("f(x: Int) -> Int\n  fails Bad\n=\n  let y = g(x)\n    if error pass\n  y\n");
    // `if` after an expression that is not a handling clause keeps the general message.
    assert_eq!(
        errors("f(x: Int) -> Int\n=\n  let y = g(x) if x > 0\n  y\n"),
        ["U0101 5:16 unexpected keyword `if`, expected end of line"]
    );
}

#[test]
fn a_reserved_keyword_as_a_binding_name_says_so() {
    // #206: `let result = ...` named only the unexpected keyword. The message says what the
    // word is reserved for; a `let` whose uses are all in its block gets one edit that renames
    // the binding and its uses, and the other binding positions get the name as advice.
    let message = "U0101 `result` is reserved: it names a function's result in `ensures`, so it cannot name a binding";
    assert_eq!(
        repairs("f() -> Int\n=\n  let result = 1\n  result + 1\n"),
        [(
            message.to_owned(),
            vec![(
                "rename the binding and its uses to `outcome`".to_owned(),
                Some("outcome = 1\n  outcome".to_owned())
            )]
        )]
    );
    // A name the file already uses is not proposed.
    assert_eq!(
        repairs("f(outcome: Int) -> Int\n=\n  let result = outcome\n  result\n")[0].1,
        [(
            "rename the binding and its uses to `outcome2`".to_owned(),
            Some("outcome2 = outcome\n  outcome2".to_owned())
        )]
    );
    // Without a later use, the edit renames the binding alone.
    assert_eq!(
        repairs("f() -> Int\n=\n  let result = 1\n  2\n")[0].1,
        [(
            "rename the binding and its uses to `outcome`".to_owned(),
            Some("outcome".to_owned())
        )]
    );
    // `it` also names the value of a `checked` predicate or a `where` clause in the same
    // block, so renaming its uses could change their meaning: advice only.
    assert_eq!(
        repairs("f(y: Int) -> Int\n  fails Bad\n=\n  let it = 1\n  let z = checked(y, it > 0) else fail Bad\n  z + it\n")[0].1,
        [("choose another name, such as `item`".to_owned(), None)]
    );
    // Parameters, loop variables and other keywords get advice.
    assert_eq!(
        repairs("f(result: Int) -> Int = 1\n"),
        [(
            message.to_owned(),
            vec![("choose another name, such as `outcome`".to_owned(), None)]
        )]
    );
    assert_eq!(
        repairs("f(xs: List[Int]) -> Int = xs.map(each: fn(it) -> 1).sum\n"),
        [(
            "U0101 `it` is reserved: it names the value that a `where` clause or a `checked` predicate constrains, so it cannot name a binding".to_owned(),
            vec![("choose another name, such as `item`".to_owned(), None)]
        )]
    );
    assert_eq!(
        repairs("f() -> Int\n=\n  let when = 1\n  when\n"),
        [(
            "U0101 `when` is a reserved keyword, so it cannot name a binding".to_owned(),
            vec![("choose another name, such as `when_value`".to_owned(), None)]
        )]
    );
}

#[test]
fn a_declaration_keyword_as_a_local_name_is_renamed_through_its_scope_743() {
    // ADR 0096, amendment of 2026-10-15: none of these keywords can stand in a function's
    // body, so within a binding's scope every token of the word is a use, and the rename is
    // exact. It covers interpolation holes and leaves a name after `.` and a label alone.
    let message = "U0101 `op` is a reserved keyword, so it cannot name a binding";
    let renamed = |edit: &str| {
        vec![(
            "rename the binding and its uses to `op_value`".to_owned(),
            Some(edit.to_owned()),
        )]
    };
    assert_eq!(
        repairs(
            "f(n: Int) -> Text\n=\n  let op = n + 1\n  let s = \"{op}!\"\n  g(x: op.op, op: op)\n"
        ),
        [(
            message.to_owned(),
            renamed(
                "op_value = n + 1\n  let s = \"{op_value}!\"\n  g(x: op_value.op, op: op_value"
            )
        )]
    );
    // A lambda's parameter: its body, to the end of the call around it.
    assert_eq!(
        repairs("f(xs: List[Int]) -> Int = xs.map(each: fn(op) -> op + 1).sum + g(op)\n")[0].1,
        renamed("op_value) -> op_value")
    );
    // A `when` arm's binding: the arm's body, on its line or in its block.
    assert_eq!(
        repairs("f(o: Option[Int]) -> Int\n=\n  when o\n    is Some(value: op) -> op\n    is None -> 0\n")[0].1,
        renamed("op_value) -> op_value")
    );
    assert_eq!(
        repairs("f(o: Option[Int]) -> Int\n=\n  when o\n    is Some(value: op) ->\n      let n = op\n      n + op\n    is None -> 0\n")[0].1,
        renamed("op_value) ->\n      let n = op_value\n      n + op_value")
    );
    // A handler operation's parameter: the capability's operation names it, not a caller.
    assert_eq!(
        repairs("handler memory(limit: Int) for Store\n  op put(key, op) = op + limit\n  op size() = limit\n")[0].1,
        renamed("op_value) = op_value")
    );
    // The fresh name avoids every name of the file (rule 3a).
    assert_eq!(
        repairs("op_value() -> Int = 1\n\nf() -> Int\n=\n  let op = 2\n  op + op_value()\n")[0].1,
        vec![(
            "rename the binding and its uses to `op_value2`".to_owned(),
            Some("op_value2 = 2\n  op_value2".to_owned())
        )]
    );
    // A function's or a handler declaration's parameter is named by its callers, or holds
    // the handler's own `op` declarations in its scope: advice only.
    let advice = vec![("choose another name, such as `op_value`".to_owned(), None)];
    assert_eq!(repairs("f(op: Int) -> Int = op\n")[0].1, advice);
    assert_eq!(
        repairs("handler memory(op: Int) for Store\n  op size() = 1\n")[0].1,
        advice
    );
    // Outside a function or handler operation body, such as in an example, advice only.
    assert_eq!(
        repairs("example \"e\"\n  do f(fn(op) -> op)\n  expect true\n")[0].1,
        advice
    );
    // A second binding of the word in the scope: advice only.
    assert_eq!(
        repairs("f() -> Int\n=\n  let op = 1\n  let op = 2\n  op\n")[0].1,
        advice
    );
}

#[test]
fn a_percent_sign_gets_the_remainder_rule_and_no_edit_743() {
    // ADR 0096, amendment of 2026-10-15: `mod` is Euclidean and `%` truncates, so a rewrite
    // could change the meaning for negative operands; the description gives both readings.
    let found = repairs("f(a: Int) -> Int = a % 3\n");
    assert!(
        found[0].0.starts_with("U0001 invalid character `%`"),
        "{found:?}"
    );
    assert_eq!(
        found[0].1,
        [(
            "Una writes the remainder as `mod`, which is Euclidean: `-7 mod 3 == 2`, where `%` in C, TypeScript and Rust gives `-1`; the two agree when both operands are non-negative".to_owned(),
            None
        )]
    );
    // A `%` in a text literal or a comment is not a token.
    assert_eq!(
        errors("f() -> Text = \"50%\" -- 50%\n"),
        Vec::<String>::new()
    );
}

#[test]
fn renames_are_chosen_across_the_files_of_a_package_125() {
    // #125 (R-1): a word gets one new name for the whole package, which no file has, so a
    // renamed declaration and its uses in another module never come to name another
    // declaration. Here `a` has `the_table` and `the_table_2`, `b` imports `table` and `c`
    // has a private `the_table_3`: every occurrence becomes `the_table_4`.
    let files = [
        "module a\n\npub the_table(x: Int) -> Int = x + 100\n\nthe_table_2() -> Int = 2\n\npub table(x: Int) -> Int = x\n",
        "module b\n\nimport a (table)\n\npub g() -> Int = table(1)\n",
        "module c\n\nthe_table_3() -> Int = 3\n",
    ];
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let ids: Vec<_> = files
        .iter()
        .enumerate()
        .map(|(n, text)| db.add(format!("{n}.una"), text.as_bytes()))
        .collect();
    let mut lexed: Vec<_> = ids
        .iter()
        .map(|&id| lex(db.file(id), id, &mut interner))
        .collect();
    // One file at a time, `a` and `b` would choose different names.
    let alone: Vec<Option<String>> = lexed
        .iter()
        .flat_map(|l| l.errors.iter())
        .map(|e| e.repairs[0].edit.as_ref().map(|(_, name)| name.clone()))
        .collect();
    assert_eq!(
        alone,
        [
            Some("the_table_3".to_owned()),
            Some("the_table".to_owned()),
            Some("the_table".to_owned())
        ]
    );
    una_syntax::rename_across(&mut lexed, &interner);
    let together: Vec<Option<String>> = lexed
        .iter()
        .flat_map(|l| l.errors.iter())
        .map(|e| {
            assert_eq!(e.repairs.len(), 1);
            e.repairs[0].edit.as_ref().map(|(_, name)| name.clone())
        })
        .collect();
    assert_eq!(together, vec![Some("the_table_4".to_owned()); 3]);
}

/// The code, message and repairs of every lexical and syntax error of `body`, after
/// `module t`, under `keywords`.
fn repairs_in(body: &str, keywords: KeywordSet) -> Vec<(String, Vec<Shown>)> {
    let text = format!("module t\n\n{body}");
    let mut db = SourceDb::new();
    let id = db.add("t.una", text.as_bytes());
    let mut interner = Interner::new();
    let lexed = lex_in(db.file(id), id, &mut interner, keywords);
    let parsed = parse(&lexed, id, &interner);
    lexed
        .errors
        .iter()
        .chain(&parsed.errors)
        .map(|error| {
            (
                format!("{} {}", error.code, error.message),
                error
                    .repairs
                    .iter()
                    .map(|r| (r.description.clone(), r.edit.as_ref().map(|e| e.1.clone())))
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn edition_0_4_advises_edition_0_5_for_the_moved_syntax() {
    // ADR 0139 rule 7, #1743: `simd` as a name, `Decimal[s]` and the `with:` and `and:` labels
    // moved to edition 0.5 with no diagnostic of their own; in edition 0.4 each keeps the
    // error edition 0.3 gives it, with advice (no edit) to raise the edition. Edition 0.3
    // gives the same errors without the advice, and edition 0.5 none.
    let advice =
        |what: &str| format!("{what} (ADR 0139): write `edition 0.5` in ita.una to use it");
    let simd = "f() -> Int\n=\n  let simd = 4\n  simd\n";
    let rename = (
        "rename it to `the_simd`".to_owned(),
        Some("the_simd".to_owned()),
    );
    let reserved = "U0103 `simd` is reserved for v0.2 (systems profile)".to_owned();
    assert_eq!(
        repairs_in(simd, KeywordSet::V0_4),
        vec![
            (
                reserved.clone(),
                vec![
                    rename.clone(),
                    (advice("`simd` is a name from edition 0.5"), None)
                ]
            );
            2
        ]
    );
    assert_eq!(
        repairs_in(simd, KeywordSet::V0_3),
        vec![(reserved, vec![rename]); 2]
    );
    // Edition 0.5 binds with `const` (ADR 0137).
    assert_eq!(
        repairs_in(&simd.replace("let ", "const "), KeywordSet::V0_5),
        vec![]
    );
    // Controls (review of #1758): in edition 0.4 another reserved word keeps exactly its
    // rename, and an integer argument of another type keeps no repair.
    assert_eq!(
        repairs_in(
            "f() -> Int\n=\n  let within = 4\n  within\n",
            KeywordSet::V0_4
        )
        .into_iter()
        .map(|(_, repairs)| repairs)
        .collect::<Vec<_>>(),
        vec![
            vec![(
                "rename it to `the_within`".to_owned(),
                Some("the_within".to_owned())
            )];
            2
        ]
    );
    for other in ["f(x: List[2]) -> Int = 0\n", "f(x: Option[2]) -> Int = 0\n"] {
        let found = repairs_in(other, KeywordSet::V0_4);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].0.starts_with("U0101 "), "{found:?}");
        assert!(found[0].1.is_empty(), "{other}: {found:?}");
    }
    let decimal = "f(x: Decimal[2]) -> Int = 0\n";
    let found = repairs_in(decimal, KeywordSet::V0_4);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].0.starts_with("U0101 "), "{found:?}");
    assert_eq!(
        found[0].1,
        vec![(advice("`Decimal[s]` exists from edition 0.5"), None)]
    );
    let earlier = repairs_in(decimal, KeywordSet::V0_3);
    assert_eq!(earlier.len(), 1, "{earlier:?}");
    assert_eq!(earlier[0].0, found[0].0, "the error is edition 0.3's");
    assert!(earlier[0].1.is_empty(), "{earlier:?}");
    assert_eq!(repairs_in(decimal, KeywordSet::V0_5), vec![]);
    for label in ["with", "and"] {
        let call = format!("f(a: Int) -> Int = g(a, {label}: a)\n");
        let message = format!("U0101 unexpected keyword `{label}`, expected an expression");
        assert_eq!(
            repairs_in(&call, KeywordSet::V0_4),
            vec![(
                message.clone(),
                vec![(
                    advice(&format!("the label `{label}:` exists from edition 0.5")),
                    None
                )]
            )]
        );
        assert_eq!(repairs_in(&call, KeywordSet::V0_3), vec![(message, vec![])]);
        assert_eq!(repairs_in(&call, KeywordSet::V0_5), vec![]);
    }
}

#[test]
fn tuple_advice_names_const_and_var_only_in_edition_0_5() {
    // ADR 0137 rule 1, review of #2258 (L2): before edition 0.5 a line that starts with
    // `const (` or `var (` gets only the diagnostic of its edition and the help of rule 1, never
    // the ADR 0096 advice for a `let` line; in edition 0.5 a `const (` line gets that advice.
    let help = "`const` and `var` are keywords of edition 0.5";
    for keywords in [KeywordSet::V0_2, KeywordSet::V0_4] {
        for line in ["const (a, b) = g()", "var (a, b) = g()"] {
            let found = repairs_in(&format!("f() -> Int\n=\n  {line}\n  a\n"), keywords);
            for (message, repairs) in &found {
                for (description, _) in repairs {
                    assert!(
                        description == help || description.starts_with("rename"),
                        "{keywords:?} {line}: {message}: {description}"
                    );
                }
            }
        }
    }
    let tuple = repairs_in(
        "f() -> Int\n=\n  const (a, b) = g()\n  a\n",
        KeywordSet::V0_5,
    );
    let with_let = repairs_in("f() -> Int\n=\n  let (a, b) = g()\n  a\n", KeywordSet::V0_4);
    assert_eq!(
        tuple.iter().map(|(_, r)| r.clone()).collect::<Vec<_>>(),
        with_let.iter().map(|(_, r)| r.clone()).collect::<Vec<_>>(),
        "{tuple:?} {with_let:?}"
    );
}
