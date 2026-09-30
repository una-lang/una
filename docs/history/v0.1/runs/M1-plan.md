# M1 plan: lexer and parser

Status: approved by the owner on 2026-09-29, with the amendments of `OVERNIGHT.md` Part B. The owner's decisions are recorded in ADR 0017 (Accepted). Decisions taken during implementation are logged in `docs/reviews/open-decisions.md` and tagged `DECISION D-NNN` in the code.

Read list: `spec/01`, `spec/02`, `compiler/architecture.md §4, §5`, ADRs 0011, 0014, 0015, 0017.

## 1. Tasks and commits

Each commit writes its tests first and passes fmt, clippy and tests.

1. **Source database** (`feat(syntax)`, task 1; `spec/01 §1`, `spec/13 §1`, ADR 0015 D2): `FileId`; `Span` (byte offsets into the decoded text); `SourceDb` and `SourceFile`; decoding that turns each maximal ill-formed subsequence into U+FFFD and records its position, normalizes `\r\n` to `\n` and keeps a byte order mark and lone `\r` for the lexer; a line index (offset to line and column in scalar values, end of file one past the last character); conversion to `una_diag::Location`; a deterministic interner (`Symbol`, `indexmap`).
2. **`una check` loading** (`feat(cli)`, task 4 part 1; `spec/13 §5`, ADR 0015 D1, D3, D4): `una check [path] [--tier 0|1]`; interim loading of a file or `src/**/*.una`; bare file names for single files; human diagnostics on stderr with excerpts, JSON on stdout; `una_cli::check(path, tier)`; the runner passes tier 0 for `tier0-ok`; M0 CLI tests that ran `una check` in the repository root use `examples/hello`.
3. **Lexer** (`feat(syntax)`, task 2; `spec/01`, ADRs 0014, 0015, 0017): tokens, the three keyword classes (U0103 table checked against `spec/01 §5` by a test), layout with `NEWLINE`, `INDENT`, `DEDENT`, bracket joining, comments and doc comments as trivia; U0001 to U0007, U0103 and bracket balance (U0101). ADR 0017 and its spec and registry edits land here, with the conformance cases of these codes and `.gitattributes`.
4. **Parser** (`feat(syntax)`, task 3 and error recovery; `spec/02`, `compiler/architecture.md §5`): recursive descent for all of `spec/02`, typed AST with spans, doc comments attached to declarations, U0101 with expected tokens, U0102 for function and operation clauses, recovery at the next declaration. Conformance cases for U0101 and U0102, a test that every example parses, insta snapshots of a compact AST dump of the examples.
5. **Explain texts** (`docs(diag)`): the ten texts of section 3.
6. **Robustness** (`test(syntax)`): the truncation test of section 5, and a test that conformance cases without syntax expectations parse without syntax diagnostics.
7. **Self-review** (`docs(plan)`): the checklist of section 7, progress and morning report.

## 2. Conformance cases (`tests/conformance/syntax/`)

Single-error cases have three directive lines, `module` on line 4, a blank line 5 and code from line 6 unless stated otherwise. Positions are verified by the runner, which requires them to exist in the file. Passing cases use `-- expect: tier0-ok`.

| Code | Case | Offending source | Expected |
| --- | --- | --- | --- |
| U0001 | `byte_order_mark` | byte order mark before line 1 | `1:1` |
| U0001 | `invalid_utf8` | `-- caf<0xE9> au lait` | `6:7` |
| U0001 | `lone_carriage_return` | `-- first<CR>second` | `6:9` |
| U0001 | `non_ascii_identifier` | `pub café() -> Text = "coffee"` | `6:8` |
| U0001 | `invalid_character` | `pub one() -> Int = 1;` | `6:21` |
| U0001 | `bidi_control_in_comment` | `-- <U+202E>` inside a comment | at the control |
| U0001 | `control_character_in_text` | a raw U+0007 inside a text literal | at the control |
| U0001 ok | `non_ascii_in_text_and_comments` | accents, CJK and a real U+FFFD in a comment and a literal | tier0-ok |
| U0001 ok | `bidi_escape_in_text` | `"\u{202E}"` written as an escape | tier0-ok |
| U0002 | `tab_in_indentation` | body line starting with a tab | `9:1` |
| U0002 | `tab_after_spaces_in_indentation` | two spaces, then a tab | `9:3` |
| U0002 ok | `tab_in_text_and_comment` | tabs inside a literal and a comment | tier0-ok |
| U0003 | `indentation_not_a_multiple_of_two` | body indented by 3 spaces | `9:1` |
| U0003 | `indentation_two_levels_deeper` | body indented by 4 spaces | `9:1` |
| U0003 | `dedent_to_unknown_level` | `else` dedented to 1 space | `10:1` |
| U0003 ok | `nested_blocks` | `if`/`else if`/`else` blocks, `let` bound to a `when` | tier0-ok |
| U0004 | `unterminated_text` | `pub greeting() -> Text = "hello` | `6:26` |
| U0004 | `unterminated_text_at_end_of_file` | the same without a final newline | `6:26` |
| U0004 | `unterminated_interpolation` | `pub greeting(name: Text) -> Text = "hello {name"` | `6:36` |
| U0004/5 ok | `text_literals` | every valid escape | tier0-ok |
| U0005 | `unknown_escape` | `pub folder() -> Text = "C:\data"` | `6:27` |
| U0005 | `invalid_unicode_escapes` | `\u{110000}`, `\u{D800}`, `\u{}`, `\u0041` | four errors, each at its backslash |
| U0006 | `text_inside_interpolation` | `pub greeting() -> Text = "hello {trim(" world ")}"` | `6:39` |
| U0006 ok | `interpolation` | field access, arithmetic and calls inside `{}`, escaped braces | tier0-ok |
| U0007 | `integer_leading_zero` | `007` | at the literal |
| U0007 | `integer_doubled_underscore` | `1__0` | at the literal |
| U0007 | `integer_trailing_underscore` | `1_` | at the literal |
| U0007 | `unescaped_brace_in_text` | `"a } b"` | at the `}` |
| U0101 | `missing_arrow` | `pub one() Int = 1` | `6:11` |
| U0101 | `chained_comparison` | `pub between(x: Int) -> Bool = 0 < x < 10` | `6:37` |
| U0101 | `unclosed_parenthesis` | `pub three() -> Int = (1 + 2`, then another declaration | `6:22` |
| U0101 | `unmatched_closing_bracket` | `pub one() -> Int = 1)` | `6:21` |
| U0101 | `keyword_as_name` | `pub total(if: Int) -> Int = 1` | `6:11` |
| U0101 | `recovery_per_declaration` | two broken declarations around a valid one | one U0101 per broken declaration |
| U0101 ok | `grammar_tour` | sums, records, refinements, a reflect predicate, short and long functions, `when`, `let` | tier0-ok |
| U0102 | `clause_out_of_order` | `ensures` before `requires` | at `requires` |
| U0102 | `repeated_fails` | a second `fails` | at the second `fails` |
| U0102 | `repeated_derived_from` | a second `derived from` | at the second clause |
| U0102 | `operation_clause_out_of_order` | an operation with `fails` before `ensures` | at `ensures` |
| U0102 | `repeated_operation_accepts` | an operation with two `accepts` | at the second `accepts` |
| U0102 ok | `canonical_clause_order` | every function clause, and an operation with every clause, in order | tier0-ok |
| U0103 | `reserved_keyword_return` | `pub double(x: Int) -> Int = return x * 2` | `6:29` |
| U0103 | `reserved_keyword_as_name` | `pub table() -> Int = 1` | `6:5` |
| U0103 ok | `contextual_keywords_as_names` | `text`, `source`, `to` as parameters; names such as `matches` | tier0-ok |

Every failing case is otherwise a valid program, so later milestones add no diagnostics to it.

## 3. Explain texts

The examples use `module demo`, because `example` is a keyword. Each failing example yields only its own code under the recovery rules of section 6, and each fixed example is valid at every tier.

````markdown
# U0001: Invalid character, byte order mark or lone carriage return

Every source file is UTF-8 text without a byte order mark, and its lines end with `\n` or `\r\n`. Identifiers, keywords and punctuation are ASCII; any other character, such as an accented letter, may appear only inside text literals and comments. Control characters other than tab and line feed, and Unicode bidirectional controls, are rejected everywhere, including literals and comments, so that reviewed code always reads as it runs; write them in text with `\u{...}` escapes. U0001 reports a character that cannot appear where it is, a byte sequence that is not valid UTF-8, a byte order mark at the start of the file, and a carriage return not followed by a line feed. Lexing continues after the error, so later problems in the file are still reported.

## Failing example

```una
module demo

pub café() -> Text = "coffee"
```

## Fixed example

```una
module demo

pub cafe() -> Text = "coffee"
```
````

````markdown
# U0002: Tab in indentation

Una uses significant indentation made of spaces only, two per level (`spec/01 §2`). A tab in the leading whitespace of a line is rejected, because its width depends on the editor and would make the block structure ambiguous. Tabs inside text literals and comments are not indentation and are allowed. The diagnostic carries a mechanical edit that replaces the indentation with spaces.

## Failing example

The body line starts with a tab.

```una
module demo

pub double(x: Int) -> Int
  requires x >= 0
=
	x * 2
```

## Fixed example

```una
module demo

pub double(x: Int) -> Int
  requires x >= 0
=
  x * 2
```
````

````markdown
# U0003: Inconsistent indentation

Each indentation level is exactly two spaces deeper than the enclosing one, and a line that closes blocks must return to the indentation of an enclosing level (`spec/01 §2`). U0003 reports a line indented by an odd number of spaces relative to its block, a line more than one level deeper than the line before it, and a line dedented to a column that no enclosing block uses. The diagnostic proposes mechanical edits that re-indent the line to the nearest valid levels.

## Failing example

```una
module demo

pub double(x: Int) -> Int
  requires x >= 0
=
   x * 2
```

## Fixed example

```una
module demo

pub double(x: Int) -> Int
  requires x >= 0
=
  x * 2
```
````

````markdown
# U0004: Unterminated text literal

A text literal starts and ends with `"` on the same line; it cannot span lines (`spec/01 §6`). U0004 reports a literal whose closing quote is missing before the end of the line or of the file, including a literal whose interpolation is missing its `}`. Write a quote inside text as `\"` and a line break as `\n`.

## Failing example

```una
module demo

pub greeting() -> Text = "hello
```

## Fixed example

```una
module demo

pub greeting() -> Text = "hello"
```
````

````markdown
# U0005: Invalid escape sequence

Inside a text literal, a backslash starts an escape sequence. The valid escapes are `\"`, `\\`, `\n`, `\t`, `\{`, `\}` and `\u{H}`, where `H` is one to six hexadecimal digits naming a Unicode scalar value (`spec/01 §6`). U0005 reports any other character after a backslash, a `\u` escape without braces or digits, and a code point that is a surrogate or above `10FFFF`. Write a backslash itself as `\\`.

## Failing example

```una
module demo

pub folder() -> Text = "C:\data"
```

## Fixed example

```una
module demo

pub folder() -> Text = "C:\\data"
```
````

````markdown
# U0006: Text literal inside interpolation

Interpolation embeds an expression in a text literal, as in `"seats {e.remaining}"` (`spec/01 §6`). The embedded expression must not contain another text literal, which keeps every literal on one level and easy to read and edit. Bind the inner text with `let` and interpolate the name instead.

## Failing example

```una
module demo

pub greeting() -> Text = "hello {trim(" world ")}"
```

## Fixed example

```una
module demo

pub greeting() -> Text
=
  let name = trim(" world ")
  "hello {name}"
```
````

````markdown
# U0007: Malformed literal

An integer literal is written in decimal digits, with `_` allowed only between two digits to group them, and without leading zeros except for `0` itself (`spec/01 §6`). Inside a text literal, `{` starts an interpolation and `}` ends it, so a `}` outside an interpolation must be escaped as `\}`. U0007 reports an integer literal with a leading zero, a doubled underscore, or an underscore at its start or end, and an unescaped `}` in text. When the intended spelling is unambiguous, the diagnostic carries a mechanical edit.

## Failing example

```una
module demo

pub timeout() -> Int = 030
```

## Fixed example

```una
module demo

pub timeout() -> Int = 30
```
````

````markdown
# U0101: Unexpected token

The parser found a token that the grammar of `spec/02` does not allow at that point; the message names the token found and what was expected. U0101 also reports brackets left unbalanced within a declaration, chained comparisons such as `a < b < c` (`spec/01 §7`), and keywords such as `if` used as names. After the error, parsing resumes at the next declaration, so the rest of the file is still checked.

## Failing example

```una
module demo

pub between(x: Int) -> Bool = 0 < x < 10
```

## Fixed example

```una
module demo

pub between(x: Int) -> Bool = 0 < x and x < 10
```
````

````markdown
# U0102: Clause out of order or repeated

The clauses of a function header follow one canonical order: every `requires`, then every `ensures`, then every `uses`, then at most one `fails`, then at most one `derived from` (`spec/02 §3`). Capability operations follow the same idea: every `requires`, then every `ensures`, then at most one `fails`, then at most one `accepts` (`spec/02 §4`). A fixed order lets people and tools find each part of a contract at a glance. U0102 reports a clause placed after a clause that must follow it, and a repeated `fails`, `derived from` or `accepts` clause: a function or operation fails with a single error kind (ADR 0006), and one `derived from` lists all its requirements.

## Failing example

```una
module demo

pub double(x: Int) -> Int
  ensures result == x * 2
  requires x >= 0
=
  x * 2
```

## Fixed example

```una
module demo

pub double(x: Int) -> Int
  requires x >= 0
  ensures result == x * 2
=
  x * 2
```
````

````markdown
# U0103: Reserved keyword not available in v0.1

Some words are reserved for features planned in later versions of Una, and others name constructs from other languages that Una expresses differently. Using one as a keyword or as a name is rejected, and the message says which version plans the feature or which Una construct to use instead (`spec/01 §5`). For example, Una has no `return`: the last expression of a body is its result.

## Failing example

```una
module demo

pub double(x: Int) -> Int = return x * 2
```

## Fixed example

```una
module demo

pub double(x: Int) -> Int = x * 2
```
````

## 4. Other tests

- `una-syntax` unit tests: decoding and columns; every token kind; layout around nested blocks, blank and comment-only lines and end of file; bracket joining; each lexer error with its position, message, hint, repair and recovery; every valid and invalid escape; interpolation; integer literal forms; the keyword tables against `spec/01 §5`; every grammar production; precedence and associativity; U0102 order for functions and operations; recovery; doc comment attachment and orphan doc comments; spans.
- `crates/una-syntax/tests/examples.rs`: every `.una` file under `examples/` parses without diagnostics.
- insta snapshots of a compact AST dump of the two example packages, reviewed line by line.
- CLI: a syntax error exits 1 with human output on stderr and nothing on stdout; `--json` prints on stdout with an empty stderr; `--tier 0` and `--tier 1` are accepted and `--tier 2` is a usage error; a missing path and a directory without `src/` exit 2.

## 5. Acceptance criteria

| Criterion | Evidence |
| --- | --- |
| Conformance cases for U0001 to U0007 and U0101 to U0103 | Runner summary: every syntax case passes, no case pending on M1, 3 pending (M5, M6, M8) |
| Every example parses | `tests/examples.rs`; `una check examples/events` and `examples/hello` exit 0 without output |
| Robustness (wording of ADR 0017, D8) | For every example and every line boundary, parsing the prefix never panics and every reported position lies inside the prefix; a prefix that ends right after the first line of a multi-line declaration, or right after the `=` line of a long-form function, yields at least one diagnostic |
| Explain texts | `una explain` for each code; each failing example yields its code at tier 0 and each fixed example yields nothing |
| Every commit | fmt, clippy and tests locally; green CI after each push |

## 6. Decisions

Owner decisions (ADR 0017, Accepted):
1. U0007 "Malformed literal": integer literals with a leading zero other than `0`, a doubled underscore, or a leading or trailing underscore ("malformed integer literal `007`", with a mechanical edit when unambiguous: `007` to `7`, `1_` to `1`, `1__0` to `1_0`), and an unescaped `}` in text ("unescaped `}` in text; write `\}`", with its edit).
2. Control characters U+0000 to U+001F other than tab and line feed, U+007F, and the bidirectional controls U+202A to U+202E and U+2066 to U+2069 are U0001 everywhere, including literals and comments; text writes them as `\u{...}`.
3. A character that cannot start a token is U0001, one per character, with a hint for `;` (statements end at line breaks), `!` not followed by `=` (use `not`), `&` (use `and`), `'` (text literals use double quotes) and `_` starting a name (names start with a letter).
4. A missing interpolation `}` is one U0004 at the outer opening quote.
5. U0102 also orders capability operation clauses: `requires*`, `ensures*`, `fails?`, `accepts?`.
6. A doc comment not followed by a declaration is a plain comment.
7. The robustness criterion reads as in section 5.

Accepted design choices:
- Positions: U0001 at the character (for invalid UTF-8, at its U+FFFD column); U0002 at the first tab of the indentation; U0003 at column 1, spanning the indentation; U0004 at the opening quote; U0005 at the backslash, spanning the escape; U0006 at the nested literal's opening quote; U0007 at the literal or the `}`; U0101 at the unexpected token, or one past the last character at end of file; U0102 at the clause keyword; U0103 at the keyword.
- Repairs: U0002 replaces the indentation with spaces; U0003 re-indents to the nearest valid levels, ranked.
- Recovery: a line whose indentation has a tab reports only U0002 and takes the nearest valid level; U0003 snaps to the nearest valid level, ties to the deeper one; brackets still open at the next top-level declaration get U0101 at each opener and synthetic closers; an unmatched closer is dropped; a U0103 word is read as a name, except at the start of an expression when the next token could also start one (`return x * 2`), where it is skipped; the parser reports at most one U0101 per declaration and resumes at the next one; U0102 does not stop parsing.
- Explain file format: title, paragraph, `## Failing example` and `## Fixed example` with one `una` block each.
- `--tier` accepts 0 and 1, defaulting to 1.

Housekeeping: `una-cli` depends on `una-syntax`; `indexmap` for the interner; integer literals keep their digits as text until M4 or M7 need values; `.gitattributes` keeps `tests/conformance/syntax/*.una` byte-exact (`-text`); M2's U7001 may require reformatting passing cases and explain examples.

## 7. Self-review checklist

Every MUST, MUST NOT and error rule of the milestone's spec sections (`spec/01`, `spec/02`), plus the rules of other sections that M1 implements. "Deferred" names the milestone that owns the rule. Test names are in `crates/una-syntax/tests/` unless a path is given; `case` means a conformance case in `tests/conformance/syntax/`.

### `spec/01` Lexical structure

| Rule | Status | Evidence |
| --- | --- | --- |
| §1.1 Valid UTF-8, no byte order mark (U0001) | Done | cases `invalid_utf8`, `byte_order_mark`; `lexer.rs` `a_byte_order_mark_is_reported_at_1_1_and_skipped`, `invalid_utf8_is_one_error_wherever_it_is` |
| §1.2 `\r\n` normalized; a lone `\r` is U0001 | Done | case `lone_carriage_return`; `source.rs` `crlf_becomes_lf_and_lone_carriage_returns_stay`, `crlf_lines_have_no_extra_column` |
| §1.3 Non-ASCII only in text and comments | Done | cases `non_ascii_identifier`, `non_ascii_in_text_and_comments` |
| §1.4 Extension `.una`, exactly one module per file | Partly | the module header is required (`parser.rs` `a_missing_module_header_is_reported_once_and_declarations_still_parse`); directories load only `*.una` (`una-cli/tests/cli.rs`). A single file named on the command line is not checked for its extension until the package loader of M3 replaces interim loading (ADR 0015 D3) |
| §1.5 Lexing continues after U0001; an ill-formed subsequence is one U+FFFD column | Done | `source.rs` `each_maximal_ill_formed_subsequence_becomes_one_replacement_character`; case `invalid_utf8` |
| §1.6 Control and bidirectional characters are U0001 everywhere | Done | cases `control_character_in_text`, `bidi_control_in_comment`, `bidi_escape_in_text`; `forbidden_characters_are_rejected_everywhere` |
| §1.7 A character that cannot start a token is U0001, once per character, with hints | Done | case `invalid_character`; `characters_that_cannot_start_a_token_have_hints` |
| §2.1 A tab in leading whitespace is U0002 | Done | cases `tab_in_indentation`, `tab_after_spaces_in_indentation`, `tab_in_text_and_comment`; `a_tab_in_indentation_reports_only_t0002_with_an_edit`, `a_tab_leading_a_comment_or_a_joined_line_is_t0002_without_layout` (D-013, D-017) |
| §2.2 Levels are exactly two spaces (U0003) | Done | cases `indentation_not_a_multiple_of_two`, `indentation_two_levels_deeper`, `dedent_to_unknown_level`, `nested_blocks`; `inconsistent_indentation_snaps_to_the_nearest_level` |
| §2.3 Blank and comment-only lines produce no layout tokens | Done | `blank_and_comment_only_lines_produce_no_layout` |
| §2.4 Lines join inside brackets; brackets MUST be balanced within a declaration (U0101) | Done | cases `unclosed_parenthesis`, `unmatched_closing_bracket`; `brackets_join_lines`, `brackets_left_open_close_at_the_next_declaration`, `closing_brackets_must_match` |
| §2.5 `NEWLINE` at the end of the last line, one `DEDENT` per open level | Done | `the_end_of_file_ends_the_line_and_closes_every_level` |
| §2.6 Trailing whitespace is U7001 | Deferred to M2 | `una fmt` and U7001 |
| §3 Comments; doc comments attach to the next declaration; orphans are plain (ADR 0017) | Done | `doc_comments_and_trailing_comments_are_trivia`; `parser.rs` `doc_comments_attach_to_the_next_declaration`, `doc_comments_of_a_broken_declaration_do_not_move` (D-021) |
| §4 Identifier and `cap_ref` patterns | Done | `keywords_contextual_keywords_and_reserved_words`, `capability_references` (D-014) |
| §4 Naming conventions (U1005) | Deferred to M3 | name resolution |
| §5 Reserved keywords cannot be identifiers | Done | case `keyword_as_name`; `parser.rs` `function_header_errors` |
| §5 Contextual keywords are names outside `requirement` and `policy` | Done | case `contextual_keywords_as_names`; `parser.rs` `contextual_keywords_are_names_outside_their_blocks`, `requirements`, `capabilities_handlers_and_policies` |
| §5 Words reserved for later versions are U0103 with the table's message | Done | cases `reserved_keyword_return`, `reserved_keyword_as_name`; `keyword_tables_match_spec_01_section_5` reads the tables from the spec; `parser.rs` `reserved_words_are_skipped_before_an_expression_and_read_as_names_elsewhere` |
| §6 Integers: decimal, `_` between digits, no leading zero (U0007); unbounded values | Done | cases `integer_leading_zero`, `integer_doubled_underscore`, `integer_trailing_underscore`; `integers_and_malformed_integers`; digits are kept as text (D-016) |
| §6 Text: single line (U0004), escapes (U0005), `}` MUST be escaped (U0007) | Done | cases `unterminated_text`, `unterminated_text_at_end_of_file`, `text_literals`, `unknown_escape`, `invalid_unicode_escapes`, `unescaped_brace_in_text`; `text_literals_decode_escapes_and_split_interpolations`, `invalid_escapes_are_reported_at_the_backslash` |
| §6 Interpolation MUST NOT contain a text literal (U0006); a missing `}` is U0004 at the opening quote | Done | cases `text_inside_interpolation`, `interpolation`, `unterminated_interpolation`; `a_text_literal_inside_an_interpolation_is_rejected`, `robustness.rs` `text_nested_in_interpolations_is_bounded` (ADR 0019) |
| §7 Precedence and associativity; comparisons do not chain (U0101) | Done | case `chained_comparison`; `parser.rs` `precedence_and_associativity`, `comparisons_do_not_chain` |
| §7 Ranges only after `in`, in `when` arms and in kind range sugar; `{` `}` only in text | Done | the grammar parses ranges only there (`parser.rs` `kind_bodies`, `when_and_patterns`, `precedence_and_associativity`); `{` outside text is U0001 |

### `spec/02` Grammar

| Rule | Status | Evidence |
| --- | --- | --- |
| Introduction: LL(2) except the kind body, which is a sum when its line has `|` at bracket depth zero | Done | two tokens of lookahead at most, and one line scan for kind bodies; `parser.rs` `kind_bodies`, `a_bar_inside_brackets_does_not_make_a_sum`, `a_sum_needs_two_variants_or_a_leading_bar`; the `[` after a label level stays LL(2) (ADR 0018, D-018) |
| §1 File: module header, imports, then declarations | Done | `parser.rs` `module_header_and_imports`, `imports_come_before_declarations` |
| §2 Kinds and types, including labels, function types and ranges | Done | `parser.rs` `kind_bodies`, `types`, `a_bracket_after_a_level_starts_label_categories`; case `grammar_tour` |
| §3 Predicates and functions in short and long form | Done | `parser.rs` `predicates`, `functions`, `function_header_errors`; case `missing_arrow` |
| §3 Clause order; at most one `fails` and one `derived from` (U0102) | Done | cases `clause_out_of_order`, `repeated_fails`, `repeated_derived_from`, `canonical_clause_order`; `parser.rs` `function_clauses_follow_the_canonical_order`, `clause_errors_do_not_stop_parsing` |
| §4 Capabilities, handlers and policies | Done | `parser.rs` `capabilities_handlers_and_policies`, `policy_lines_are_fixed` |
| §4 Operation clause order; at most one `fails` and one `accepts` (U0102, ADR 0017) | Done | cases `operation_clause_out_of_order`, `repeated_operation_accepts`, `canonical_clause_order`; `parser.rs` `operation_clauses_follow_the_canonical_order` |
| §5 Requirements and examples; `expect` tries `fails`, `performed` and `not performed` first | Done | `parser.rs` `requirements`, `examples` |
| §6 A block ends with a `handled` line or a block expression; `let` never ends a block | Done | case `block_ending_with_let`; `parser.rs` `block_shape_errors`, `if_forms`, `when_and_patterns` |
| §6 `with` bindings and handling clauses | Done | `parser.rs` `statements_and_handling_clauses` |
| §6 A bare handled statement MUST have type `Unit` (U2011) | Deferred to M4 | type checking |
| §7 Every expression form | Done | `parser.rs` `expression_forms`, `expression_errors`, `precedence_and_associativity` |
| §7 A bare `cap_ref` parses, to be reported as U3005 | Parsed; U3005 deferred to M5 | `parser.rs` `expression_forms` (`@log`); case `authority/cap_as_value` stays pending on M5 |
| §8 Context restrictions U2012, U2013, U2008 | Deferred to M4 | type checking |
| §8 Context restrictions U3013, U3005, U3010 | Deferred to M5 | capabilities |
| §8 Context restriction U4004 | Deferred to M6 | information flow |

### Other rules implemented in M1

| Rule | Status | Evidence |
| --- | --- | --- |
| `spec/06 §3` Handling clauses: `if error` before `if none`, each at most once (U0102) | Done (D-020) | cases `handling_clause_out_of_order`, `repeated_if_error`; `parser.rs` `handling_clauses_follow_the_canonical_order` |
| `spec/13 §1` Positions in scalar values; package-relative file names (ADR 0015 D2, D4) | Done | `source.rs` position tests; `una-cli/tests/cli.rs` `a_single_file_is_its_own_package_root`, `json_diagnostics_go_to_stdout` |
| `spec/13 §4` Every M1 code has an explain text; each failing example yields its code and each fixed example nothing | Done | `una-cli/tests/explain_examples.rs` |
| `spec/13 §5` Streams, exit codes, `--tier` (ADR 0015 D1) | Done | `una-cli/tests/cli.rs` `human_diagnostics_go_to_stderr`, `json_diagnostics_go_to_stdout`, `usage_errors_exit_with_2_and_leave_stdout_empty` |
| M1 acceptance: every example parses | Done | `examples.rs` `every_example_parses_without_diagnostics`, `example_syntax_trees_match_their_snapshots` (six reviewed snapshots); `una check examples/events` and `examples/hello` exit 0 without output |
| M1 acceptance: robustness (ADR 0017 decision 7) | Done | `robustness.rs` `every_prefix_of_every_example_is_parsed_safely` (168 prefixes) |
| No input crashes the lexer or parser (ADR 0019, D-022) | Done | `robustness.rs` deep nesting tests; `una-cli/tests/cli.rs` `deep_nesting_is_a_diagnostic_even_on_a_small_stack` |
| Pending cases cannot hide syntax errors | Done | the conformance runner fails any case with syntax errors it does not expect |
| Determinism (`compiler/testing.md §4`) | Done | `una-cli/tests/cli.rs` `check_json_is_byte_identical_across_runs`: every example and every syntax case, twice and with `RAYON_NUM_THREADS=1` |

### Known gaps carried forward

- **M2**: U7001 for trailing whitespace and non-canonical layout; the formatter can compare trees with `una_syntax::dump`, which ignores spans.
- **M3**: the package loader replaces interim loading (ADR 0015 D3) and checks file extensions and module paths (U1008); U1005 naming conventions. A declaration with a U0102 error stays in the tree, so later passes must not report errors caused by its repeated or misplaced clauses; the simplest rule is to use the first `fails` clause and to skip checks that depend on clause order.
- **M4 to M6**: the context restrictions of `spec/02 §8` and U2011.
- Every pass that recurses over the tree relies on its depth being at most `una_syntax::MAX_NESTING` and runs on the large stack of `una check` (ADR 0019).
