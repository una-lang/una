# M2 plan: formatter

Status: written during the autonomous run (`OVERNIGHT.md` C.2); not reviewed by the owner. Decisions are logged in `docs/reviews/open-decisions.md` and tagged `DECISION D-NNN` in the code.

Read list: `spec/01 §2`, `spec/02 §3`, `spec/04 §1, §3`, ADR 0011; also `spec/13 §1, §5` (U7001, `una fmt`), ADR 0012 (`una fmt --check --json`), `compiler/testing.md §1` (property tests), `compiler/architecture.md §3` (U7001 at tier 0).

## 1. What the spec fixes and what it leaves open

Fixed by the spec:
- two spaces per indentation level, no tabs (`spec/01 §2`, ADR 0011);
- no trailing whitespace (`spec/01 §2.6`);
- the short form of a function when it has no clauses and its body is a single line, the long form otherwise; no maximum line width (`spec/04 §1`);
- clause order (`spec/02 §3, §4`), already enforced by U0102;
- constructor arguments in declaration order (`spec/04 §3.4`);
- `una check` reports U7001 for non-canonical source, `una fmt` fixes it (ADR 0011, `spec/13 §5`).

Left open: comments, blank lines, spacing inside a line, lines continued inside brackets, the other `= body` forms, import order, sum syntax, line endings, and how U7001 is reported. ADR 0020 (Provisional) decides them; section 2 summarizes it.

## 2. The canonical form (ADR 0020)

1. **File.** LF line breaks only; the file ends with exactly one line break; no blank line at its start or end; no trailing whitespace.
2. **Logical lines.** Each logical line is printed on one line, indented two spaces per level: lines continued inside brackets are joined, unless a comment sits inside, which keeps the breaks with continuation lines one level deeper. Tokens keep their source spelling (names, integer literals with their `_`, text literals with their escapes).
3. **Spacing.** One space between tokens, except: none after `(`, `[` and `.`; none before `)`, `]`, `,`, `:` and `.`; none around `..` and `..<`; none after a unary `-` (unless another `-` follows); none between a name and `(` or `[` (calls, generic parameters, type arguments, capability types, label categories), nor in `fn(` and `checked(`; none inside text literals except in their interpolations, which are spaced like expressions without spaces inside the braces. An import keeps one space before its `(`.
4. **Bodies.** A function without clauses, predicate, handler operation or `when` arm whose body is a block of one plain line (no `with`, no handling clause, not a `when` or `if` block) is written on one line after its `=` or `->` (`spec/04 §1`, extended by D-024), unless a comment sits in between.
5. **Sums.** A sum with two or more variants has no leading `|`; a single variant keeps it, as the grammar requires.
6. **Constructors.** Arguments of a record or variant constructor are ordered by the declaration of its fields when every argument is labeled or punned and no comment sits in the line (`spec/04 §3.4`). In M2 this applies to kinds and variants declared in the same file; imported ones follow in M3 (D-026).
7. **Imports.** Imports are sorted by module path, and the names of each import bytewise (D-025).
8. **Blank lines.** Exactly one between top-level items (the header, the imports, each declaration), none between imports. Elsewhere, runs of blank lines become one blank line, and none follows a line that opens a block or surrounds the `=` line of the long form.
9. **Comments.** The formatter never moves a comment to another line. Comment text is kept, minus trailing whitespace. A trailing comment stays at the end of its line after one space. A comment on its own line keeps its place, at the indentation level nearest to its column among the levels of the surrounding lines. A comment at level 0 between two declarations leads the next one.

Files with syntax errors are neither formatted nor checked for U7001 (D-027).

## 3. Tasks and commits

1. **Plan and ADR** (`docs(plan)`): this plan, ADR 0020, decision log entries.
2. **Source support** (`feat(syntax)`): `SourceFile` records the first `\r\n`, for U7001; `.gitattributes` keeps `.una` files LF on checkout.
3. **Formatter** (`feat(fmt)`, task 1): `una_fmt::format` from tokens, trivia and the AST; unit tests for every rule; property tests over every `.una` file of the repository: idempotence, round trip modulo the formatter's rewrites (D-030), and comments preserved.
4. **U7001 and `una fmt`** (`feat(cli)`, task 2): `una_fmt::differences` turns the line diff between a file and its canonical form into one U7001 per changed hunk, with a classified message and a mechanical edit (D-028); `una check` reports U7001 at tier 0; `una fmt [path]` rewrites files and `una fmt --check` reports without writing (D-029). CLI tests.
5. **Canonical repository** (`style`): the examples and every conformance case pass `una fmt --check`; changed snapshots reviewed.
6. **Conformance and explain** (`test(fmt)`, `docs(diag)`): the `fmt` cases of section 4 and the U7001 explain text.
7. **Self-review** (`docs(plan)`): checklist, progress, report.

## 4. Conformance cases (`tests/conformance/fmt/`)

| Case | Content | Expected |
| --- | --- | --- |
| `trailing_whitespace` | spaces after a declaration | U7001 at the line |
| `missing_final_line_break` | no `\n` at the end | U7001 at the last line |
| `crlf_line_breaks` | `\r\n` line breaks (byte-exact) | U7001 at the first line |
| `missing_blank_line` | two declarations without a blank line | U7001 at the second |
| `extra_blank_lines` | three blank lines between declarations | U7001 at the extra lines |
| `long_form_without_clauses` | `pub f() -> Int` / `=` / `  1` | U7001 at the signature |
| `operator_spacing` | `1+2*3` | U7001 at the line |
| `joined_brackets` | a call continued over three lines | U7001 at its first line |
| `comment_indentation` | a comment indented by 3 spaces | U7001 at the comment |
| `constructor_field_order` | `Point(y: 2, x: 1)` | U7001 at the line |
| `sum_leading_bar` | `= \| Small \| Large` | U7001 at the line |
| `inline_arm_body` | an arm body on its own line | U7001 at the arm |
| `canonical_layout` | comments, doc comments, trailing comments, blank lines in a body, nested blocks, a single-variant sum | tier0-ok |

Import sorting is covered by unit tests; its conformance case needs two modules and waits for the package model of M3.

## 5. Acceptance criteria

| Criterion | Evidence |
| --- | --- |
| Idempotence for every `.una` file in the repository | `crates/una-fmt/tests/properties.rs` over every file that parses |
| Round trip modulo spans | the same test compares the trees without spans, after applying the formatter's documented rewrites to the original (D-030), and checks that every comment survives |
| All examples are canonical | the same test, and `una fmt --check examples/events examples/hello` |
| U7001 with a mechanical edit; `una fmt`, `una fmt --check` | conformance cases of section 4; CLI tests |

## 6. Known gaps

- Constructor field order for imported kinds, and a conformance case for import order, wait for the package model of M3.
- `una fmt` formats the files `una check` loads (ADR 0015 D3) until M3 replaces interim loading.

## 7. Self-review checklist

Every rule of the milestone's read list (`spec/01 §2`, `spec/02 §3`, `spec/04 §1, §3`, ADR 0011) and of the command it adds (`spec/13 §5`). Test names are in `crates/una-fmt/tests/` unless a path is given; `case` means a conformance case in `tests/conformance/fmt/`.

| Rule | Status | Evidence |
| --- | --- | --- |
| `spec/01 §2.1, §2.2` Two spaces per level, no tabs | Done | the formatter indents two spaces per level (`format.rs` `blank_lines_inside_declarations`, `comment_spacing_and_indentation_are_canonical`); tabs and wrong levels are U0002 and U0003 (M1), so such files are not formatted (D-027) |
| `spec/01 §2.3` Blank and comment-only lines have no layout | Done | comments keep their lines and snap to a level (`comments_keep_their_text_and_place`); case `comment_indentation` |
| `spec/01 §2.4` Lines join inside brackets | Done | continued lines are joined (`lines_continued_inside_brackets_are_joined`, case `joined_brackets`), unless a comment sits inside (`a_comment_inside_a_line_keeps_its_breaks_and_blocks_rewrites`) |
| `spec/01 §2.6` Trailing whitespace is U7001, reported by `una check`, fixed by `una fmt` | Done | case `trailing_whitespace`; `una-cli/tests/cli.rs` `fmt_rewrites_non_canonical_files_and_leaves_files_with_syntax_errors` |
| `spec/02 §3` Clause order is canonical | Done | enforced by U0102 (M1); the formatter keeps clauses in place, and files with U0102 are not formatted |
| `spec/04 §1` Short form when there are no clauses and the body is one line, long form otherwise; `una fmt` converts; no width limit | Done | `a_function_without_clauses_whose_body_is_one_line_uses_the_short_form`; case `long_form_without_clauses`; lines are never broken for width |
| `spec/04 §3.4` The formatter orders constructor arguments by declaration | Done for kinds declared in the file (D-026) | `constructor_arguments_follow_the_declaration`; case `constructor_field_order`; imported kinds follow in M3 |
| `spec/04 §3.1 to §3.3, §3.5` and method-call syntax | Deferred to M4 and M7 | labels (U2004 to U2006), punning and evaluation order are type checking and interpretation |
| ADR 0011 U7001 in `una check`, fixed by `una fmt`, mechanical edits | Done | `una-cli/tests/cli.rs` `check_reports_t7001_with_a_mechanical_edit`; every fmt case |
| `spec/13 §5` `una fmt [path]`, `--check` without writing | Done (D-029) | `una-cli/tests/cli.rs` `fmt_rewrites_non_canonical_files_and_leaves_files_with_syntax_errors`, `fmt_converts_line_breaks_to_line_feeds` |
| ADR 0012 `una fmt --check --json` prints the diagnostic list | Done | the same CLI tests |
| `spec/13 §4` U7001 has an explain text whose examples behave | Done | `una-cli/tests/explain_examples.rs` |
| ADR 0020 rules 1 to 10 | Done | `format.rs` (one test per rule), `diff.rs` unit tests, the fmt cases |
| M2 acceptance: idempotence for every `.una` file | Done | `properties.rs` over every file of the repository that parses |
| M2 acceptance: round trip modulo spans | Done (D-030) | `properties.rs` compares the trees after the formatter's documented rewrites and checks that every comment survives |
| M2 acceptance: all examples are canonical | Done | `properties.rs`; `una fmt --check examples/events` and `examples/hello` exit 0 |
| Determinism (`compiler/testing.md §4`) | Done | `una-cli/tests/cli.rs` `check_json_is_byte_identical_across_runs` covers every example and every syntax and fmt case |

### Known gaps carried forward

- **M3**: constructor order for imported kinds and variants (D-026), and a conformance case for import order, which needs two modules.
- **M3**: the package loader replaces interim loading; `una fmt` follows it.
- Every later milestone adds conformance cases that must be canonical; `una fmt` makes them so.
