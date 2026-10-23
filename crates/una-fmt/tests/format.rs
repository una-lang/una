//! The canonical form of ADR 0020, rule by rule.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use pretty_assertions::assert_eq;
use una_syntax::{Interner, KeywordSet, SourceDb, lex, lex_in, parse};

/// The canonical form of `text`, which must lex and parse without errors; also checks that
/// formatting the result again changes nothing.
fn fmt(text: &str) -> String {
    let once = format_once(text);
    assert_eq!(format_once(&once), once, "formatting is not idempotent");
    once
}

fn fmt_in(text: &str, keywords: KeywordSet) -> String {
    let mut db = SourceDb::new();
    let id = db.add("t.una", text.as_bytes());
    let mut interner = Interner::new();
    let lexed = lex_in(db.file(id), id, &mut interner, keywords);
    let parsed = parse(&lexed, id, &interner);
    assert!(
        lexed.errors.is_empty() && parsed.errors.is_empty(),
        "{:?} {:?}\n{text}",
        lexed.errors,
        parsed.errors
    );
    una_fmt::format(db.file(id), &lexed, &parsed.file, &interner)
}

fn format_once(text: &str) -> String {
    let mut db = SourceDb::new();
    let id = db.add("t.una", text.as_bytes());
    let mut interner = Interner::new();
    let lexed = lex(db.file(id), id, &mut interner);
    let parsed = parse(&lexed, id, &interner);
    assert!(
        lexed.errors.is_empty() && parsed.errors.is_empty(),
        "{:?} {:?}\n{text}",
        lexed.errors,
        parsed.errors
    );
    una_fmt::format(db.file(id), &lexed, &parsed.file, &interner)
}

/// The canonical form of the declarations `body`, after `module t` and a blank line.
fn decls(body: &str) -> String {
    fmt(&format!("module t\n\n{body}"))
        .strip_prefix("module t\n\n")
        .unwrap()
        .to_owned()
}

#[test]
fn canonical_text_is_unchanged() {
    let text = "module t\n\npub f(x: Int) -> Int = x + 1\n";
    assert_eq!(fmt(text), text);
}

#[test]
fn spacing_between_tokens() {
    let cases = [
        (
            "pub f(x:Int)->Int=x+1*2\n",
            "pub f(x: Int) -> Int = x + 1 * 2\n",
        ),
        (
            "f ( a : Int , b : Int ) -> Bool = a<b and not(b==a)\n",
            "f(a: Int, b: Int) -> Bool = a < b and not (b == a)\n",
        ),
        (
            "first [T] (xs : List [T]) -> Option [T] = xs . first ( )\n",
            "first[T](xs: List[T]) -> Option[T] = xs.first()\n",
        ),
        (
            "f(a: secret [PII, Location] Phone, b: [ Int ]) -> internal Hashed = h(a)\n",
            "f(a: secret[PII, Location] Phone, b: [Int]) -> internal Hashed = h(a)\n",
        ),
        ("kind P = Int in 0 .. 100\n", "kind P = Int in 0..100\n"),
        ("kind P = Int in - 5 ..< 5\n", "kind P = Int in -5..<5\n"),
        (
            "f(x: Int) -> Int = - 1 + - x - - x\n",
            "f(x: Int) -> Int = -1 + -x - -x\n",
        ),
        ("f(x: Int) -> Int = - - x\n", "f(x: Int) -> Int = - -x\n"),
        (
            "f(x: Int) -> Int = ( - x ) * 2\n",
            "f(x: Int) -> Int = (-x) * 2\n",
        ),
        (
            "f() -> Text = \"a { x . y } b\"\n",
            "f() -> Text = \"a {x.y} b\"\n",
        ),
        (
            "f() -> Text = \"{a}{ b }\\t\\u{41}\"\n",
            "f() -> Text = \"{a}{b}\\t\\u{41}\"\n",
        ),
        ("f() -> Int = 1_000_000\n", "f() -> Int = 1_000_000\n"),
        (
            "f(g: fn (Int) -> Int uses @log : Log [ write ] fails E) -> Int = g(1)\n",
            "f(g: fn(Int) -> Int uses @log: Log[write] fails E) -> Int = g(1)\n",
        ),
        (
            "f(x: Int) -> Int = checked (x , it > 0) else fail Bad ( at : 1 )\n",
            "f(x: Int) -> Int = checked(x, it > 0) else fail Bad(at: 1)\n",
        ),
        (
            "f(xs: List[Int]) -> List[Int] = map(xs, fn (x) -> x * 2)\n",
            "f(xs: List[Int]) -> List[Int] = map(xs, fn(x) -> x * 2)\n",
        ),
        (
            "f() -> Unit = @log . write ( \"a\" )\n",
            "f() -> Unit = @log.write(\"a\")\n",
        ),
        (
            "f(x: Int) -> Bool = x is Some and x in 0..9\n",
            "f(x: Int) -> Bool = x is Some and x in 0..9\n",
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(decls(source), expected, "{source}");
    }
}

#[test]
fn lines_continued_inside_brackets_are_joined() {
    assert_eq!(
        decls("f() -> Int = g(1,\n    2,\n  3)\n"),
        "f() -> Int = g(1, 2, 3)\n"
    );
    assert_eq!(
        decls("f() -> List[Int] = [\n  1,\n  2\n]\n"),
        "f() -> List[Int] = [1, 2]\n"
    );
}

#[test]
fn wraps_at_legal_argument_and_element_separators() {
    let a = "a".repeat(60);
    let b = "b".repeat(60);
    assert_eq!(
        decls(&format!("f() -> Int = g({a}, {b})\n")),
        format!("f() -> Int = g({a},\n  {b}\n)\n")
    );

    let a = "a".repeat(42);
    let b = "b".repeat(42);
    let c = "c".repeat(42);
    assert_eq!(
        decls(&format!("f() -> Int = g({a}, {b}, {c})\n")),
        format!("f() -> Int = g({a},\n  {b}, {c}\n)\n")
    );

    let a = "a".repeat(52);
    let b = "b".repeat(52);
    assert_eq!(
        decls(&format!("f() -> Int = g(h({a}, {b}), 1)\n")),
        format!("f() -> Int = g(h({a},\n    {b}\n  ), 1)\n")
    );

    let a = "a".repeat(50);
    let b = "b".repeat(50);
    let c = "c".repeat(50);
    let d = "d".repeat(50);
    let nested = decls(&format!("f() -> Int = outer(inner({a}, {b}), {c}, {d})\n"));
    assert_eq!(
        nested,
        format!("f() -> Int = outer(inner({a},\n    {b}\n  ),\n  {c},\n  {d}\n)\n")
    );
    assert_eq!(decls(&nested), nested);

    let values = format!(
        "[{}]",
        (0..24)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    assert_eq!(
        decls(&format!("f() -> List[Int] = {values}\n")),
        "f() -> List[Int] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,\n  22, 23\n]\n"
    );

    let arguments = (0..1_000)
        .map(|i| format!("item_{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let formatted = decls(&format!("f() -> Int = g({arguments})\n"));
    assert!(formatted.lines().all(|line| line.chars().count() <= 100));
    assert_eq!(decls(&formatted), formatted);
}

#[test]
fn wrapping_counts_unicode_scalars_and_does_not_split_indivisible_tokens() {
    let at_limit = format!(
        "f() -> Int = g(\"{}\", \"{}\")\n",
        "🙂".repeat(50),
        "🙂".repeat(28)
    );
    let at_limit = decls(&at_limit);
    assert_eq!(at_limit.lines().count(), 1);
    assert_eq!(at_limit.trim_end().chars().count(), 100);

    let over_limit = format!(
        "f() -> Int = g(\"{}\", \"{}\")\n",
        "🙂".repeat(50),
        "🙂".repeat(29)
    );
    let over_limit = decls(&over_limit);
    assert_eq!(over_limit.lines().count(), 3);
    assert!(over_limit.lines().all(|line| line.chars().count() <= 100));

    let indivisible = format!("f() -> Text = \"{}\"\n", "🙂".repeat(120));
    let formatted = decls(&indivisible);
    assert_eq!(formatted.lines().count(), 1);
    assert!(formatted.trim_end().chars().count() > 100);
    assert_eq!(decls(&formatted), formatted);
}

#[test]
fn wrapping_is_edition_independent_and_comments_keep_their_source_breaks() {
    let a = "a".repeat(60);
    let b = "b".repeat(60);
    let source = format!("module t\n\nf() -> Int = g({a}, {b})\n");
    let expected = format!("module t\n\nf() -> Int = g({a},\n  {b}\n)\n");
    for edition in [
        KeywordSet::V0_1,
        KeywordSet::V0_2,
        KeywordSet::V0_3,
        KeywordSet::V0_4,
    ] {
        assert_eq!(fmt_in(&source, edition), expected);
    }

    let comment = "-- keep this comment on its original physical line";
    assert_eq!(
        decls(&format!("f() -> Int = g({a},\n  {comment}\n  {b})\n")),
        format!("f() -> Int = g({a},\n  {comment}\n  {b})\n")
    );
}

#[test]
fn blank_lines_between_top_level_items() {
    assert_eq!(
        fmt(
            "module t\nimport a.b (x)\n\n\nimport c.d (y)\nf() -> Int = 1\ng() -> Int = 2\n\n\n\nh() -> Int = 3\n\n\n"
        ),
        "module t\n\nimport a.b (x)\nimport c.d (y)\n\nf() -> Int = 1\n\ng() -> Int = 2\n\nh() -> Int = 3\n"
    );
}

#[test]
fn blank_lines_inside_declarations() {
    assert_eq!(
        decls("f(x: Int) -> Int\n\n  requires x > 0\n\n=\n\n  let a = x\n\n\n  let b = a\n  b\n"),
        "f(x: Int) -> Int\n  requires x > 0\n=\n  let a = x\n\n  let b = a\n  b\n"
    );
    assert_eq!(
        decls("f(x: Int) -> Int\n=\n  when x\n\n    is 1 -> 1\n\n    else -> 2\n"),
        "f(x: Int) -> Int\n=\n  when x\n    is 1 -> 1\n\n    else -> 2\n"
    );
}

#[test]
fn a_function_without_clauses_whose_body_is_one_line_uses_the_short_form() {
    assert_eq!(
        decls("f(x: Int) -> Int\n=\n  x * 2\n"),
        "f(x: Int) -> Int = x * 2\n"
    );
    let long = [
        "f(x: Int) -> Int\n  requires x > 0\n=\n  x\n",
        "f(x: Int) -> Int\n=\n  let y = x\n  y\n",
        "f() -> Int\n=\n  g()\n    if error use 0\n",
        "f() -> Unit\n=\n  g() with @log = @audit\n",
        "f(x: Int) -> Int\n=\n  when x\n    else -> 1\n",
        "f(x: Int) -> Int\n=\n  if x > 0\n    1\n  else\n    2\n",
    ];
    for text in long {
        assert_eq!(decls(text), text);
    }
}

#[test]
fn predicates_handler_operations_and_arms_put_a_one_line_body_on_their_line() {
    assert_eq!(
        decls("predicate reflect small(n: Int) -> Bool =\n  n < 10\n"),
        "predicate reflect small(n: Int) -> Bool = n < 10\n"
    );
    assert_eq!(
        decls("handler h() for Log\n  op write(line) =\n    unit\n"),
        "handler h() for Log\n  op write(line) = unit\n"
    );
    assert_eq!(
        decls(
            "f(x: Int) -> Int\n=\n  when x\n    is 1 ->\n      10\n    else ->\n      let y = x\n      y\n"
        ),
        "f(x: Int) -> Int\n=\n  when x\n    is 1 -> 10\n    else ->\n      let y = x\n      y\n"
    );
}

#[test]
fn a_sum_has_a_leading_bar_only_with_one_variant() {
    assert_eq!(
        decls("kind Size = | Small | Large\n"),
        "kind Size = Small | Large\n"
    );
    assert_eq!(decls("kind One = | Only\n"), "kind One = | Only\n");
    assert_eq!(
        decls("kind Shape =| Circle(r: Int)|Square(s: Int)\n"),
        "kind Shape = Circle(r: Int) | Square(s: Int)\n"
    );
}

#[test]
fn constructor_arguments_follow_the_declaration() {
    // spec/04 §3.4; kinds and variants declared in the same file.
    let kinds = "kind Point\n  x: Int\n  y: Int\n\nkind Shape = Rect(w: Int, h: Int) | Dot\n\n";
    let cases = [
        (
            "f() -> Point = Point(y: 2, x: 1)\n",
            "f() -> Point = Point(x: 1, y: 2)\n",
        ),
        (
            "f(x: Int, y: Int) -> Point = Point(y, x)\n",
            "f(x: Int, y: Int) -> Point = Point(x, y)\n",
        ),
        (
            "f() -> Shape = Rect(h: 1, w: 2)\n",
            "f() -> Shape = Rect(w: 2, h: 1)\n",
        ),
        (
            "f() -> Int = g(p: Point(y: Point(y: 1, x: 2).x, x: 3))\n",
            "f() -> Int = g(p: Point(x: 3, y: Point(x: 2, y: 1).x))\n",
        ),
        // Unchanged: an unlabeled argument, an unknown label, a repeated label.
        (
            "f() -> Point = Point(2, x: 1)\n",
            "f() -> Point = Point(2, x: 1)\n",
        ),
        (
            "f() -> Point = Point(z: 2, x: 1)\n",
            "f() -> Point = Point(z: 2, x: 1)\n",
        ),
        (
            "f() -> Point = Point(y: 2, y: 1)\n",
            "f() -> Point = Point(y: 2, y: 1)\n",
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(
            decls(&format!("{kinds}{source}")),
            format!("{kinds}{expected}"),
            "{source}"
        );
    }
    let failing = "kind E = | Bad(code: Int, reason: Text)\n\nf() -> Int\n  fails E\n=\n  fail Bad(reason: \"x\", code: 1)\n";
    assert_eq!(
        decls(failing),
        "kind E = | Bad(code: Int, reason: Text)\n\nf() -> Int\n  fails E\n=\n  fail Bad(code: 1, reason: \"x\")\n"
    );
    // An imported name is not reordered until M3 resolves it.
    assert_eq!(
        fmt(
            "module t\n\nimport a.b (Point)\n\nkind Point\n  x: Int\n  y: Int\n\nf() -> Point = Point(y: 2, x: 1)\n"
        ),
        "module t\n\nimport a.b (Point)\n\nkind Point\n  x: Int\n  y: Int\n\nf() -> Point = Point(y: 2, x: 1)\n"
    );
}

#[test]
fn imports_are_sorted() {
    assert_eq!(
        fmt(
            "module t\n\nimport std.testing (fake_hash)\nimport events.types (parse_phone, Phone)\nimport events.keys (b, A, a)\n"
        ),
        "module t\n\nimport events.keys (A, a, b)\nimport events.types (Phone, parse_phone)\nimport std.testing (fake_hash)\n"
    );
}

#[test]
fn comments_keep_their_text_and_place() {
    let text = "-- test: a directive block\n-- spec: spec/01 §3\nmodule t\n\n--- Doc for f.\n--- Second line.\npub f(x: Int) -> Int -- trailing\n  requires x > 0\n=\n  -- inside the body\n  let y = x\n\n  -- before the result\n  y\n\n-- A section header.\n\ng() -> Int = 1\n-- The end.\n";
    assert_eq!(fmt(text), text);
}

#[test]
fn comment_spacing_and_indentation_are_canonical() {
    assert_eq!(
        decls("f() -> Int = 1   -- note   \n"),
        "f() -> Int = 1 -- note\n"
    );
    assert_eq!(
        decls("f(x: Int) -> Int\n=\n   -- three spaces\n  let y = x\n -- one space\n  y\n"),
        "f(x: Int) -> Int\n=\n  -- three spaces\n  let y = x\n  -- one space\n  y\n"
    );
}

#[test]
fn a_comment_at_level_zero_leads_the_next_declaration() {
    assert_eq!(
        decls("f() -> Int = 1\n-- about g\ng() -> Int = 2\n"),
        "f() -> Int = 1\n\n-- about g\ng() -> Int = 2\n"
    );
    assert_eq!(
        decls("f(x: Int) -> Int\n=\n  let y = x\n  y\n  -- end of f\ng() -> Int = 2\n"),
        "f(x: Int) -> Int\n=\n  let y = x\n  y\n  -- end of f\n\ng() -> Int = 2\n"
    );
}

#[test]
fn a_comment_inside_a_line_keeps_its_breaks_and_blocks_rewrites() {
    // The formatter never moves a comment to another line. A line with a
    // comment inside keeps its breaks, continuation lines one level deeper.
    assert_eq!(
        decls("f() -> Int = g(1, -- first\n        2,\n    3)\n"),
        "f() -> Int = g(1, -- first\n  2,\n  3)\n"
    );
    assert_eq!(
        decls("f() -> Int = g(\n      -- the only argument\n  1)\n"),
        "f() -> Int = g(\n  -- the only argument\n  1)\n"
    );
    // No merge into the short form across a comment.
    let long = "--- Doc.\nf() -> Int -- signature\n= -- equals\n  -- body\n  1 -- one\n";
    assert_eq!(decls(long), long);
    // No reordering across a comment.
    let kinds = "kind Point\n  x: Int\n  y: Int\n\n";
    let call = "f() -> Point = Point(y: 2, -- y first\n  x: 1)\n";
    assert_eq!(decls(&format!("{kinds}{call}")), format!("{kinds}{call}"));
    // A trailing comment after the body line does not block the short form.
    assert_eq!(
        decls("f() -> Int\n=\n  1 -- one\n"),
        "f() -> Int = 1 -- one\n"
    );
}

#[test]
fn line_breaks_are_line_feeds_and_the_file_ends_with_one() {
    assert_eq!(
        fmt("module t\r\n\r\nf() -> Int = 1\r\n"),
        "module t\n\nf() -> Int = 1\n"
    );
    assert_eq!(
        fmt("module t\n\nf() -> Int = 1"),
        "module t\n\nf() -> Int = 1\n"
    );
}

/// The canonical form of `text` in edition 0.2, formatted once.
fn format_0_2(text: &str) -> String {
    let mut db = SourceDb::new();
    let id = db.add("t.una", text.as_bytes());
    let mut interner = Interner::new();
    let lexed = lex_in(db.file(id), id, &mut interner, KeywordSet::V0_2);
    let parsed = parse(&lexed, id, &interner);
    assert!(
        lexed.errors.is_empty() && parsed.errors.is_empty(),
        "{:?} {:?}\n{text}",
        lexed.errors,
        parsed.errors
    );
    una_fmt::format(db.file(id), &lexed, &parsed.file, &interner)
}

#[test]
fn a_single_call_with_edit_or_take_keeps_its_block_form() {
    // `spec/15 §4`, `spec/17 §3` rule 5: the call is legal only as a statement.
    let kept = [
        "module t\n\nf(xs: List[Int]) -> Int\n=\n  eat(take xs)\n",
        "module t\n\nf(edit n: Int) -> Unit\n=\n  inc(edit n)\n",
        "module t\n\nf(edit t: Tally) -> Unit\n=\n  edit t.add(by: 1)\n",
        "module t\n\nf(k: Int, edit n: Int) -> Unit\n=\n  when k\n    is 1 ->\n      inc(edit n)\n    else -> unit\n",
    ];
    for text in kept {
        assert_eq!(format_0_2(text), text);
    }
}
