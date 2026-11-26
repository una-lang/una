# 01. Lexical structure

Status: Normative, v0.1.

## 1. Source text

1. A source file MUST be valid UTF-8 and MUST NOT start with a byte order mark (U0001).
2. Line endings `\r\n` are normalized to `\n` before lexing. A lone `\r` is an error (U0001).
3. Identifiers and keywords are ASCII. Non-ASCII characters MAY appear only inside text literals and comments (U0001 elsewhere).
4. A source file has the extension `.una` and contains exactly one module.
5. After U0001, lexing continues. Each maximal ill-formed subsequence of invalid UTF-8 is read as U+FFFD and counts as one column (`spec/13 §1`, ADR 0015).
6. Control characters U+0000 to U+001F other than tab and line feed, U+007F, and the bidirectional controls U+202A to U+202E and U+2066 to U+2069 are an error anywhere in a file, including text literals and comments (U0001, ADR 0017). Text writes them with `\u{...}` escapes.
7. A character that cannot start a token where it appears, such as `;` or `&`, is an error (U0001), reported once per character (ADR 0017).

## 2. Lines and indentation

Una uses significant indentation. The lexer converts leading whitespace into `INDENT` and `DEDENT` tokens using an indentation stack, as follows.

1. Indentation uses spaces only. A tab in leading whitespace is an error (U0002).
2. Each indentation level is exactly 2 spaces deeper than the enclosing one (U0003 otherwise).
3. Blank lines and lines containing only a comment do not produce `NEWLINE`, `INDENT` or `DEDENT`.
4. Inside `(` `)` and `[` `]`, newlines and indentation are ignored (implicit line joining). Brackets MUST be balanced within a declaration (U0101).
5. A logical line ends with a `NEWLINE` token. At end of file the lexer emits `NEWLINE` if the last logical line lacks one, then one `DEDENT` per open level.
6. Trailing whitespace is non-canonical (U7001, reported by `una check` and fixed by `una fmt`).

Algorithm, for each logical line with indentation `n` and stack top `t`:

```text
if n == t:            emit NEWLINE before the line (except the first line)
if n == t + 2:        push n; emit INDENT
if n > t + 2:         error U0003
if n < t:             pop while top > n, emitting DEDENT per pop; if top != n: error U0003
```

## 3. Comments

- `--` starts a comment that runs to the end of the line.
- `---` starts a doc comment. Consecutive doc comment lines attach to the next declaration and appear in `una contract` output.
- There are no block comments.
- A doc comment not followed by a declaration is an ordinary comment (ADR 0017).
- A declaration's doc comments are every doc comment between the end of the preceding token (the previous declaration, import or module header) and the declaration's first token, even when blank lines or ordinary comments separate them from the declaration, so a doc comment below the module header attaches to the file's first declaration.

## 4. Identifiers and naming conventions

| Token | Pattern | Used for |
| --- | --- | --- |
| `lower_ident` | `[a-z][a-z0-9_]*` | Values, parameters, fields, functions, predicates, handlers, policies, modules, capability operations |
| `upper_ident` | `[A-Z][A-Za-z0-9_]*` | Kinds, variants, capabilities, label categories, requirement identifiers |
| `cap_ref` | `@` immediately followed by `lower_ident` | Capability references (`@audit`) |

Convention checks (U1005): `lower_ident` MUST NOT contain `__` or end with `_`. Kinds, variants, capabilities and categories use UpperCamelCase (`[A-Z][a-zA-Z0-9]*`). Requirement identifiers use upper snake case ending in a number (`EVENTS_3`, pattern `[A-Z][A-Z0-9]*(_[A-Z0-9]+)*_[0-9]+`).

## 5. Keywords

**Reserved keywords** cannot be used as identifiers in any context:

```text
accepts and attempt capability checked confidential derived do else ensures
error example expect fail fails false fn for from given handler if implies
import in internal is it kind let mod module none not op opaque or pass
performed policy predicate pub public reflect requirement requires restricted
result secret then true trusted unit use uses when where with
```

**Contextual keywords** are keywords only inside `requirement` and `policy` blocks, and identifiers elsewhere:

```text
active generated human owner proposed purpose regulation source status text to via
```

(v0.3, ADR 0094 rule 6.) `gives` is a contextual keyword: in edition `0.3`, after an operation name inside the brackets of a header's `uses` (`spec/02 §3`, `spec/05 §3` rule 6), and an identifier everywhere else, in every edition. A program of edition `0.1` or `0.2` that writes it there gets U0103 at the word, "`gives` is a keyword of edition 0.3 (operation labels)"; the word is not reserved in those editions, so a function, a local or an operation named `gives` stays accepted.

(v0.3, ADR 0104.) In edition `0.3`, `measure` is a keyword only at the start of a top-level declaration, after an optional `pub` and before an upper identifier, and `of` is one after an integer type (`spec/25`). Both are identifiers elsewhere.

**Reserved, not available in v0.1.** Using any of these as an identifier or keyword is an error (U0103). The message names either the planned version and feature, or the alternative in Una. It is written `` `<keyword>` is <message> `` with the message of the table below, for example `` `match` is not a keyword in Una; use `when` `` (ADR 0014).

```text
actor all async await commit costs edit impl iso law loop macro match observe
parallel profile query region return service shared simd table take together
traced trait type unsafe var while
```

| Keywords | Message |
| --- | --- |
| `edit`, `take`, `region`, `shared`, `iso` | reserved for v0.2 (memory model) |
| `together`, `parallel`, `actor` | reserved for v0.2 (concurrency) |
| `simd`, `profile` | reserved for v0.2 (systems profile) |
| `trait`, `impl` | reserved for v0.2 (constraints on generics) |
| `law`, `all` | reserved for a later edition (laws) |
| `costs` | reserved for a later edition (cost contracts) |
| `traced` | reserved for a later edition (provenance tracing) |
| `service`, `table`, `query`, `observe`, `commit` | reserved for a later edition (platform) |
| `match` | not a keyword in Una; use `when` |
| `return` | not a keyword in Una; the last expression is the result |
| `var` | not a keyword in Una; values are immutable, use `let` |
| `while`, `loop` | not a keyword in Una; use recursion or `fold` |
| `type` | not a keyword in Una; use `kind` |
| `unsafe` | not a keyword in Una; use a `trusted` module |
| `macro` | not supported (charter §6) |
| `async`, `await` | not supported; see structured concurrency in v0.2 |

(2026-11-25, #836.) A word reserved for a later version that names something carries a rename edit (`spec/13 §3`), the same new name at every occurrence in the package. The edit is offered only where the program with the package's renames applied parses there: where the word stands as the keyword of its later construct, as `edit` in `let edit total = 0` or `take` in `f(take x)` of edition `0.1`, no name keeps the program's meaning, so the U0103 carries no edit and `una fix` leaves the word as it is.

(2026-11-10, #1899, ADR 0014 Consequences.) The messages of `law`, `all`, `costs`, `traced`, `service`, `table`, `query`, `observe` and `commit` say "reserved for a later edition" instead of naming v0.3 or v0.4, since no release shipped those features; the same holds for `shared`, `iso` and `weak` in edition `0.2` (`spec/17 §9`). The words stay reserved, with the same code, in every edition.

(v0.2, ADR 0056.) **Editions.** The keyword tables of this section are those of edition `0.1`, and a package of edition `0.1` keeps them in every version of the compiler. Only a package of edition `0.2` (`spec/13 §7`) gets the memory model's keywords (`spec/17`). There:
- `edit`, `take` and `old` become keywords;
- `shared` and `iso` move to v0.3, and `weak` joins them;
- the messages of `region`, `var`, `while` and `loop` change.

The new rows are in `spec/17 §9`. (v0.2, ADR 0065.) In edition `0.2`, `together` also becomes a keyword (`spec/19`), and the messages of `parallel`, `actor`, `async` and `await` change (`spec/19 §9`).

(v0.3, ADR 0105.) In edition `0.3`, `test` is a reserved keyword: the keyword of the test declaration (`spec/08 §2`), which editions `0.1` and `0.2` write `example`. A name `test` there is U0101, as any keyword used as a name: a local bound inside a function body or a handler operation body gets the exact rename to `test_value` (`spec/13 §3`, ADR 0096), and a function, a field, a handler, a predicate or a module path segment gets the rename as advice. `example` stays reserved in edition `0.3`, so it never comes back with another meaning: at column 1, where a declaration starts, it is U0103 with the message of the table below and the exact rewrite of the keyword to `test`, which `una fix` applies; the declaration is then read as a test's, so its body gets its own checks and no U0101. Anywhere else `example` is the keyword it is in editions `0.1` to `0.3`. Editions `0.1` and `0.2` do not change: `test` stays an identifier there, and `example` is the keyword of the test declaration.

| Keywords | Message (edition `0.3`) |
| --- | --- |
| `example` | written `test` in edition 0.3 |

(v0.4, ADR 0116.) In edition `0.4`, `diverges` and `within` are reserved as names (U0103): `diverges` is a word of a header and `within` the word of a deadline, and anywhere else neither can be a name (`spec/26 §1`). (Edition 0.4, ADR 0117 rule 5.) `within` is the word of a deadline right after `for ever` and right after a token that ends an expression (a name, a capability reference, a literal, `)` or `]`): there it is no name, and anywhere else it stays reserved (`spec/27 §5`). (Edition 0.4, ADR 0117 rule 6.) `first`, followed by `of` and the end of its line, opens a race (`spec/27 §6`); elsewhere `first` and `of` are names, so the prelude's `first` stays a method. (Edition 0.4, ADR 0117 rule 7.) `tasks` and `start` begin their statements where `spec/27 §7` rule 1 says, and are names elsewhere; the message of `async` and `await` becomes `` not supported; use `tasks` and `start`, or `together` `` (`spec/19 §9`). (v0.5, ADR 0120 rule 1, moved from edition `0.4` by ADR 0139.) `simd` stays reserved in edition `0.4`, with the message of the table above, its rename, and the advice to raise the edition of `spec/13 §7`. In edition `0.5`, `simd` is no longer reserved: it is an identifier, and a contextual keyword only in the `simd required` clause once that clause lands (`spec/29`). (v0.5, ADR 0120 rule 2.) In edition `0.5`, `with` and `and` right before `:` are also the labels of a call's arguments, as in `a.dot(with: b)` and `c.add_product(of: a, and: b)` (`spec/29 §17`), and keywords everywhere else; in editions `0.1` to `0.4` they are keywords there too, and such a call is a syntax error (U0101). (Edition 0.4, ADR 0105 amendment of 2026-11-05.) In edition `0.4`, `example` is neither a keyword nor reserved: it is an identifier everywhere, so it names a local, a parameter, a function, a field or a module path segment, and a line `example "..."` at column 1 is an ordinary syntax error (U0101), with no U0103 and no rewrite to `test`. The test declaration is written `test`, as in edition `0.3`; `una fix` rewrites `example` to `test` in edition `0.3` packages only. (v0.2, ADR 0066.) The message of `trait` and `impl` changes too (`spec/20 §9`). (v0.2, ADR 0069.) In edition `0.2`, `with` right after `.` is an identifier, the name of a map's method (`spec/21 §2`), and a keyword everywhere else. The benchmark baseline stays on edition `0.1`. (v0.3, ADR 0056 amendment.) Edition `0.3` has the keyword tables of edition `0.2` (`spec/13 §7`): editions are cumulative.

(v0.5, ADR 0137 rule 1.) **The binding words of edition `0.5`.** In edition `0.5`:
- `const` and `var` are reserved keywords: `const` begins an immutable binding and `var` one that can be assigned (`spec/02 §6`, `spec/17 §2`);
- `let` is reserved: it is no keyword and no name. Each occurrence is U7007 at the word, the code of a form a preview edition changed (`spec/13 §4`), with the message `` this form of edition 0.5 changed in release v0.5.2: `let` is written `const`, or `var` for a binding that is reassigned ``. Its first repair is the exact rewrite, which `una fix` applies: `let edit x` becomes `var x`, and `let x` becomes `const x`, keeping the annotation and the value;
- the row of `var` in the U0103 tables is removed, since it is a keyword;
- every other row is that of edition `0.4`.

A name `const` or `var` in edition `0.5` is U0101, as any keyword used as a name. In editions `0.1` to `0.4` nothing changes: `let` is a keyword, `var` gives the U0103 message of its edition, and `const` is an identifier. A line that starts with `const` or `var` followed by a lower-case name, at the top level or in a block, keeps its diagnostic (U0101 for `const`, U0103 for `var`) and gains the help `` `const` and `var` are keywords of edition 0.5 ``.

## 6. Literals

| Literal | Syntax | Notes |
| --- | --- | --- |
| Integer | `0`, `42`, `1_000_000` | Decimal only. `_` only between digits. No leading zeros except `0` itself. Negative numbers use unary `-`. Values are unbounded (ADR 0004). A leading zero, or a doubled, leading or trailing `_`, is a malformed literal (U0007, ADR 0017). |
| Text | `"..."` | Single line. Escapes: `\"`, `\\`, `\n`, `\t`, `\{`, `\}`, `\u{H..H}` (1 to 6 hex digits, valid scalar value). Unknown escape: U0005. Unterminated: U0004. A `}` outside an interpolation MUST be escaped as `\}` (U0007, ADR 0017). A `{` that no expression can follow, a quote, a backslash or a `}` after any blanks, is a literal brace written without its escape: U0007, with an edit to `\{`, and the literal continues (#403). |
| Interpolation | `"seats {e.remaining}"` | `{` expr `}` inside a text literal. The expression MUST NOT contain a text literal (U0006). A missing `}` leaves the literal unterminated (U0004 at its opening quote, ADR 0017). |
| Boolean | `true`, `false` | |
| Unit | `unit` | The only value of type `Unit`. |

## 7. Operators, punctuation and precedence

Punctuation: `(` `)` `[` `]` `,` `:` `->` `=` `|` `.` `..` `..<` and `{` `}` (only inside text literals).

Operator precedence, highest first:

| Level | Operators | Associativity |
| --- | --- | --- |
| 1 | call `f(..)`, field or method `.name`, capability operation `@c.op(..)` | left |
| 2 | unary `-` | prefix |
| 3 | `*` `/` `mod` | left |
| 4 | `+` `-` | left |
| 5 | `==` `!=` `<` `<=` `>` `>=` `is` `in` | none (chaining is U0101) |
| 6 | `not` | prefix |
| 7 | `and` | left |
| 8 | `or` | left |
| 9 | `implies` | right |

Range operators `..` (inclusive) and `..<` (exclusive upper bound) appear only as the right operand of `in`, in `when` arms and in the range sugar of kinds (`spec/03 §3`). They are not general expressions.
