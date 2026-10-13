# 15. Canonical form

Status: Normative, v0.1.

Every source file has one canonical form (charter P4, ADR 0011). `una fmt` MUST rewrite a file into it, and `una check` MUST report every difference from it as U7001. This chapter defines the canonical form of a file that has no syntax error (ADR 0020). A file with a syntax error has no canonical form as a whole: it is not formatted, and only its declarations that parsed are checked for U7001, each against its canonical text (ADR 0096 rule 10). Its blank lines between declarations, the order of its imports and its final line break are checked once it parses.

## 1. File

Line breaks are `\n`. The file ends with exactly one line break, no line has trailing whitespace, and the file neither starts nor ends with a blank line.

## 2. Lines and indentation

Every logical line is one line: lines continued inside brackets are joined, since no line width applies, unless a comment sits inside the line; then its line breaks are kept and each continuation line is indented one level deeper than the line. Each line is indented two spaces per level (`spec/01 §2`). Every token keeps its source spelling, so integer literals keep their `_` groups and text literals their escapes.

## 3. Spacing

Tokens are separated by one space, except:

- no space after `(`, `[` or `.`, and none before `)`, `]`, `,`, `:` or `.`;
- no space around `..` and `..<`;
- no space after a unary `-`, unless another `-` follows;
- no space between a name and a following `(` or `[`, and none in `fn(` and `checked(`, but one space between an import's module path and its `(`;
- no space inside a text literal outside its interpolations, and none just inside the braces of an interpolation.

## 4. One-line bodies

A function without clauses whose body is a block of one plain line is written in the short form (`spec/04 §1`). The same holds for the body of a predicate or handler operation after its `=`, and of a `when` arm after its `->`. A plain line is a statement with no `with` clause and no handling clause, whose expression is not a `when` or an `if` with blocks. (v0.2, `spec/17 §3` rule 5.) Nor is it a call with an `edit` or `take` marker, which is legal only as a statement, so its block form stays. The long form stays when a comment sits between the head and the end of the body, since the formatter never moves a comment.

## 5. Sums

A sum with two or more variants has no leading `|`. A sum with one variant keeps its leading `|`, which the grammar requires (`spec/02 §2`).

## 6. Constructors

When every argument of a record or variant constructor is labeled or punned, the arguments follow the order in which the fields are declared (`spec/04 §3` rule 4). This applies to the record kinds and variants a file declares or imports, and to the variants of the sum kinds it imports, when the name comes from exactly one of them and is declared once in its module. Arguments are not reordered on a line with a comment inside.

## 7. Imports

Imports are sorted bytewise by module path, stably, with the comments that lead them, and the names inside each import bytewise unless a comment sits inside the import.

## 8. Blank lines

Top-level items (the module header, each import and each declaration) are separated by exactly one blank line, except consecutive imports, which have none. Elsewhere, a run of blank lines becomes one blank line, except that no blank line follows a line opening a block (the next line is deeper), and none precedes or follows the `=` line of the long form. Blank lines before the module header and after the last declaration also collapse to one.

## 9. Comments

The formatter never moves a comment to another line, and keeps its text without trailing whitespace. A comment that ends a line of code stays there, after one space. A comment on its own line stays in place, at the indentation level nearest to its column (ties go deeper) among the levels of the lines around it; inside a line with kept breaks, one level deeper than the line. A comment on its own line at level 0 between two top-level items belongs to the next item, and the item's blank line goes before it.

## 10. U7001

`una check` compares each file with its canonical form line by line and reports one U7001 per changed region, at its first source line, with a mechanical edit that replaces the region with its canonical text. The message names the difference: trailing whitespace, a missing or extra blank line, a missing final line break, or lines that are not in canonical form. A file with `\r\n` line breaks gets one U7001 at the first of them, without an edit, since positions count decoded text.
