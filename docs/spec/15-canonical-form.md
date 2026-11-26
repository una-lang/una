# 15. Canonical form

Status: Normative, v0.1.

Every source file has one canonical form (charter P4, ADR 0011). `una fmt` MUST rewrite a file into it, and `una check` MUST report every difference from it as U7001. This chapter defines the canonical form of a file that has no syntax error (ADR 0020). A file with a syntax error has no canonical form as a whole: it is not formatted, and only its declarations that parsed are checked for U7001, each against its canonical text (ADR 0096 rule 10). Its blank lines between declarations, the order of its imports and its final line break are checked once it parses.

## 1. File

Line breaks are `\n`. The file ends with exactly one line break, no line has trailing whitespace, and the file neither starts nor ends with a blank line.

## 2. Lines and indentation

In editions 0.1 to 0.3, every logical line is one line: lines continued inside brackets are joined, since no line width applies, unless a comment sits inside the line; then its line breaks are kept and each continuation line is indented one level deeper than the line. Each line is indented two spaces per level (`spec/01 §2`). Every token keeps its source spelling, so integer literals keep their `_` groups and text literals their escapes.

In edition 0.4 and later, lines continued inside brackets are joined in the same way, and then a logical line wider than 100 Unicode scalar values wraps (ADR 0125 and its amendment of 2026-10-25). This is the canonical form of edition 0.4 and of edition 0.5, which has every rule of edition 0.4 (`spec/13 §7`); editions 0.1 to 0.3 never wrap.

1. **Separators.** A group is a pair of `(` and `)` or of `[` and `]` on the logical line, outside text interpolation. Its level is the number of groups of the logical line that hold it, itself included: an outermost group has level 1. A separator is a comma directly inside a group, between arguments or elements, and has its group's level. A line breaks only after a separator: never inside a token, at an operator, after an opener or before a comma.
2. **Printed lines.** A printed line starts at the start of the logical line, right after the separator that ended the printed line before it, or at the closer of a wrapped group, and runs to the closer of the next wrapped group or to the end of the logical line. Its width is two per indentation level plus every Unicode scalar value of its text on one line, including the closers of groups that do not wrap and the text after its last separator.
3. **Breaks.** When a printed line is wider than 100, it ends at a separator instead, and that separator's group is wrapped. Of the separators whose text up to and including them fits within 100, the one with the lowest level is chosen, the latest of those on a tie, so an inner group breaks only when no outer separator keeps the line within 100. When none fits, the line ends at its first separator. A line without separators stays whole and may be wider than 100.
4. **Indentation.** For a logical line at level `d`, the line after a separator of level `l` is indented at level `d + l`. The closer of a wrapped group of level `l` starts a line at level `d + l - 1`, and the text after the closer follows it on that line.
5. **Comments.** A logical line with a comment inside it or at its end keeps its source line breaks, each continuation line one level deeper than the line, and does not wrap.

Finding the printed lines reads each token of the logical line a bounded number of times: linear in its length.

## 3. Spacing

Tokens are separated by one space, except:

- no space after `(`, `[` or `.`, and none before `)`, `]`, `,`, `:` or `.`;
- no space around `..` and `..<`;
- no space after a unary `-`, unless another `-` follows;
- no space between a name and a following `(` or `[`, and none in `fn(` and `checked(`, but one space between an import's module path and its `(`;
- no space inside a text literal outside its interpolations, and none just inside the braces of an interpolation.

(v0.5, ADR 0137 rule 9.) In edition `0.5`, `una fmt` writes `const x = e`, `const x: T = e`, `var x = e` and `var x: T = e`, with one space after the keyword, around `=` and after `:`, as it writes `let` in earlier editions: `var x :Int=1` becomes `var x: Int = 1`. A file of edition `0.5` with a `let` holds U7007, and `una fmt` refuses it, as it refuses any file with a syntax error; `una fix` comes first.

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
