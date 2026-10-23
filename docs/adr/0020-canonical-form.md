# 0020. The canonical form

- Status: Accepted
- Date: 2026-09-29
- Deciders: Leonardo (accepted on 2026-09-29), on a proposal by Claude (overnight run)

## Context

ADR 0011 makes canonical formatting part of the language: `una check` reports U7001 for non-canonical source and `una fmt` fixes it. The specification fixes only part of the canonical form: two spaces per indentation level and no tabs (`spec/01 §2`), no trailing whitespace (`spec/01 §2.6`), the short form of a function without clauses whose body is one line (`spec/04 §1`), the clause order (`spec/02 §3, §4`, enforced by U0102), constructor arguments in declaration order (`spec/04 §3.4`), and no maximum line width. Milestone M2 needs every other rule: comments, blank lines, spacing, continued lines, the other `= body` forms, sums, imports, line endings, and the shape of U7001. Since U7001 rejects every program that is not canonical, these rules are part of the language.

Decision log entries: D-023 to D-031 (`docs/history/v0.1/open-decisions.md`).

## Decision

The canonical form of a file that has no syntax error is defined by these rules. A file with a syntax error has no canonical form: it is neither formatted nor checked for U7001.

1. **File.** Line breaks are `\n`; the file ends with exactly one line break; no line has trailing whitespace; the file neither starts nor ends with a blank line.
2. **Lines and indentation.** Every logical line is one line: lines continued inside brackets are joined, since no line width applies, unless a comment sits inside the line; then its line breaks are kept and each continuation line is indented one level deeper than the line. Each line is indented two spaces per level. Every token keeps its source spelling, so integer literals keep their `_` groups and text literals their escapes.
3. **Spacing.** Tokens are separated by one space, except: no space after `(`, `[` or `.`; no space before `)`, `]`, `,`, `:` or `.`; no space around `..` and `..<`; no space after a unary `-`, unless another `-` follows; no space between a name and a following `(` or `[`, and none in `fn(` and `checked(`, but one space between an import's module path and its `(`; no space inside a text literal outside its interpolations, and none just inside the braces of an interpolation.
4. **One-line bodies.** A function without clauses whose body is a block of one plain line is written in the short form (`spec/04 §1`). The same holds for the body of a predicate or handler operation after its `=`, and of a `when` arm after its `->`. A plain line is a statement with no `with` clause and no handling clause, whose expression is not a `when` or an `if` with blocks. The long form stays when a comment sits between the head and the end of the body, since the formatter never moves a comment.
5. **Sums.** A sum with two or more variants has no leading `|`. A sum with one variant keeps its leading `|`, which the grammar requires.
6. **Constructors.** When every argument of a record or variant constructor is labeled or punned, the arguments follow the order in which the fields are declared (`spec/04 §3.4`). This applies to the record kinds and variants a file declares or imports, and to the variants of the sum kinds it imports, when the name comes from exactly one of them and is declared once in its module: M2 ordered the kinds of the same file, and M10 added the imported ones (D-026). Arguments are not reordered on a line with a comment inside.
7. **Imports.** Imports are sorted bytewise by module path, stably, with the comments that lead them, and the names inside each import bytewise unless a comment sits inside the import.
8. **Blank lines.** Top-level items (the module header, each import and each declaration) are separated by exactly one blank line, except consecutive imports, which have none. Elsewhere, a run of blank lines becomes one blank line, except that no blank line follows a line opening a block (the next line is deeper), and none precedes or follows the `=` line of the long form. Blank lines before the module header and after the last declaration also collapse to one.
9. **Comments.** The formatter never moves a comment to another line, and keeps its text without trailing whitespace. A comment that ends a line of code stays there, after one space. A comment on its own line stays in place, at the indentation level nearest to its column (ties go deeper) among the levels of the lines around it; inside a line with kept breaks, one level deeper than the line. A comment on its own line at level 0 between two top-level items belongs to the next item, and the item's blank line goes before it.
10. **U7001.** `una check` compares each file with its canonical form line by line and reports one U7001 per changed region, at its first source line, with a mechanical edit that replaces the region with its canonical text. The message names the difference: trailing whitespace, a missing or extra blank line, a missing final line break, or lines that are not in canonical form. A file with `\r\n` line breaks gets one U7001 at the first of them, without an edit, since positions count decoded text.

### Changes to other documents on acceptance (made on 2026-09-29)

- A new `spec/15-canonical-form.md` states rules 1 to 10 normatively, and `spec/04 §1` refers to it.

## Consequences

- Two agents that write the same program converge on the same bytes, which is the goal of ADR 0011.
- Every rule is mechanical, so `una fmt` applies it and U7001 carries an edit.
- The rules are stricter than the specification requires (rules 4, 5 and 7 go beyond `spec/04 §1`); relaxing any of them later breaks no canonical program.
- Comments never move, so documentation stays attached where it was written; the price is that a comment inside a line or a body keeps that line's breaks or the long form.

## Alternatives considered

- **Keep the author's layout except where the specification requires otherwise:** leaves many canonical forms of one program, against ADR 0011 and charter P4.
- **A width limit with line breaking:** `spec/04 §1` rules it out for v0.1.
- **Aligned trailing comments:** alignment changes whenever a line changes, which makes unrelated lines differ in every diff.
- **Keeping import order:** two agents adding imports in a different order would produce different files.
- **Reporting U7001 once per file:** an edit for the whole file hides where the differences are and grows with the file.

### Amendment on 2026-10-23 (ADR 0125)

Rule 2's statement that no line width applies is superseded by ADR 0125. The canonical formatter targets 100 Unicode scalar values and wraps only after grammar-valid comma separators between arguments or elements inside parentheses or square brackets. A continuation is indented two spaces deeper than the containing delimiter, with two more spaces per nested delimiter. A wrapped group closes on its own line at the opener's indentation, keeping any following comma on that line. This is a soft target: an unbreakable segment may exceed it. A logical line containing a comment keeps its source line breaks and comment placement. The rule applies to every edition; it does not change parsing or semantics. ADR 0125 defines the deterministic break choice and defers repository-wide reformatting to a separate change.
