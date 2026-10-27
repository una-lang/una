//! The lexer against `spec/01` and ADRs 0014, 0015 and 0017.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use pretty_assertions::assert_eq;
use una_syntax::{Interner, Keyword, SourceDb, TokenKind, keywords, lex};

struct Lexed {
    tokens: Vec<String>,
    errors: Vec<String>,
    edits: Vec<Vec<String>>,
    comments: Vec<(String, bool)>,
}

fn render(kind: &TokenKind, interner: &Interner) -> String {
    match kind {
        TokenKind::Keyword(keyword) => keyword.as_str().to_owned(),
        TokenKind::LowerIdent(name) => format!("id:{}", interner.resolve(*name)),
        TokenKind::UpperIdent(name) => format!("Id:{}", interner.resolve(*name)),
        TokenKind::Reserved(name) => format!("reserved:{}", interner.resolve(*name)),
        TokenKind::CapRef(name) => format!("@{}", interner.resolve(*name)),
        TokenKind::Int(digits) => format!("int:{digits}"),
        TokenKind::TextStart => "\"<".to_owned(),
        TokenKind::TextChunk(text) => format!("chunk:{text:?}"),
        TokenKind::InterpStart => "{".to_owned(),
        TokenKind::InterpEnd => "}".to_owned(),
        TokenKind::TextEnd => "\">".to_owned(),
        TokenKind::Newline => "NL".to_owned(),
        TokenKind::Indent => "IN".to_owned(),
        TokenKind::Dedent => "DE".to_owned(),
        TokenKind::Eof => "EOF".to_owned(),
        punctuation => punctuation.symbol().to_owned(),
    }
}

fn lex_bytes(bytes: &[u8]) -> Lexed {
    let mut sources = SourceDb::new();
    let id = sources.add("t.una", bytes);
    let mut interner = Interner::new();
    let lexed = lex(sources.file(id), id, &mut interner);
    let text = sources.file(id).text().to_owned();
    Lexed {
        tokens: lexed
            .tokens
            .iter()
            .map(|token| render(&token.kind, &interner))
            .collect(),
        errors: lexed
            .errors
            .iter()
            .map(|error| {
                let diagnostic = error.to_diagnostic(&sources);
                format!(
                    "{} {}:{} {}",
                    diagnostic.code,
                    diagnostic.primary.start.line,
                    diagnostic.primary.start.col,
                    diagnostic.message
                )
            })
            .collect(),
        edits: lexed
            .errors
            .iter()
            .map(|error| {
                error
                    .repairs
                    .iter()
                    .map(|repair| match &repair.edit {
                        Some((span, replacement)) => {
                            format!("{:?} -> {replacement:?}", &text[span.start..span.end])
                        }
                        None => repair.description.clone(),
                    })
                    .collect()
            })
            .collect(),
        comments: lexed
            .comments
            .iter()
            .map(|comment| {
                (
                    text[comment.span.start..comment.span.end].to_owned(),
                    comment.doc,
                )
            })
            .collect(),
    }
}

fn lex_text(text: &str) -> Lexed {
    lex_bytes(text.as_bytes())
}

fn tokens(text: &str) -> Vec<String> {
    let lexed = lex_text(text);
    assert_eq!(
        lexed.errors,
        Vec::<String>::new(),
        "unexpected errors in {text:?}"
    );
    lexed.tokens
}

fn errors(text: &str) -> Vec<String> {
    lex_text(text).errors
}

#[test]
fn a_short_function_is_one_logical_line() {
    assert_eq!(
        tokens("pub double(x: Int) -> Int = x * 2\n"),
        [
            "pub",
            "id:double",
            "(",
            "id:x",
            ":",
            "Id:Int",
            ")",
            "->",
            "Id:Int",
            "=",
            "id:x",
            "*",
            "int:2",
            "NL",
            "EOF"
        ]
    );
}

#[test]
fn a_long_function_indents_its_clauses_and_body() {
    let text = "pub f(x: Int) -> Int\n  requires x >= 0\n=\n  let y = x\n  y\n";
    assert_eq!(
        tokens(text),
        [
            "pub", "id:f", "(", "id:x", ":", "Id:Int", ")", "->", "Id:Int", "NL", "IN", "requires",
            "id:x", ">=", "int:0", "NL", "DE", "=", "NL", "IN", "let", "id:y", "=", "id:x", "NL",
            "id:y", "NL", "DE", "EOF"
        ]
    );
}

#[test]
fn blank_and_comment_only_lines_produce_no_layout() {
    let with = "f() -> Int\n\n  -- a comment\n=\n\n    -- deeper comment\n  1\n";
    let without = "f() -> Int\n=\n  1\n";
    assert_eq!(tokens(with), tokens(without));
    assert_eq!(
        lex_text(with).comments,
        [
            ("-- a comment".to_owned(), false),
            ("-- deeper comment".to_owned(), false)
        ]
    );
}

#[test]
fn doc_comments_and_trailing_comments_are_trivia() {
    let lexed = lex_text("--- Doubles.\n---- also doc\nf() -> Int = 1 -- trailing\n");
    assert_eq!(
        lexed.comments,
        [
            ("--- Doubles.".to_owned(), true),
            ("---- also doc".to_owned(), true),
            ("-- trailing".to_owned(), false)
        ]
    );
    assert_eq!(
        lexed.tokens,
        ["id:f", "(", ")", "->", "Id:Int", "=", "int:1", "NL", "EOF"]
    );
}

#[test]
fn the_end_of_file_ends_the_line_and_closes_every_level() {
    assert_eq!(
        tokens("f() -> Int\n=\n  when x\n    else -> 1"),
        [
            "id:f", "(", ")", "->", "Id:Int", "NL", "=", "NL", "IN", "when", "id:x", "NL", "IN",
            "else", "->", "int:1", "NL", "DE", "DE", "EOF"
        ]
    );
    assert_eq!(tokens(""), ["EOF"]);
    assert_eq!(tokens("\n\n-- only a comment\n"), ["EOF"]);
}

#[test]
fn brackets_join_lines() {
    assert_eq!(
        tokens("pub f() -> Int = g(\n  1,\n\n    2) + h[\n3]\n"),
        [
            "pub", "id:f", "(", ")", "->", "Id:Int", "=", "id:g", "(", "int:1", ",", "int:2", ")",
            "+", "id:h", "[", "int:3", "]", "NL", "EOF"
        ]
    );
}

#[test]
fn keywords_contextual_keywords_and_reserved_words() {
    let lexed = lex_text("let text = match @audit\n");
    assert_eq!(
        lexed.tokens,
        [
            "let",
            "id:text",
            "=",
            "reserved:match",
            "@audit",
            "NL",
            "EOF"
        ]
    );
    assert_eq!(
        lexed.errors,
        ["U0103 1:12 `match` is not a keyword in Una; use `when`"]
    );
    assert_eq!(
        errors("pub table() -> Int = 1\n"),
        ["U0103 1:5 `table` is reserved for v0.4 (platform)"]
    );
}

#[test]
fn every_operator_and_punctuation() {
    assert_eq!(
        tokens("== != <= >= < > -> .. ..< . + - * / | , : = ( ) [ ]\n"),
        [
            "==", "!=", "<=", ">=", "<", ">", "->", "..", "..<", ".", "+", "-", "*", "/", "|", ",",
            ":", "=", "(", ")", "[", "]", "NL", "EOF"
        ]
    );
    assert_eq!(
        tokens("a-1 x.len 0..<300\n"),
        [
            "id:a", "-", "int:1", "id:x", ".", "id:len", "int:0", "..<", "int:300", "NL", "EOF"
        ]
    );
}

#[test]
fn integers_and_malformed_integers() {
    assert_eq!(
        tokens("0 42 1_000_000\n"),
        ["int:0", "int:42", "int:1000000", "NL", "EOF"]
    );
    let lexed = lex_text("007 1__0 1_ _5 00\n");
    assert_eq!(
        lexed.tokens,
        ["int:7", "int:10", "int:1", "int:5", "int:0", "NL", "EOF"]
    );
    assert_eq!(
        lexed.errors,
        [
            "U0007 1:1 malformed integer literal `007`",
            "U0007 1:5 malformed integer literal `1__0`",
            "U0007 1:10 malformed integer literal `1_`",
            "U0007 1:13 malformed integer literal `_5`",
            "U0007 1:16 malformed integer literal `00`",
        ]
    );
    assert_eq!(
        lexed.edits,
        [
            vec!["\"007\" -> \"7\"".to_owned()],
            vec!["\"1__0\" -> \"1_0\"".to_owned()],
            vec!["\"1_\" -> \"1\"".to_owned()],
            vec!["\"_5\" -> \"5\"".to_owned()],
            vec!["\"00\" -> \"0\"".to_owned()],
        ]
    );
    // Letters after digits start a new token.
    assert_eq!(tokens("12abc\n"), ["int:12", "id:abc", "NL", "EOF"]);
}

#[test]
fn text_literals_decode_escapes_and_split_interpolations() {
    assert_eq!(
        tokens("\"say \\\"hi\\\" \\\\ \\n\\t \\{x\\} \\u{41}\\u{1F600}\"\n"),
        [
            "\"<",
            "chunk:\"say \\\"hi\\\" \\\\ \\n\\t {x} A\u{1f600}\"",
            "\">",
            "NL",
            "EOF"
        ]
    );
    assert_eq!(
        tokens("\"seats {e.remaining + 1}: {label}\"\n"),
        [
            "\"<",
            "chunk:\"seats \"",
            "{",
            "id:e",
            ".",
            "id:remaining",
            "+",
            "int:1",
            "}",
            "chunk:\": \"",
            "{",
            "id:label",
            "}",
            "\">",
            "NL",
            "EOF"
        ]
    );
    assert_eq!(tokens("\"\"\n"), ["\"<", "\">", "NL", "EOF"]);
}

#[test]
fn unterminated_literals_are_one_error_at_the_opening_quote() {
    assert_eq!(
        errors("x = \"hello\n"),
        ["U0004 1:5 unterminated text literal; close it with `\"` on the same line"]
    );
    assert_eq!(
        errors("x = \"hello"),
        ["U0004 1:5 unterminated text literal; close it with `\"` on the same line"]
    );
    assert_eq!(
        errors("x = \"a {b\n"),
        ["U0004 1:5 unterminated text literal; close it with `\"` on the same line"]
    );
    // ADR 0017, decision 4: a quote without a partner closes the literal whose `}` is missing.
    let lexed = lex_text("x = \"hello {name\" + y\n");
    assert_eq!(
        lexed.errors,
        ["U0004 1:5 unterminated text literal; its interpolation is missing `}`"]
    );
    assert_eq!(
        lexed.tokens,
        [
            "id:x",
            "=",
            "\"<",
            "chunk:\"hello \"",
            "{",
            "id:name",
            "}",
            "\">",
            "+",
            "id:y",
            "NL",
            "EOF"
        ]
    );
}

#[test]
fn invalid_escapes_are_reported_at_the_backslash() {
    assert_eq!(
        errors(
            "\"C:\\data\" \"\\u0041\" \"\\u{}\" \"\\u{1234567}\" \"\\u{D800}\" \"\\u{110000}\" \"\\u{12G4}\"\n"
        ),
        [
            "U0005 1:4 invalid escape `\\d`; write `\\\\` for a backslash",
            "U0005 1:12 `\\u` escape needs braces, as in `\\u{1F600}`",
            "U0005 1:21 empty `\\u{}` escape",
            "U0005 1:28 `\\u{...}` escape has more than six digits",
            "U0005 1:42 `\\u{D800}` is not a Unicode scalar value",
            "U0005 1:53 `\\u{110000}` is not a Unicode scalar value",
            "U0005 1:66 `\\u{...}` escape needs one to six hexadecimal digits and a closing `}`",
        ]
    );
}

#[test]
fn a_text_literal_inside_an_interpolation_is_rejected() {
    let lexed = lex_text("x = \"a {f(\"b\")} c\"\n");
    assert_eq!(
        lexed.errors,
        [
            "U0006 1:11 text literal inside interpolation; bind it with `let` and interpolate the name"
        ]
    );
    assert_eq!(
        lexed.tokens,
        [
            "id:x",
            "=",
            "\"<",
            "chunk:\"a \"",
            "{",
            "id:f",
            "(",
            "\"<",
            "chunk:\"b\"",
            "\">",
            ")",
            "}",
            "chunk:\" c\"",
            "\">",
            "NL",
            "EOF"
        ]
    );
}

#[test]
fn an_unescaped_closing_brace_in_text_is_malformed() {
    let lexed = lex_text("x = \"a } b\"\n");
    assert_eq!(
        lexed.errors,
        ["U0007 1:8 unescaped `}` in text; write `\\}`"]
    );
    assert_eq!(lexed.edits, [vec!["\"}\" -> \"\\\\}\"".to_owned()]]);
}

#[test]
fn characters_that_cannot_start_a_token_have_hints() {
    // ADR 0017, decision 3: one diagnostic per character, with a hint where one helps.
    assert_eq!(
        errors("a; !b && c 'd' _e $ # ? ~ ^ ` \\ { } @X é\n"),
        [
            "U0001 1:2 invalid character `;`; statements end at line breaks",
            "U0001 1:4 invalid character `!`; use `not`",
            "U0001 1:7 invalid character `&`; use `and`",
            "U0001 1:8 invalid character `&`; use `and`",
            "U0001 1:12 invalid character `'`; text literals use double quotes",
            "U0001 1:14 invalid character `'`; text literals use double quotes",
            "U0001 1:16 invalid character `_`; names start with a letter",
            "U0001 1:19 invalid character `$`",
            "U0001 1:21 invalid character `#`",
            "U0001 1:23 invalid character `?`",
            "U0001 1:25 invalid character `~`",
            "U0001 1:27 invalid character `^`",
            "U0001 1:29 invalid character `` ` ``",
            "U0001 1:31 invalid character `\\`",
            "U0001 1:33 invalid character `{`",
            "U0001 1:35 invalid character `}`",
            "U0001 1:37 invalid character `@`",
            "U0001 1:40 non-ASCII character `é` outside a text literal or comment",
        ]
    );
}

#[test]
fn forbidden_characters_are_rejected_everywhere() {
    // ADR 0017, decision 2: control and bidirectional characters, anywhere.
    assert_eq!(
        errors("a\u{7}b -- c\u{202e}d\nx = \"e\u{2066}f\u{7f}\"\n"),
        [
            "U0001 1:2 control character U+0007 is not allowed; text literals can write it as `\\u{7}`",
            "U0001 1:9 bidirectional control character U+202E is not allowed; text literals can write it as `\\u{202E}`",
            "U0001 2:7 bidirectional control character U+2066 is not allowed; text literals can write it as `\\u{2066}`",
            "U0001 2:9 control character U+007F is not allowed; text literals can write it as `\\u{7F}`",
        ]
    );
    assert_eq!(
        errors("x = 1\ry\n"),
        ["U0001 1:6 carriage return not followed by a line feed"]
    );
    assert_eq!(tokens("x = \"tab\there\" -- a\ttab\n").len(), 7);
}

#[test]
fn invalid_utf8_is_one_error_wherever_it_is() {
    let lexed = lex_bytes(b"a\xff b -- c\xe2\x82\nx = \"\xfe\"\n");
    assert_eq!(
        lexed.errors,
        [
            "U0001 1:2 invalid UTF-8 byte sequence",
            "U0001 1:10 invalid UTF-8 byte sequence",
            "U0001 2:6 invalid UTF-8 byte sequence",
        ]
    );
    assert_eq!(
        lexed.tokens,
        ["id:a", "id:b", "NL", "id:x", "=", "\"<", "\">", "NL", "EOF"]
    );
}

#[test]
fn a_byte_order_mark_is_reported_at_1_1_and_skipped() {
    let lexed = lex_bytes(b"\xef\xbb\xbfmodule m\n");
    assert_eq!(
        lexed.errors,
        ["U0001 1:1 byte order mark at the start of the file"]
    );
    assert_eq!(lexed.tokens, ["module", "id:m", "NL", "EOF"]);
}

#[test]
fn a_tab_in_indentation_reports_only_t0002_with_an_edit() {
    let lexed = lex_text("f() -> Int\n=\n\t1\n  \tx\n");
    assert_eq!(
        lexed.errors,
        [
            "U0002 3:1 tab in indentation; indentation uses spaces, two per level",
            "U0002 4:3 tab in indentation; indentation uses spaces, two per level",
        ]
    );
    assert_eq!(
        lexed.edits,
        [
            vec!["\"\\t\" -> \"  \"".to_owned()],
            vec!["\"  \\t\" -> \"    \"".to_owned()],
        ]
    );
    assert_eq!(
        lexed.tokens,
        // A tab counts two columns when choosing the recovery level, so
        // `  \t` (4) opens a deeper level after a line at 2.
        [
            "id:f", "(", ")", "->", "Id:Int", "NL", "=", "NL", "IN", "int:1", "NL", "IN", "id:x",
            "NL", "DE", "DE", "EOF"
        ]
    );
}

#[test]
fn a_tab_leading_a_comment_or_a_joined_line_is_t0002_without_layout() {
    // Rule 1 of `spec/01 §2` applies to every line.
    let lexed = lex_text("x = f(\n\t1)\n\t-- note\ny = 2\n");
    assert_eq!(
        lexed.errors,
        [
            "U0002 2:1 tab in indentation; indentation uses spaces, two per level",
            "U0002 3:1 tab in indentation; indentation uses spaces, two per level",
        ]
    );
    assert_eq!(
        lexed.tokens,
        [
            "id:x", "=", "id:f", "(", "int:1", ")", "NL", "id:y", "=", "int:2", "NL", "EOF"
        ]
    );
}

#[test]
fn inconsistent_indentation_snaps_to_the_nearest_level() {
    let lexed = lex_text("f() -> Int\n=\n   1\n");
    assert_eq!(
        lexed.errors,
        ["U0003 3:1 inconsistent indentation of 3 spaces; expected 0 or 2"]
    );
    assert_eq!(
        lexed.edits,
        [vec![
            "\"   \" -> \"  \"".to_owned(),
            "\"   \" -> \"\"".to_owned()
        ]]
    );
    assert_eq!(
        lexed.tokens,
        [
            "id:f", "(", ")", "->", "Id:Int", "NL", "=", "NL", "IN", "int:1", "NL", "DE", "EOF"
        ]
    );
    // A dedent to a column no block uses: ties go to the deeper level.
    let lexed = lex_text("f() -> Int\n=\n  if x\n    1\n else\n    2\n");
    assert_eq!(
        lexed.errors,
        ["U0003 5:1 inconsistent indentation of 1 space; expected 0, 2, 4 or 6"]
    );
    assert_eq!(
        lexed.tokens,
        [
            "id:f", "(", ")", "->", "Id:Int", "NL", "=", "NL", "IN", "if", "id:x", "NL", "IN",
            "int:1", "NL", "DE", "else", "NL", "IN", "int:2", "NL", "DE", "DE", "EOF"
        ]
    );
}

#[test]
fn brackets_left_open_close_at_the_next_declaration() {
    let lexed = lex_text("pub f() -> Int = (1 + [2\n\npub g() -> Int = 4\n");
    assert_eq!(
        lexed.errors,
        [
            "U0101 1:23 unclosed `[` before the next declaration",
            "U0101 1:18 unclosed `(` before the next declaration",
        ]
    );
    assert_eq!(
        lexed.tokens,
        [
            "pub", "id:f", "(", ")", "->", "Id:Int", "=", "(", "int:1", "+", "[", "int:2", "]",
            ")", "NL", "pub", "id:g", "(", ")", "->", "Id:Int", "=", "int:4", "NL", "EOF"
        ]
    );
    assert_eq!(
        errors("x = f(1\n"),
        ["U0101 1:6 unclosed `(` before the end of the file"]
    );
    // A generic function signature is a declaration start too.
    assert_eq!(
        errors("x = f(1\nfirst[T](xs: List[T]) -> T = xs\n"),
        ["U0101 1:6 unclosed `(` before the next declaration"]
    );
}

#[test]
fn closing_brackets_must_match() {
    assert_eq!(errors("x = 1)\n"), ["U0101 1:6 unmatched `)`"]);
    assert_eq!(
        errors("x = f(1]\n"),
        [
            "U0101 1:8 `]` does not close `(`",
            "U0101 1:6 unclosed `(` before the end of the file"
        ]
    );
    assert_eq!(
        errors("x = \"{f(1}\"\n"),
        ["U0101 1:8 unclosed `(` before the end of the interpolation"]
    );
}

#[test]
fn capability_references() {
    assert_eq!(
        tokens("@audit.record(x)\n"),
        ["@audit", ".", "id:record", "(", "id:x", ")", "NL", "EOF"]
    );
    // A keyword cannot name a capability.
    assert_eq!(
        errors("@if\n"),
        ["U0101 1:2 keyword `if` cannot name a capability"]
    );
    assert_eq!(
        errors("@table\n"),
        ["U0103 1:2 `table` is reserved for v0.4 (platform)"]
    );
}

/// The code block that follows `marker` in `spec/01 §5`, split into words.
fn spec_block(spec: &str, marker: &str) -> Vec<String> {
    let after = spec.split(marker).nth(1).unwrap();
    let block = after
        .split("```text")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    block.split_whitespace().map(str::to_owned).collect()
}

#[test]
fn keyword_tables_match_spec_01_section_5() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/spec/01-lexical-structure.md");
    let spec = std::fs::read_to_string(path).unwrap();
    let reserved: Vec<&str> = Keyword::ALL
        .iter()
        .map(|keyword| keyword.as_str())
        .collect();
    assert_eq!(reserved, spec_block(&spec, "**Reserved keywords**"));
    assert_eq!(
        keywords::CONTEXTUAL.to_vec(),
        spec_block(&spec, "**Contextual keywords**")
    );
    let mut later: Vec<&str> = keywords::RESERVED_FOR_LATER
        .iter()
        .map(|(word, _)| *word)
        .collect();
    later.sort_unstable();
    assert_eq!(
        later,
        spec_block(&spec, "**Reserved, not available in v0.1.**")
    );
    // Every row of the message table, keyword by keyword.
    let table = spec.split("| Keywords | Message |").nth(1).unwrap();
    let mut from_spec = BTreeMap::new();
    for row in table
        .lines()
        .skip(2)
        .take_while(|line| line.starts_with('|'))
    {
        let cells: Vec<&str> = row.trim_matches('|').split('|').map(str::trim).collect();
        for word in cells[0].split(", ") {
            from_spec.insert(word.trim_matches('`').to_owned(), cells[1].to_owned());
        }
    }
    let from_code: BTreeMap<String, String> = keywords::RESERVED_FOR_LATER
        .iter()
        .map(|(word, message)| ((*word).to_owned(), (*message).to_owned()))
        .collect();
    assert_eq!(from_code, from_spec);
}

#[test]
fn the_edition_0_2_table_matches_spec_17_19_and_20_section_9() {
    // The rows of edition 0.2: those of the memory model (`spec/17 §9`), of structured
    // concurrency (`spec/19 §9`) and of constraints on generics (`spec/20 §9`).
    let mut from_spec = BTreeMap::new();
    for chapter in [
        "17-memory-model.md",
        "19-structured-concurrency.md",
        "20-constraints-on-generics.md",
    ] {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/spec")
            .join(chapter);
        let spec = std::fs::read_to_string(path).unwrap();
        let section = spec.split("## 9. Reserved words").nth(1).unwrap();
        let table = section.split("| Keywords | Message |").nth(1).unwrap();
        for row in table
            .lines()
            .skip(2)
            .take_while(|line| line.starts_with('|'))
        {
            let cells: Vec<&str> = row.trim_matches('|').split('|').map(str::trim).collect();
            for word in cells[0].split(", ") {
                from_spec.insert(word.trim_matches('`').to_owned(), cells[1].to_owned());
            }
        }
    }
    let from_code: BTreeMap<String, String> = keywords::RESERVED_IN_0_2
        .iter()
        .map(|(word, message)| ((*word).to_owned(), (*message).to_owned()))
        .collect();
    assert_eq!(from_code, from_spec);
}

#[test]
fn with_after_a_dot_names_the_map_method_in_edition_0_2() {
    // `spec/21 §2`, ADR 0069 M2: `m.with(key: k, value: v)`; elsewhere `with` stays a keyword.
    let kinds = |keywords: una_syntax::KeywordSet| {
        let mut db = SourceDb::new();
        let mut interner = Interner::new();
        let id = db.add("t.una", b"m.with with");
        let lexed = una_syntax::lex_in(db.file(id), id, &mut interner, keywords);
        lexed
            .tokens
            .iter()
            .map(|token| render(&token.kind, &interner))
            .filter(|token| token != "eof")
            .collect::<Vec<_>>()
    };
    assert_eq!(
        kinds(una_syntax::KeywordSet::V0_2)[..4],
        ["id:m", ".", "id:with", "with"]
    );
    assert_eq!(
        kinds(una_syntax::KeywordSet::V0_1)[..4],
        ["id:m", ".", "with", "with"]
    );
}

#[test]
fn test_is_the_keyword_of_the_test_declaration_from_edition_0_3() {
    // ADR 0105 rules 2 and 3: `test` is a keyword from edition 0.3, where `example` at column 1
    // is U0103 with the exact rewrite of the keyword; both stay as they were in edition 0.2.
    let lexed = |keywords: una_syntax::KeywordSet| {
        let mut db = SourceDb::new();
        let mut interner = Interner::new();
        let id = db.add("t.una", b"test\nexample\n  example\n");
        let lexed = una_syntax::lex_in(db.file(id), id, &mut interner, keywords);
        let tokens: Vec<String> = lexed
            .tokens
            .iter()
            .map(|token| render(&token.kind, &interner))
            .filter(|token| token != "eof")
            .collect();
        let errors: Vec<(String, Option<String>)> = lexed
            .errors
            .iter()
            .map(|error| {
                let diagnostic = error.to_diagnostic(&db);
                (
                    format!(
                        "{} {}:{} {}",
                        diagnostic.code,
                        diagnostic.primary.start.line,
                        diagnostic.primary.start.col,
                        diagnostic.message
                    ),
                    diagnostic.repairs[0]
                        .edit
                        .as_ref()
                        .map(|edit| edit.replacement.clone()),
                )
            })
            .collect();
        (tokens, errors)
    };
    let (tokens, errors) = lexed(una_syntax::KeywordSet::V0_2);
    assert_eq!(
        tokens[..3],
        ["id:test".to_owned(), "NL".to_owned(), "example".to_owned()]
    );
    assert_eq!(errors, []);
    let (tokens, errors) = lexed(una_syntax::KeywordSet::V0_3);
    assert_eq!(
        tokens[..3],
        ["test".to_owned(), "NL".to_owned(), "example".to_owned()]
    );
    // Only the `example` at column 1 is the declaration keyword of the wrong edition.
    assert_eq!(
        errors,
        [(
            "U0103 2:1 `example` is written `test` in edition 0.3".to_owned(),
            Some("test".to_owned())
        )]
    );
}
