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
| `law`, `all` | reserved for v0.3 (laws) |
| `costs` | reserved for v0.3 or later (cost contracts) |
| `traced` | reserved for v0.3 (provenance tracing) |
| `service`, `table`, `query`, `observe`, `commit` | reserved for v0.4 (platform) |
| `match` | not a keyword in Una; use `when` |
| `return` | not a keyword in Una; the last expression is the result |
| `var` | not a keyword in Una; values are immutable, use `let` |
| `while`, `loop` | not a keyword in Una; use recursion or `fold` |
| `type` | not a keyword in Una; use `kind` |
| `unsafe` | not a keyword in Una; use a `trusted` module |
| `macro` | not supported (charter §6) |
| `async`, `await` | not supported; see structured concurrency in v0.2 |

## 6. Literals

| Literal | Syntax | Notes |
| --- | --- | --- |
| Integer | `0`, `42`, `1_000_000` | Decimal only. `_` only between digits. No leading zeros except `0` itself. Negative numbers use unary `-`. Values are unbounded (ADR 0004). A leading zero, or a doubled, leading or trailing `_`, is a malformed literal (U0007, ADR 0017). |
| Text | `"..."` | Single line. Escapes: `\"`, `\\`, `\n`, `\t`, `\{`, `\}`, `\u{H..H}` (1 to 6 hex digits, valid scalar value). Unknown escape: U0005. Unterminated: U0004. A `}` outside an interpolation MUST be escaped as `\}` (U0007, ADR 0017). |
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
