//! The parser against `spec/02`, the precedence table of `spec/01 §7`, U0102 and the recovery
//! rules of the approved M1 plan.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use pretty_assertions::assert_eq;
use una_syntax::ast::{DeclKind, ExprKind, File, Stmt};
use una_syntax::{Interner, SourceDb, dump, lex, parse};

struct Parsed {
    file: File,
    tree: String,
    errors: Vec<String>,
    db: SourceDb,
}

fn parse_source(text: &str) -> Parsed {
    let mut db = SourceDb::new();
    let id = db.add("t.una", text.as_bytes());
    let mut interner = Interner::new();
    let lexed = lex(db.file(id), id, &mut interner);
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
        ["U0101 3:11 unexpected keyword `if`, expected a parameter name"]
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
fn expression_errors() {
    assert_eq!(
        errors("f() -> Int = \"{}\"\n"),
        ["U0101 3:16 unexpected `}`, expected an expression"]
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
            "U0103 3:5 `table` is reserved for v0.4 (platform)",
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
