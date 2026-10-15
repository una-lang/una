# Una Language Guide: Syntax Reference (v0.1)

**Status.** Informative summary of the normative specification in `docs/spec/`, as of 2026-09-29, including ADR 0036, and checked against the v0.1 reference compiler (`docs/fixes/0001-syntax-guide-review.md`). If this guide and `docs/spec/` disagree, the spec wins.

**Conventions.** Code blocks tagged `una` are Una source. Diagnostic codes use the form `U####` (see §16). Where this guide says "error", the compiler rejects the program with the code shown.

---

## Contents

0. [Una at a glance](#0-una-at-a-glance)
1. [Source files and lexical structure](#1-source-files-and-lexical-structure)
2. [Modules and imports](#2-modules-and-imports)
3. [Kinds and types](#3-kinds-and-types)
4. [Functions and contracts](#4-functions-and-contracts)
5. [Expressions and control flow](#5-expressions-and-control-flow)
6. [Errors](#6-errors)
7. [Capabilities and effects](#7-capabilities-and-effects)
8. [Information flow](#8-information-flow)
9. [Requirements](#9-requirements)
10. [Examples (executable tests)](#10-examples-executable-tests)
11. [Verification](#11-verification)
12. [Standard library](#12-standard-library)
13. [Canonical form](#13-canonical-form)
14. [Project files](#14-project-files)
15. [Command line](#15-command-line)
16. [Diagnostics](#16-diagnostics)
17. [Complete example package](#17-complete-example-package)
18. [Not in v0.1](#18-not-in-v01)

---

## 0. Una at a glance

Una is a verification-first language for machine-generated software. Its design follows a few visible rules:

- **A function is an equation.** The header is the contract; `=` separates it from the body, the implementation.
- **`@` marks authority.** Every interaction with the outside world goes through a capability written `@name`. A line without `@` touches nothing outside the program.
- **Words over symbols:** `and`, `or`, `not`, `implies`, `mod`, `if error fail`.
- **One canonical form:** significant indentation of 2 spaces, one formatter, and non-canonical source is an error.
- **Nothing implicit.** No exceptions, no null, no implicit conversions, no ambient authority.

```una
module events.seating

pub kind Seats = Int in 0..5000

pub kind Availability = SoldOut | Few | Open

pub kind Event
  name: Text
  remaining: Seats

--- Availability bands for an event.
pub availability(e: Event) -> Availability
  ensures e.remaining < 1 implies result is SoldOut
=
  when e.remaining
    in 0..0 -> SoldOut
    in 1..<20 -> Few
    in 20..5000 -> Open
```

The compiler proves the `ensures` clause and that the three ranges cover every possible `Seats` value, before the program runs.

---

## 1. Source files and lexical structure

### 1.1 Files and characters

| Rule | Detail | Error |
| --- | --- | --- |
| Encoding | UTF-8, with no byte order mark | U0001 |
| Line endings | The lexer accepts `\n` and `\r\n` (read as `\n`); a lone `\r` is rejected. Only `\n` is canonical: `una check` reports a file with `\r\n` as one U7001 at the first of them, without an edit, and `una fmt` rewrites the file with `\n` (§13). | U0001, U7001 |
| One module per file | Extension `.una` | |
| ASCII outside literals and comments | Identifiers, keywords and punctuation are ASCII. Other characters (accents, CJK, emoji) may appear only inside text literals and comments. | U0001 |
| Control characters | U+0000 to U+001F (except tab and line feed) and U+007F are forbidden everywhere, including literals and comments | U0001 |
| Bidirectional controls | U+202A to U+202E and U+2066 to U+2069 are forbidden everywhere, as a defense against "Trojan Source" attacks | U0001 |

Characters that are forbidden in raw form can still appear in text through escapes: `"\u{202E}"` is valid.

### 1.2 Indentation

Una uses significant indentation.

- **Spaces only.** A tab in leading whitespace is an error (U0002). Tabs inside literals and comments are allowed.
- **Exactly 2 spaces per level.** Any other depth is an error (U0003), and so is dedenting to a column no enclosing block uses.
- **Blank and comment-only lines** do not affect indentation.
- **Inside `( )` and `[ ]`**, the parser accepts line breaks and ignores the indentation of continued lines. The canonical form has no line width, so it joins continued lines into one: `una fmt` joins them, and `una check` reports the split as U7001. The only exception is a line with a comment inside its brackets, whose line breaks are kept, each continuation line one level deeper (§13).
- **Brackets must be balanced** within a declaration (U0101).

```una
pub total(a: Int, b: Int) -> Int
  requires a >= 0
=
  let sum = add_all(first: a, second: b)
  sum
```

### 1.3 Comments

| Form | Meaning |
| --- | --- |
| `-- text` | Line comment |
| `--- text` | Doc comment. Consecutive lines attach to the next declaration and appear in `una contract` output. |

- There are no block comments.
- A doc comment that is not followed by a declaration (at end of file, inside a body, or before `module`) is treated as a plain comment.

### 1.4 Identifiers and naming conventions

| Token | Pattern | Used for |
| --- | --- | --- |
| lower identifier | `[a-z][a-z0-9_]*` | values, parameters, fields, functions, predicates, handlers, policies, module path segments, capability operations |
| upper identifier | `[A-Z][A-Za-z0-9_]*` | kinds, variants, capabilities, label categories, requirement IDs |
| capability reference | `@` followed by a lower identifier | `@audit`, `@console` |

Naming problems fall into two cases.

**Wrong case class: a parse error (U0101).** Where the grammar expects an upper identifier, a lower one is not a name at all, and the reverse:

| You wrote | Message |
| --- | --- |
| `kind bad_kind = Int` | unexpected `bad_kind`, expected a kind name |
| a variant written in lowercase, as in `kind K = \| open` | unexpected `open`, expected a variant name |
| `requirement events3` | unexpected `events3`, expected a requirement identifier |

**Right case class, wrong convention: U1005.** The name has the right first letter, but breaks the convention of its class:

- lower identifiers use `snake_case`, without `__` and without a trailing `_` (`f__x`, `g_`);
- kinds, variants, capabilities and categories use `UpperCamelCase` (`Bad_Name`);
- requirement IDs use upper snake case ending in `_` and a number: `EVENTS_3`, `CHECKIN_12` (not `Events_3`).

The message names the construct and suggests a fix: "kind `Bad_Name` must be UpperCamelCase, as in `BadName`".

Names cannot start with `_` (U0001, with the hint "names start with a letter").

### 1.5 Keywords

**Reserved.** These are never usable as names:

```text
accepts and attempt capability checked confidential derived do else ensures
error example expect fail fails false fn for from given handler if implies
import in internal is it kind let mod module none not op opaque or pass
performed policy predicate pub public reflect requirement requires restricted
result secret then true trusted unit use uses when where with
```

**Contextual.** These are keywords only inside `requirement` and `policy` blocks, and ordinary names elsewhere (a parameter called `status` or `source` is fine):

```text
active generated human owner proposed purpose regulation source status text to via
```

**Reserved for later, or deliberately absent** (U0103). The message is "`<word>` is <text>", with the text below, as in "`macro` is not supported (charter §6)", and names the plan or the Una alternative:

| Word | Message |
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

### 1.6 Literals

**Integers**

- Decimal only, with `_` allowed between digits: `0`, `42`, `1_000_000`.
- Negative numbers use unary minus: `-5`.
- `Int` is unbounded; there is no overflow in v0.1.
- Malformed integers are U0007, with a mechanical fix when unambiguous:

| Written | Problem | Fix |
| --- | --- | --- |
| `007` | leading zero | `7` |
| `1__000` | doubled underscore | `1_000` |
| `1_` | trailing underscore | `1` |

**Text**

- Double quotes on a single line: `"hello"`. An unterminated literal is U0004.
- Escapes: `\"`, `\\`, `\n`, `\t`, `\{`, `\}`, and `\u{H}` with 1 to 6 hex digits naming a Unicode scalar value.
- An unknown or invalid escape (`"C:\data"`, `\u{D800}`, `\u{110000}`, `\u0041`) is U0005.
- A bare `}` outside an interpolation is U0007: write `\}`.

**Interpolation**

`"{event.name}: {label}"` embeds an expression between braces.

- The embedded expression cannot contain a text literal (U0006). Bind it with `let` first.
- Only `Int`, `Bool` and `Text` can be interpolated (U2001 otherwise). Convert other values with `when`.
- A missing closing `}` is reported as U0004 at the opening quote.
- Interpolation **propagates labels**: `"id {phone}"` is as secret as `phone` (§8).

```una
let label = when status
  is SoldOut -> "sold out"
  is Few -> "few seats left"
  is Open -> "open"
"{event.name}: {label}"
```

**Other literals:** `true`, `false`, and `unit`, the only value of type `Unit`.

### 1.7 Operators and precedence

From highest to lowest precedence:

| Level | Operators | Associativity |
| --- | --- | --- |
| 1 | call `f(..)`, field or method `.name`, capability operation `@c.op(..)` | left |
| 2 | unary `-` | prefix |
| 3 | `*` `/` `mod` | left |
| 4 | `+` `-` | left |
| 5 | `==` `!=` `<` `<=` `>` `>=` `is` `in` | none |
| 6 | `not` | prefix |
| 7 | `and` | left |
| 8 | `or` | left |
| 9 | `implies` | right |

Nuances:

- **No chained comparisons.** `0 < x < 10` is U0101; write `0 < x and x < 10`, or `x in 1..<10`.
- **`not` binds looser than comparisons.** `not a == b` means `not (a == b)`.
- **`implies` groups to the right.** `a implies b implies c` means `a implies (b implies c)`.
- **`and`, `or` and `implies` short-circuit.**
- **Division and `mod` are Euclidean,** so the remainder is never negative: `-7 / 2 == -4` and `-7 mod 2 == 1`. Division by a value the compiler cannot prove nonzero is a proof obligation (§11), never a runtime crash.
- **Ranges:** `a..b` includes `b`; `a..<b` excludes it. Ranges appear only after `in`, in `when` arms and in kind sugar. They are not values.
- **`is` tests a variant or a literal:** `result is SoldOut`, `opt is None`, `flag is true`.

Characters with no meaning in Una are U0001. Some messages carry a hint:

| Character | Hint |
| --- | --- |
| `;` | statements end at line breaks |
| `!` (not part of `!=`) | use `not` |
| `&` | use `and` |
| `'` | text literals use double quotes |
| `_` at the start of a name | names start with a letter |

---

## 2. Modules and imports

```una
module events.checkin

import events.keys (attendee_key)
import events.seating (Availability, Event, availability)
import std.testing (fake_hash)
```

- **Module path and file path must match** (U1008): module `events.checkin` lives in `src/events/checkin.una`.
- **`trusted module`** marks a module allowed to declare policies (§8.5). It must also be listed in `una.toml`.
- **Imports list names explicitly.** Kinds, functions, capabilities, handlers, policies and predicates are all imported the same way.
- **The prelude is imported automatically** (§12).
- **Visibility:** declarations are private unless marked `pub`. Using a private declaration from another module is U1004. Import cycles are U1003.
- **Order:** declarations can appear in any order within a module.

---

## 3. Kinds and types

### 3.1 Built-in types

| Type | Values | Notes |
| --- | --- | --- |
| `Int` | mathematical integers | unbounded |
| `Nat` | `Int where it >= 0` | prelude alias |
| `Bool` | `true`, `false` | |
| `Text` | Unicode strings | `t.len` counts Unicode scalar values |
| `Unit` | `unit` | |
| `List[T]` or `[T]` | `[1, 2, 3]` | immutable; `xs.len` |
| `Option[T]` | `Some(value)`, `None` | |
| `Result[T, E]` | `Ok(value)`, `Err(failure)` | produced by `attempt` (§6.4) |
| `Hashed` | opaque digests | produced by the `Keys` capability; only `==` and `!=` |
| `fn(A, B) -> C` | lambdas and named functions | not comparable |

### 3.2 Declaring kinds

A `kind` declaration takes one of four forms: an alias, a refinement, a sum and a record. Any of them may take type parameters (§3.3).

```una
kind Label = Text                                            -- alias

kind Phone = Text where valid_phone(it)                      -- refinement
kind Seats = Int in 0..5000                                  -- refinement, with range sugar

kind Availability = SoldOut | Few | Open                     -- sum
kind Lookup = Found(registration: Registration) | Missing    -- sum, with a payload

kind Event                                                   -- record
  name: Text
  remaining: Seats

kind NonEmpty[T] = List[T] where it.len > 0                  -- a generic refinement
```

Rules:

- **Sum or alias.** A right-hand side with `|` at bracket depth zero is a sum; otherwise it is an alias or refinement. A single-variant sum needs a leading bar: `kind PhoneError = | Malformed`.
- **Records and sums are nominal.** Two declarations with the same shape are different types.
- **Aliases and refinements are structural over their base.** A `Seats` value can be used wherever an `Int` is expected. Using an `Int` where `Seats` is expected creates a proof obligation: the compiler must prove the value lies in `0..5000`.
- **Refinements stack.** Refining a refined kind combines both predicates.
- **Recursion** is allowed only through `List` or `Option` fields (U2002), so every value is finite.
- **Range sugar:** `T in a..b` means `T where a <= it and it <= b`; `T in a..<b` excludes `b`.

### 3.3 Generics

Kinds and functions can take type parameters. They are unconstrained in v0.1 (no traits), so the only operations on a type parameter are equality and whatever function parameters provide.

```una
pub first_or[T](xs: List[T], default: T) -> T = xs.first().or_else(default)
```

Type arguments are inferred at call sites.

### 3.4 The refinement language (RL)

`where`, `requires` and `ensures` accept only expressions the verifier can reason about:

| Allowed | Examples |
| --- | --- |
| Integer and boolean literals | `0`, `true` |
| `it`, parameters, `result`, and field access on them | `it.remaining`, `e.remaining` |
| Length of `Text` and `List` | `it.len`, `xs.len` |
| Linear arithmetic | `a + b`, `a - b`, `-a`, `3 * a`; `/` and `mod` only by a nonzero literal |
| Comparisons and ranges | `==`, `!=`, `<`, `<=`, `>`, `>=`, `x in 1..10` |
| Variant tests, with `is` only | `result is SoldOut`, `not (result is Open)`; `==` or `!=` against a variant, as in `result != Open`, is U2014 |
| Boolean connectives | `and`, `or`, `not`, `implies` |
| Predicate calls | `valid_phone(it)`, `adult(age)` |

Anything else, such as calls to ordinary functions or `a * b` with two variables, is U2014, reported on the outermost offending sub-expression. Also outside RL: the ordering of `Text`, `is` with a literal, `==` or `!=` against a variant, `unit` and interpolated text. For `x == V` the diagnostic carries an edit to `x is V`, and for `x is 1` an edit to `x == 1`.

### 3.5 Predicates

Predicates are named, pure, total boolean functions usable in RL. There are two forms:

| Form | Body | Seen by the solver | How a value gets the fact |
| --- | --- | --- | --- |
| `predicate reflect` | must be in RL and not recursive | yes, as a definition | proven statically |
| `predicate opaque` | any pure, non-failing expression | no, only its name | a runtime `checked` (§6.5) or a capability operation's `ensures` (§7.1) |

```una
predicate reflect adult(age: Int) -> Bool = age >= 18

--- Format check only.
pub predicate opaque valid_phone(t: Text) -> Bool = t.len == 11 and is_digits(t)
```

Opaque predicates never introduce axioms, so they cannot make the logic inconsistent. They carry validation that the solver cannot reason about (formats, checksums), and the fact travels in the type once established.

### 3.6 Equality, ordering and conversions

- `==` and `!=` are structural, for every type except functions.
- `<`, `<=`, `>`, `>=` work on `Int`, on `Text` (lexicographic), and on sums without payloads (declaration order).
- There are **no implicit conversions**, not even between distinct kinds with the same shape, or between `Int` and `Text`.

### 3.7 Subtyping, in one table

| Flow | Allowed when |
| --- | --- |
| refined value into refined position | the compiler proves the target predicate (obligation) |
| labeled value into labeled position | source label ⊑ target label (§8) |
| function into function position | parameters contravariant, result covariant, and its `uses` and `fails` are included in the target's |

---

## 4. Functions and contracts

### 4.1 Two forms

```una
double(x: Int) -> Int = x * 2                      -- short form

pub availability(e: Event) -> Availability         -- long form
  ensures e.remaining < 1 implies result is SoldOut
  derived from EVENTS_3
=
  when e.remaining
    in 0..0 -> SoldOut
    in 1..<20 -> Few
    in 20..5000 -> Open
```

- **Short form:** no clauses, and a body that is a single-line expression. A `with` clause belongs to a statement, not an expression, so a body with `with` needs the long form (U0101 otherwise, §7.4).
- **Long form:** the signature, then the indented clauses, then `=` alone on a line, then the indented body.

`una fmt` converts between them: a long form with no clauses and a one-line body becomes the short form.

### 4.2 Clauses

| Clause | Meaning | Checked how |
| --- | --- | --- |
| `requires p` | precondition over parameters | proven at every call site; assumed inside the body |
| `ensures q` | postcondition over parameters and `result` | proven on every normal return path; assumed by callers |
| `uses @c: C[ops]` | may perform `ops` through capability `@c` | effects of the body must fit (U3003) |
| `fails E` | may fail with variants of sum kind `E` | failures of the body must be variants of `E` (U3009) |
| `derived from R1, R2` | traceability to requirements | each requirement must exist (U1006) |

Rules:

- **Canonical order:** every `requires`, then every `ensures`, then every `uses` (one capability per clause), then at most one `fails`, then at most one `derived from`. Anything else is U0102.
- **`requires` and `ensures`** must be in RL (U2014).
- **`result`** exists only in `ensures` clauses and example `expect` lines (U2012).
- **`ensures` covers normal returns only;** failures carry no postcondition.
- **Partial correctness:** `ensures` holds if the function returns. Termination is not checked in v0.1.
- **Pure means no `uses`.** A function without `uses` cannot contain any `@` (U3004).
- **No `fails` means it cannot fail.**

```una
pub seats_after(e: Event, count: Int) -> Seats
  requires count >= 1
  requires count <= e.remaining
  ensures result == e.remaining - count
=
  e.remaining - count
```

The compiler proves that `e.remaining - count` fits `Seats` (between 0 and 5000) using both `requires` clauses, and proves the `ensures` clause. Every caller must prove both `requires` clauses at the call site.

### 4.3 Visibility and inference

- `pub` functions must declare every `uses` and their `fails` explicitly. They are the reviewed boundary.
- Private functions may omit `uses` and `fails`; the compiler infers them. `una contract <name>` shows the inferred header.
- Parameter and result types are never inferred.

### 4.4 Calls and labeled arguments

| Rule | Example |
| --- | --- |
| One argument: the label is optional | `double(4)` |
| Two or more: the first may be unlabeled, the rest must be labeled (U2004) | `seats_after(event, count: 2)` |
| Punning: from the second argument on, a bare identifier equal to the parameter name counts as labeled; the first argument always binds the first parameter (ADR 0049) | `record(key, status)` |
| Argument count: too many arguments, or a parameter that receives none, is U2001. The count is checked before the labels. | `f(1, b: 2, b: 3)` for a two-parameter `f`: U2001 "`f` takes 2 arguments, but 3 are given", and no U2006 |
| Unknown label: U2005 | `f(1, c: 2)` |
| Duplicate argument, with the right count: U2006 | `f(a: 1, a: 2)` |
| Arguments are evaluated left to right | |

Constructors follow related rules:

- Records and variants with two or more fields need every field labeled (punning allowed). An unlabeled field is U2004 ("`P` has 2 fields, so every argument needs its field's label"); missing or unknown fields are U2009.
- Any order of labeled fields type-checks, but the canonical form is declaration order (§13): `una check` reports another order as U7001, and `una fmt` rewrites it.
- A single-field constructor may omit the label: `Some(r)`, `Ok(v)`. Extra arguments to a single-field constructor, and parentheses after a variant without fields, are U2009.

```una
let e = Event(name: "Meetup", remaining: 120)
let found = Found(registration: r)
```

### 4.5 Method-call syntax

- `e.f(args)` calls `f(e, args)`. The receiver becomes the first, unlabeled argument.
- If `e` has a field `f` of function type, `e.f(args)` calls that field instead.
- `e.f` without arguments is field access, or `f(e)` when `e` has no field `f` and `f` is a function with one parameter.
- `.len` is built in for `Text` and `List`.

```una
let open_events = schedule.filter(keep: fn(e) -> e.remaining > 0)
let names = open_events.map(each: fn(e) -> e.name)
let count = names.len
```

### 4.6 Lambdas and function types

```una
fn(x) -> x + 1                    -- parameter type taken from the expected type
fn(x: Int) -> x + 1               -- explicit annotation (required without an expected type, U2010)
```

- Function types are written `fn(A, B) -> C`, optionally with `uses` and `fails`: `fn(Text) -> Unit uses @log: Log[write]`.
- A lambda is **pure** unless its expected type declares `uses` or `fails`.
- An effectful or fallible lambda is **second-class**: it may be passed as an argument, but not returned, stored or bound with `let` (U3006).
- **A lambda body is one expression, with no room for a handling clause.** So a lambda cannot call an operation or a function that can fail: `fn(k) -> @audit.record(k)`, where `record` declares `fails`, is U3007. An effectful lambda passed as an argument can call only operations that cannot fail. A lambda may still fail directly, as in `fn(k) -> if k > 0 then k else fail Bad`, when its expected type declares `fails`.
- Put fallible work in a named function with handling clauses, and pass the function by name: `run_it(1, act: parse)`.
- Lambdas capture variables by value.
- The prelude's higher-order functions (`map`, `filter`, `fold`, `first_where`) accept only pure, non-failing lambdas in v0.1: a lambda that uses a capability, passed to `map`, is U3004, since the lambda's expected type has no `uses`.

### 4.7 Recursion and the entry point

- Direct and mutual recursion are allowed. There is no tail-call guarantee in v0.1, and exhausting the stack is runtime diagnostic U6003.
- The entry point is `pub main() -> Unit` in the module set as `entry` in `una.toml`. It can declare `uses` over host capabilities (§7.7) and a `fails` clause.
- If `main` fails, the runtime prints the variant name and exits with code 1. Payload fields are printed only if they are `public`.

---

## 5. Expressions and control flow

### 5.1 Blocks, `let` and statements

A block is a sequence of statements followed by a final expression, which is the block's value.

```una
=
  let seats = clamp_seats(raw)                      -- type inferred
  let event: Event = Event(name: n, remaining: seats)  -- annotated
  availability(event)                               -- final expression is the result
```

- Values are immutable, and there is no reassignment.
- A statement that is not a `let` must have type `Unit` (U2011). To ignore a value, bind it.
- A `let` whose right-hand side is in RL keeps an exact fact that later proofs can use: after `let half = total / 2`, the verifier knows `half == total / 2`. A `let` bound to a function call keeps only what that function's `ensures` promises.

### 5.2 `if`

```una
let size = if n > 100 then "large" else "small"   -- one-line form; else is mandatory

if n > 100                                        -- block form
  "large"
else if n > 10
  "medium"
else
  "small"
```

`if` always has an `else`, because it is an expression.

### 5.3 `when`

```una
when status                            -- variants
  is SoldOut -> "sold out"
  is Few -> "few seats left"
  is Open -> "open"

when lookup                            -- payload binding
  is Found(registration) -> registration.name
  is Missing -> "unknown"

when lookup                            -- binding under another name
  is Found(registration: r) -> r.name
  is Missing -> "unknown"

when e.remaining                       -- ranges
  in 0..0 -> SoldOut
  in 1..<20 -> Few
  in 20..5000 -> Open

when code                              -- literals, with else
  is 200 -> "ok"
  is 404 -> "not found"
  else -> "other"

when status                            -- a block arm
  is SoldOut ->
    let note = "try another date"
    note
  else -> "welcome"
```

Rules:

- **Variant arms** must be exhaustive, or end with `else` (U2007).
- **Range and literal arms** without `else` create an obligation: the compiler must prove the arms cover every value the subject can take. With `Seats = Int in 0..5000`, the three ranges above are proven exhaustive.
- **`else` must be the last arm,** and unreachable arms are rejected (U2008).
- **Single-field variants** bind their field under any name: `is Some(r)`.
- **Results of `attempt`** are matched with `is Ok(value)` and `is Err(failure)` (§6.4).

### 5.4 One effect per line

A capability operation, or a call to a function that uses capabilities or can fail, must be the **whole expression** of a statement, a `let` or a final expression, optionally wrapped in `attempt` (U3013). It cannot be nested inside another expression.

```una
let registration = @registrations.find(key)    -- allowed
  if error fail Unavailable
  if none fail NotRegistered

availability(@registrations.find(key))         -- U3013
```

This fixes evaluation order, gives every failure a handling site and makes every effect visible on its own line. Pure calls nest freely. Example `do` lines are exempt (§10).

---

## 6. Errors

Errors are values. There are no exceptions, no panics and no implicit propagation.

### 6.1 Error kinds and `fails`

Any sum kind can serve as an error kind. A function declares at most one, with `fails E`.

```una
pub kind CheckInError = NotRegistered | Unavailable
pub kind StoreError = Timeout | Corrupted(detail: Text)
```

### 6.2 `fail`

`fail V` or `fail V(field: value)` ends the function with a failure. `V` must be a variant of the function's error kind (U3009).

```una
pub kind BookingError = | InvalidCount

pub book(e: Event, count: Int) -> Seats
  fails BookingError
=
  if count < 1 or count > e.remaining
    fail InvalidCount
  else
    seats_after(e, count)
```

In the `else` branch the verifier knows `count >= 1` and `count <= e.remaining`, so the preconditions of `seats_after` are proven. The second argument uses punning (§4.4).

### 6.3 Handling clauses

A statement whose expression can fail must be followed by an indented `if error` clause (U3007), unless it is wrapped in `attempt`.

| Clause | Applies to | Meaning |
| --- | --- | --- |
| `if error fail V(...)` | any fallible expression | any failure becomes `V`; inside the arguments, `error` is the original failure |
| `if error pass` | callee error kind equals the current function's | propagate the failure unchanged (U3008 if the kinds differ) |
| `if error use e` | any fallible expression | recover with value `e`; `error` is in scope |
| `if none fail V(...)` | an expression of type `Option[T]` | `None` becomes failure `V`; the result has type `T` |
| `if none use e` | an expression of type `Option[T]` | `None` becomes `e`; the result has type `T` |

- Each clause appears at most once, and `if error` comes before `if none` (U0102).
- `error` is valid only inside these clauses (U2013).

```una
let registration = @registrations.find(key)
  if error fail Unavailable
  if none fail NotRegistered

let detail = @store.read(path)
  if error fail Corrupted(detail: "read failed")

let cached = @cache.get(key)
  if error use None
```

### 6.4 `attempt`

`attempt` turns a single fallible call into a `Result`, so you can branch on the outcome instead of failing:

```una
let outcome = attempt @registrations.find(key)
when outcome
  is Ok(value) -> value.is_some()
  is Err(failure) -> false
```

The operand must be one call (U2001), and `attempt` counts as its handling site.

### 6.5 `checked` (escape hatch)

`checked(v, p) else fail V` tests predicate `p` at runtime, with `it` bound to `v`.

```una
pub parse_phone(raw: Text) -> Phone
  fails PhoneError
=
  checked(raw, valid_phone(it)) else fail Malformed
```

- If the test passes, the result is `v` with its type refined by `p`. This is how opaque facts are established.
- If it fails, the function fails with `V`.
- `p` must be RL or an opaque predicate call, so it can run.
- `checked` replaces a proof with a runtime test, so it is **budgeted**: `[escape_budget] checked_per_module` in `una.toml`, default 0 (U7003 when exceeded). Every use is listed in the build report.

### 6.6 What is not a failure

- **Proof obligations never become runtime failures.** Division by zero, refinement violations and broken preconditions are proven absent before the program runs.
- **Running out of stack** is runtime diagnostic U6003, not a Una failure.

---

## 7. Capabilities and effects

### 7.1 Declaring capabilities

A capability describes authority over something outside the program. Its operations are signatures, which are assumptions about the external system.

```una
pub capability Registrations
  op find(key: internal Hashed) -> Option[Registration]
    fails StoreError
    accepts internal

pub capability Audit
  op record(key: internal Hashed, status: Availability) -> Unit
    fails StoreError
    accepts internal
```

| Op clause | Meaning |
| --- | --- |
| `requires p` | precondition on arguments, proven at each call |
| `ensures q` | postcondition: an assumption callers may rely on; handlers are verified against it |
| `fails E` | the operation may fail with `E` |
| `accepts l` | the most sensitive label its arguments may carry (default `public`) |

Canonical op clause order: every `requires`, every `ensures`, then at most one `fails` and at most one `accepts` (U0102).

### 7.2 Using capabilities

```una
pub check_in(phone: secret[PII] Phone) -> Availability
  uses @registrations: Registrations[find]
  uses @audit: Audit[record]
  uses @keys: Keys[hmac]
  fails CheckInError
=
  ...
```

- `uses @x: C[ops]` grants operations `ops` of capability `C` under the name `@x`. Omitting `[ops]` grants all of them.
- `@x.op(args)` works only if `@x` is in scope (U3001) and `op` is granted (U3002).

### 7.3 Passing capabilities: by name, downward only

A callee whose header says `uses @x: C[ops]` receives the caller's `@x` automatically. The caller must hold `@x` with at least those operations (U3001, U3002). Holding more is fine: the callee is limited to what it declares, so passing a capability attenuates it.

### 7.4 `with`: rebinding and handler instances

```una
let status = check_in(phone) with @registrations = @primary_store          -- rebinding
let status = check_in(phone) with @registrations = in_memory(rows: rows)   -- handler instance
```

- **`@x = @y`** makes the callee's `@x` the caller's `@y`. Both must be the same capability.
- **`@x = h(args)`** installs handler `h` as `@x`. The handler's own `uses` become effects of the caller. What happens when the caller lacks them depends on the caller:
  - a `pub` caller that does not declare them gets U3003 at the statement: "handler `h` uses `@inner: Inner[ping]`, but this code declares no `uses`, so its effects would exceed its header";
  - a private caller has them inferred, and the error appears where the capability is really missing. In an example, for instance, it is U3001: "`g` needs `@inner: Inner[ping]`, which no `given` line binds".
- **Scope:** the binding covers only that statement's expression.
- **Statements only.** `with` is part of a statement, so it cannot appear in the short function form: `g() -> Unit = f() with @a = h()` is U0101. Use the long form, with the statement in the body.
- **Several bindings** are separated by commas: `with @a = @b, @c = h()`.

### 7.5 Handlers

A handler implements a capability in Una:

```una
handler in_memory(rows: [Registration]) for Registrations
  op find(key) = rows.first()

handler audit_log() for Audit
  op record(key, status) = unit
```

- **Complete and matching.** A handler implements every operation (U3011), with the right parameter count (U3010). Types come from the capability declaration.
- **Ordinary code.** Operation bodies are type-checked, flow-checked and verified against the operation's `ensures`. They may fail only with the operation's error kind.
- **Parameters** (`rows` above) are captured values.
- **Own authority.** A handler may declare its own `uses` before its operations.
- **Not values.** Handlers are installed only through `with` or `given`.

### 7.6 The second-class rule

Capability references are **not values**. `@x` cannot be:

- bound with `let`;
- passed as an ordinary argument;
- returned;
- stored in a kind;
- compared or interpolated.

Any of these is U3005. A bare `@x` is allowed only on either side of a `with` binding or in `given`. Effectful or fallible lambdas follow the same restrictions, except that they can be passed as arguments (U3006). Such a lambda cannot handle a failure, so it calls only operations that cannot fail (U3007 otherwise, §4.6).

Consequence: authority only flows downward through calls, and every function's reachable authority is visible in headers.

### 7.7 Host capabilities

The runtime implements these; they are part of the prelude.

| Capability | Operation | Fails | Accepts |
| --- | --- | --- | --- |
| `Console` | `print(line: Text) -> Unit` | | `public` |
| `Clock` | `now() -> Int` (ms since Unix epoch) | | `public` |
| `Env` | `get(name: Text) -> Option[internal Text]` | | `public` |
| `Files` | `read(path: Text) -> internal Text` | `FileError` | `public` |
| `Files` | `write(path: Text, content: Text) -> Unit` | `FileError` | `internal` |
| `Keys` | `hmac(key_name: Text, data: Text) -> internal Hashed` | `KeyError` | `public` for `key_name`, any label for `data`; the result carries the label of `data` |

- **Entry.** Host capabilities enter only through `main`'s `uses`, and only if `una run` grants them. An ungranted capability is U3012 before execution starts.
- **Grant syntax:** `console`, `clock`, `env`, `keys:<keyfile>`, `files:<path>:read`, `files:<path>:write`.
- **`Files` confinement.** Paths are confined to the granted prefix; escaping it fails with `Denied`.
- **Tests and handlers.** You can write handlers for host capabilities, for example a fake `Keys` in tests.

---

## 8. Information flow

### 8.1 Labels

Every type can carry a label: `secret[PII] Phone`, `internal Hashed`. A type without a label is `public`.

- **Levels,** from least to most sensitive: `public` ⊏ `internal` ⊏ `confidential` ⊏ `secret` ⊏ `restricted`.
- **Categories** are declared per package in `una.toml` (`[labels] categories = ["PII", "Location"]`). An undeclared category is U4007.
- **Order:** a label flows into another if its level is lower or equal **and** its categories are a subset.

### 8.2 Explicit and implicit flows

- **Results carry their inputs' labels.** The result of an operation is labeled with the join of its operands.
- **Branches raise the program-counter label (pc).** Branching on labeled data raises the pc inside the branches, and values produced there carry it.

```una
let flag = if r.age > 17 then 1 else 0      -- r.age : secret[PII] Int
-- flag : secret[PII] Int
```

- **Returns.** A function's result label must fit its declared return type (U4001).

### 8.3 Failing and calling under a raised pc

- `fail` inside a branch on secret data leaks information through the failure, so it is rejected (U4002). Return a labeled value instead.
- Calling a function that performs effects or can fail is allowed under a raised pc only if every sink it reaches accepts that pc (U4002).

```una
check(pin: secret Int) -> secret Int
  fails PinError
=
  if pin == 1234
    1
  else
    fail Rejected        -- U4002
```

### 8.4 Sinks

- **Operation arguments are sinks.** Every capability operation argument is a sink bounded by the operation's `accepts` label: `pc ⊔ label(args) ⊑ accepts`, otherwise U4003.
- **Console is public.** `Console.print` accepts `public` only, so `@console.print("phone {phone}")` is U4003.
- **Tests are exempt.** `expect` lines in examples observe values for the test harness and are exempt from sink checks.

### 8.5 Declassification: policies in trusted modules

Labels are lowered only through a declared policy, inside a trusted module:

```una
trusted module events.keys

import events.types (Phone)

pub policy attendee_key
  from secret[PII] Phone
  to internal Hashed
  via hmac_phone
  purpose "registration lookup and audit correlation"
  owner privacy

hmac_phone(phone: secret[PII] Phone) -> internal Hashed
  uses @keys: Keys[hmac]
  fails KeyError
=
  @keys.hmac(key_name: "phone_pepper", data: phone)
    if error pass
```

- **Callable.** The policy becomes a function: `attendee_key(phone)`. Its `uses` and `fails` are those of the `via` function.
- **Its failure is released like its value.** The `via` function decides its failure from the `from` value, so the failure carries the `to` labels: with an `internal` `to`, `if error fail` and `if error pass` are U4002, and a branch on `attempt attendee_key(phone)` runs under `internal`. Handle it under that label, with `if error use` or `attempt`, or fail publicly on a public check before the call (ADR 0090).
- **Only entry point.** The `via` function must be private (U4005), so the policy is the only way in. Flow checks are not performed inside it; it belongs to the trusted computing base.
- **Where policies live.** Policies are allowed only in trusted modules (U4004). A trusted module is declared both with `trusted module` and in `una.toml` (U7005 if either is missing).
- **Reported.** Every policy call site appears in the build report with its purpose and owner.

### 8.6 What the guarantee covers

Una guarantees **termination-insensitive noninterference up to declassification**. The channels in the model are values, failures, operation arguments and the occurrence of effects. Timing, termination and resource usage are outside the model, and the build report says so.

---

## 9. Requirements

Requirements record human intent with origin and owner, so contracts can be traced to it.

```una
requirement EVENTS_3
  text "An event with no remaining seats is always shown as sold out"
  source policy "Box Office Handbook 2.1"
  owner box_office
  status active
```

| Field | Values |
| --- | --- |
| `text` | the statement |
| `source` | `human`, `policy "<doc>"`, `regulation "<doc>"`, `generated "<task>"` |
| `owner` | accountable team or person (`snake_case`) |
| `status` | `proposed` or `active` |

- **IDs** are unique per package (U1002).
- **Generated requirements must be `proposed`** (U1007). Agents may propose; a person activates a requirement by changing its source.
- **`derived from`** in a function header links the function to requirements; `una trace EVENTS_3` shows the chain from requirement to contract to proof status.

---

## 10. Examples (executable tests)

```una
example "an unknown attendee fails and is not audited"
  given @registrations = no_registrations()
  given @audit = audit_log()
  given @keys = fake_keys()
  do check_in(parse_phone("11900000001"))
  expect fails NotRegistered
  expect not performed @audit.record
```

| Line | Meaning |
| --- | --- |
| `given @x = h(args)` | install handler `h` as `@x` for this example |
| `do e` | the expression under test |
| `expect e` | `e` is a `Bool` over `result` and must be `true` |
| `expect fails V` | `do` failed with variant `V` |
| `expect performed @x.op` | the effect trace contains at least one `op` on `@x` |
| `expect not performed @x.op` | the trace contains no `op` on `@x` |

- **No host capabilities.** Examples cannot use them (U3014); everything comes from `given`, so examples are deterministic.
- **`do` lines** may nest fallible and effectful calls, and any failure ends the example. They need no handling clauses.
- **Names** are unique per module.
- **Isolation.** Each example runs in a fresh abstract machine. `una test --json` reports the outcome and the full effect trace, with labeled arguments redacted.
- **`std.testing`** (`fake_hash`) is usable only in examples and in handlers installed only by `given`, and only by calling it, never as a function value (U3015).

---

## 11. Verification

### 11.1 What the compiler proves

| Obligation | Created at |
| --- | --- |
| `refinement` | every point where a value flows into a refined type: arguments, returns, fields, annotated `let` |
| `precondition` | every call and capability operation with `requires` |
| `postcondition` | every normal return of a function or handler operation with `ensures` |
| `division` | `/` and `mod` by a non-literal |
| `exhaustive` | `when` with range or literal arms and no `else` |

When proving an obligation, the compiler may assume:
- refinements of values in scope;
- the enclosing function's `requires`;
- the branch conditions on the current path;
- the `ensures` of functions already called;
- operation `ensures`;
- facts from `checked`.

### 11.2 Statuses

| Status | Meaning | Build |
| --- | --- | --- |
| `proven` | the solver proved the goal | ok |
| `refuted` | a counterexample exists | error U5001, with the counterexample |
| `unknown` | the solver could not decide | error U5002: in v0.1, unknown is never accepted |

A U5001 or U5002 comes with the ranked repair candidates that apply at its site, each with an escape cost. None has a mechanical edit, and none is applied automatically. For a `refinement` or `precondition` obligation, with goal `G`, the compiler words them, in this order:

| Repair | When it is proposed | Escape cost |
| --- | --- | --- |
| strengthen the type of the parameter `p` to a refined kind that implies `G` | the value is the parameter `p` of the enclosing function | 0 |
| add `requires G` to the enclosing function; its callers then prove it | the enclosing function is not `pub`, and `G` names only its parameters | 0 |
| add a guard (`if` or a `when` arm) that handles the values where `G` does not hold | always | 0 |
| check the value with `checked(value, G) else fail <Variant>` | always | 1 |

Inside an example there is no parameter and no `requires`, so only the guard and `checked` remain. The other kinds have their own repairs, all with escape cost 0:

- a `postcondition`: add a guard or an arm so that the failing path returns a value for which the `ensures` holds, or adjust the `ensures` clause;
- a `division`: guard the division with `if b != 0`;
- an `exhaustive` obligation: add an `else` arm, or arms for the values the others miss.

A goal that names no value, such as the `0 != 0` of `x / 0`, is false whatever the values: it has no repair and no counterexample, and the code itself has to change.

`checked` is the last resort: it replaces a proof with a runtime test and counts against the escape budget (§6.5).

### 11.3 Tiers

| Tier | Checks | Command |
| --- | --- | --- |
| 0 | syntax, formatting, names, types, effects, capabilities, information flow, budgets | `una check --tier 0` |
| 1 | tier 0 plus every proof obligation (Z3) | `una check` |

Verification is reproducible:
- the solver version is pinned in `verification.lock`;
- queries are deterministic and cached;
- every obligation has a stable id (`ob-…`).

---

## 12. Standard library

**Prelude types:** `Int`, `Nat`, `Bool`, `Text`, `Unit`, `List`, `Option`, `Result`, `Hashed`, plus the host capabilities of §7.7 with `FileError` and `KeyError`.

**Prelude functions.** All are pure. Their contracts are available to proofs.

| Function | Contract |
| --- | --- |
| `abs(x)`, `min(a, b:)`, `max(a, b:)` | exact bounds (`abs(x) >= 0`, ...) |
| `is_digits(t)`, `starts_with(t, prefix:)`, `trim(t)`, `to_int(t) -> Option[Int]` | `trim(t).len <= t.len` |
| `is_empty(xs)`, `first(xs)`, `at(xs, index:)` | `is_empty(xs) == (xs.len == 0)`; `first` and `at` return `None` exactly when out of range |
| `append(xs, item:)` | length grows by 1 |
| `contains(xs, item:)` | |
| `map(xs, each:)` | same length |
| `filter(xs, keep:)` | length ≤ original |
| `first_where(xs, keep:)`, `fold(xs, initial:, step:)`, `sum(xs)` | |
| `is_some(o)`, `or_else(o, default:)`, `is_ok(r)` | |

**`std.testing`:** `fake_hash(seed: Int) -> internal Hashed`. Equal seeds give equal hashes.

---

## 13. Canonical form

Every file has one canonical form. `una fmt` rewrites a file into it, and `una check` reports every difference as U7001. A file with a syntax error has no canonical form: it is neither formatted nor checked for U7001. The rules, from `spec/15`:

**File**

- Line breaks are `\n` only.
- The file ends with exactly one line break.
- No line has trailing whitespace.
- The file neither starts nor ends with a blank line.

**Lines and indentation**

- Two spaces per indentation level.
- Every logical line is one line. The formatter has no maximum line width, so lines continued inside brackets are joined (§1.2).
- The exception is a line with a comment inside its brackets: its line breaks are kept, and each continuation line is indented one level deeper than the line.
- Every token keeps its source spelling: integer literals keep their `_` groups (`1_000`), and text literals their escapes.

**Spacing.** Tokens are separated by one space, except:

- no space after `(`, `[` or `.`, and none before `)`, `]`, `,`, `:` or `.`;
- no space around `..` and `..<`;
- no space after a unary `-`, unless another `-` follows;
- no space between a name and a following `(` or `[`, and none in `fn(` and `checked(`, but one space between an import's module path and its `(`;
- no space inside a text literal outside its interpolations, and none just inside the braces of an interpolation.

**Functions and bodies**

- Header clauses and op clauses are in canonical order (§4.2, §7.1), with one `uses` clause per capability.
- A function with no clauses and a body of one plain line uses the short form (§4.1). The same holds for the body of a predicate or handler operation after its `=`, and of a `when` arm after its `->`.
- A plain line has no `with` clause and no handling clause, and is not a `when` or an `if` with blocks. Otherwise the long form stays, and it also stays when a comment sits between the head and the end of the body.
- Handling clauses sit on their own indented lines, directly below the statement.

**Sums.** A sum with two or more variants has no leading `|`. A sum with one variant keeps it, since the grammar requires it: `kind PhoneError = | Malformed`.

**Constructors**

- When every argument of a record or variant constructor is labeled or punned, the arguments follow the order in which the fields are declared (§4.4).
- This applies to the records and variants the file declares or imports, and to the variants of the sum kinds it imports, when the name comes from exactly one of them and is declared once in its module.
- Arguments are not reordered on a line with a comment inside.

**Imports**

- Imports are sorted bytewise by module path, each keeping the comments that lead it: `events.keys` before `events.seating` before `std.testing`.
- The names inside each import are sorted bytewise too (`Availability, Event, availability`: uppercase letters sort before lowercase), unless a comment sits inside the import.

**Blank lines**

- Top-level items (the module header, each import and each declaration) are separated by exactly one blank line, except consecutive imports, which have none.
- Elsewhere, a run of blank lines becomes one blank line.
- No blank line follows a line that opens a block, and none precedes or follows the `=` line of the long form.

**Comments**

- The formatter never moves a comment to another line, and it removes trailing whitespace from the comment's text.
- A comment at the end of a line of code stays there, after one space.
- A comment on its own line stays in place, at the indentation level nearest to its column (ties go deeper) among the levels of the lines around it.
- A comment on its own line at level 0 between two top-level items belongs to the next item, and the item's blank line goes before it.

**How U7001 is reported**

- `una check` compares each file with its canonical form line by line, and reports one U7001 per changed region, at its first source line.
- Each U7001 carries a mechanical edit that replaces the region with its canonical text, and its message names the difference: trailing whitespace, a missing or extra blank line, a missing final line break, or lines that are not in canonical form.
- A file with `\r\n` line breaks gets a single U7001 at the first of them, without an edit (§1.1). Run `una fmt` to fix it.

---

## 14. Project files

```text
events/
  una.toml
  verification.lock
  src/
    events/
      types.una
      seating.una
      keys.una
      checkin.una
      main.una
```

```toml
[package]
name = "events-example"
version = "0.1.0"
edition = "0.1"

[labels]
categories = ["PII", "Location"]

[trusted]
modules = ["events.keys"]

[escape_budget]
checked_per_module = 1        # default 0

[run]
entry = "events.main"
grants = ["console"]
```

`verification.lock` pins the solver so that proofs are reproducible:

```toml
encoding_version = "una-smt-1"
solver           = "z3"
solver_version   = "5.1.0"
logic            = "QF_UFLIA"
step_budget      = 5000000
random_seed      = 0
```

Unknown keys in `una.toml` are U7002. A missing or invalid lock is U7004.

---

## 15. Command line

```text
una [--json] [--color never|auto|always] <COMMAND>
```

| Command | Purpose |
| --- | --- |
| `una check [path]` | tiers 0 and 1; `--tier 0` skips the solver; `--dump-core <module>` prints the Core IR |
| `una fmt [path]` | rewrite to canonical form; `--check` reports U7001 without writing |
| `una run [path]` | run `main` of the `[run] entry` module; `--grant <spec>`, repeatable; `--max-depth <n>` |
| `una test [path]` | run all examples; `--max-depth <n>` |
| `una obligations [path]` | list every obligation and its status |
| `una contract <module.function>` | full header, declared or inferred |
| `una trace <REQUIREMENT_ID>` | requirement → contracts → proof status |
| `una report [path]` | build report: proofs, authority, policies, escapes, assumptions, trusted code |
| `una explain <code>` | explanation of a diagnostic, with a failing and a fixed example |

`[path]` is a package directory or a single `.una` file, and defaults to the current directory.

**Global flags**

- `--json` prints one JSON document on stdout, and nothing else there.
- `--color never|auto|always` sets colors in human output; `auto`, the default, uses them when the output is a terminal.

**Checking before running**

- `una run` and `una test` first check the package at tier 1, like `una check`, so Z3 at the exact version of `verification.lock` must be on the path.
- If the solver is missing or has another version, they stop with U5003. If the checks report errors, nothing runs.

**Solver flag.** The commands that check at tier 1 (`check`, `run`, `test`, `obligations`, `trace`, `report`) take `--solver-wallclock <seconds>`.

- It is a wall-clock guard against a hanging solver query, 60 seconds by default.
- A query that hits it is `unknown` with reason `wallclock` (U5002), marked non-deterministic in the build report.

**`una run`**

- It runs `pub main() -> Unit` of the `[run] entry` module, or of the module of a single file.
- **Grants.**
  - `--grant` may repeat, and the flags are combined with `[run] grants` of `una.toml` into the final grants.
  - Grant specs: `console`, `clock`, `env`, `keys:<keyfile>`, `files:<path>:read`, `files:<path>:write` and the combined `files:<path>:read,write`.
  - Relative paths in grants resolve against the package root when they come from `una.toml`, and against the current directory when they come from `--grant`.
  - A `uses` of `main` that the final grants do not cover is U3012, before anything runs.
- **Depth.** `--max-depth <n>` sets the maximum call depth, 10,000 by default. Exceeding it aborts with U6003. The interpreter's stack is sized from the depth, so a larger depth also lets a run use more memory.
- **Usage errors (exit code 2):**
  - a package without an entry, or an entry without `pub main() -> Unit`;
  - a `--grant` value that is not a grant spec;
  - an unreadable or invalid key file;
  - a `--max-depth` whose stack cannot be allocated.
- **`--json` output.** `una run` prints a `una.run/1` document whose `status` is one of:
  - `ok`;
  - `failed`: `main` failed, U6001;
  - `aborted`: U6003 or U8001;
  - `rejected`: the checks reported errors, which the document's `diagnostics` holds, and nothing ran.

**`una contract <module.function>`**

- It reads the package in the current directory and checks it at tier 0. The name is the module path and the function's name, as in `events.checkin.check_in`.
- A package with diagnostics, those of `una.toml` and `verification.lock` included, is reported as `una check` reports it, with exit code 1, before the name is looked up, since an inferred header is defined only for a package that checks.
- In a package without diagnostics, an unknown name is a usage error, with exit code 2.

**`una trace <REQUIREMENT_ID>`** also reads the package in the current directory, and checks it at tier 1. It reports the package's diagnostics first, as `una contract` does; in a package without diagnostics, an unknown identifier is a usage error (exit code 2).

**Output streams.** In human mode, diagnostics go to stderr and command results to stdout. With `--json`, the document goes to stdout and stderr stays empty, except for an internal compiler error.

**Exit codes**

| Code | Meaning |
| --- | --- |
| `0` | success |
| `1` | the output holds at least one error diagnostic with a code, or reports an obligation that is not proven (`una obligations`, `una trace`, `una report`) or a failed example (`una test`) |
| `2` | a usage error: a problem of the invocation (its flags, grants, paths or names), printed to stderr without a code, with no diagnostic document on stdout |
| `3` | an internal compiler error (U8001) |

Problems in the project files are diagnostics like any other, in human and `--json` output: an invalid `una.toml` (U7002) and a missing or invalid `verification.lock` (U7004) exit with `1`.

---

## 16. Diagnostics

Every problem has a stable code, precise positions (1-based line and column, columns counted in Unicode scalar values), and JSON output for agents.

| Range | Domain |
| --- | --- |
| U0001 to U0103 | syntax: characters, indentation, literals, tokens, clause order, reserved words |
| U1001 to U1008 | names: resolution, duplicates, cycles, visibility, conventions, requirements, file paths |
| U2001 to U2016 | types: mismatches, labels, exhaustiveness, constructors, RL, predicates |
| U3001 to U3015 | authority: capabilities, effects, failures, handlers, one effect per line |
| U4001 to U4008 | security: flows, implicit flows, sinks, policies, categories, interpolation in a pattern |
| U5001 to U5003 | verification: refuted, unknown, solver missing |
| U6001, U6003 | runtime: `main` failed, call depth exceeded |
| U7001 to U7005 | project: formatting, `una.toml`, escape budget, lock, trusted modules |
| U8001 | internal compiler error (always a bug) |

**Common mistakes**

| You wrote | Code | Fix |
| --- | --- | --- |
| `return x * 2` | U0103 | the last expression is the result |
| `match x` | U0103 | use `when` |
| `0 < x < 10` | U0101 | `x in 1..<10` |
| `x;` | U0001 | remove the `;` |
| `if !done` | U0001 | `if not done` |
| `ensures` before `requires` | U0102 | canonical clause order |
| `seats_after(e, 2)` | U2004 | `seats_after(e, count: 2)` |
| a `when` missing a variant | U2007 | add the arm or `else` |
| `f(x)` as a statement, returning `Int` | U2011 | bind it with `let` |
| `@log.write(...)` without `uses` | U3001 | add `uses @log: Log[write]` |
| `@` inside a function with no `uses` | U3004 | declare the capability |
| `let db = @db` | U3005 | pass it by name through `uses` |
| a fallible call with no `if error` | U3007 | add a handling clause or `attempt` |
| `classify(@store.get(k))` | U3013 | bind the effect on its own line |
| `fail` inside a branch on secret data | U4002 | return a labeled value |
| printing a secret value | U4003 | declassify through a policy, or don't print |
| `ensures` the code does not satisfy | U5001 | read the counterexample |
| a proof the solver cannot decide | U5002 | strengthen types, add `requires` or a guard |

---

## 17. Complete example package

**`src/events/types.una`**

```una
module events.types

--- A phone number of 11 digits, validated once at the boundary.
pub kind Phone = Text where valid_phone(it)

pub kind PhoneError = | Malformed

--- Format check only.
pub predicate opaque valid_phone(t: Text) -> Bool = t.len == 11 and is_digits(t)

--- The only way to obtain a Phone from untrusted text.
pub parse_phone(raw: Text) -> Phone
  fails PhoneError
=
  checked(raw, valid_phone(it)) else fail Malformed

example "malformed input is rejected"
  do parse_phone("123")
  expect fails Malformed
```

**`src/events/seating.una`**

```una
module events.seating

requirement EVENTS_3
  text "An event with no remaining seats is always shown as sold out"
  source policy "Box Office Handbook 2.1"
  owner box_office
  status active

pub kind Seats = Int in 0..5000

pub kind Availability = SoldOut | Few | Open

pub kind Event
  name: Text
  remaining: Seats

pub kind Registration
  name: Text
  event: Event

--- Availability bands from the Box Office Handbook.
pub availability(e: Event) -> Availability
  ensures e.remaining < 1 implies result is SoldOut
  derived from EVENTS_3
=
  when e.remaining
    in 0..0 -> SoldOut
    in 1..<20 -> Few
    in 20..5000 -> Open

--- Clamps a raw count into the seat range.
pub clamp_seats(raw: Int) -> Seats = if raw < 0 then 0 else if raw > 5000 then 5000 else raw

example "an event with no seats is sold out"
  do availability(Event(name: "Launch", remaining: 0))
  expect result is SoldOut

example "an event with many seats is open"
  do availability(Event(name: "Meetup", remaining: 120))
  expect result is Open

example "raw counts are clamped"
  do clamp_seats(9000)
  expect result == 5000
```

**`src/events/keys.una`**: the trusted policy module, shown in §8.5.

**`src/events/checkin.una`**

```una
module events.checkin

import events.keys (attendee_key)
import events.seating (Availability, Event, Registration, availability)
import events.types (Phone, parse_phone)
import std.testing (fake_hash)

pub kind CheckInError = NotRegistered | UnknownEvent | Unavailable(reason: Text)

pub kind StoreError = Timeout | Corrupted

pub capability Registrations
  op event(name: Text) -> Option[Event]
    fails StoreError
  op find(key: internal Hashed) -> Option[Registration]
    fails StoreError
    accepts internal

pub capability Audit
  op record(key: internal Hashed, status: Availability) -> Unit
    fails StoreError
    accepts internal

--- Checks an attendee in to an event by phone, shows its availability, and records it.
--- The phone never leaves this function: only its keyed hash does.
pub check_in(phone: secret[PII] Phone, event: Text) -> internal Result[Availability, CheckInError]
  uses @registrations: Registrations[find, event]
  uses @audit: Audit[record]
  uses @keys: Keys[hmac]
  fails CheckInError
=
  let found = @registrations.event(event)
    if error fail Unavailable(reason: "registrations")
    if none fail UnknownEvent
  -- The key policy's failure is decided by the phone, so it is released like the key, at
  -- `internal` (ADR 0090): from here on, every outcome is in the internal result.
  let key = attempt attendee_key(phone)
  when key
    is Ok(hashed) -> check_in_key(hashed, event: found)
    is Err -> Err(Unavailable(reason: "keys"))

--- Checks in the attendee whose keyed hash is `key`. It runs under the key's label, so
--- each outcome is in its result, not a failure of its own.
check_in_key(key: internal Hashed, event: Event) -> Result[Availability, CheckInError]
  uses @registrations: Registrations[find]
  uses @audit: Audit[record]
=
  let registration = attempt @registrations.find(key)
  when registration
    is Err -> Err(Unavailable(reason: "registrations"))
    is Ok(maybe) ->
      when maybe
        is None -> Err(NotRegistered)
        is Some(r) ->
          if r.event.name == event.name
            let outcome = record_check_in(key, status: availability(event))
            outcome
          else
            Err(NotRegistered)

--- Records the check-in of `key` with `status`, and gives `status` back.
record_check_in(key: internal Hashed, status: Availability) -> Result[Availability, CheckInError]
  uses @audit: Audit[record]
=
  let recorded = attempt @audit.record(key, status)
  when recorded
    is Ok -> Ok(status)
    is Err -> Err(Unavailable(reason: "audit"))

handler one_registration(r: Registration) for Registrations
  op event(name) = Some(r.event)
  op find(key) = Some(r)

handler no_registrations(e: Event) for Registrations
  op event(name) = Some(e)
  op find(key) = None

handler no_events() for Registrations
  op event(name) = None
  op find(key) = None

handler audit_log() for Audit
  op record(key, status) = unit

handler fake_keys() for Keys
  op hmac(key_name, data) = fake_hash(1)

example "a registered attendee is checked in and audited"
  given @registrations = one_registration(Registration(name: "Ana", event: Event(name: "Meetup", remaining: 120)))
  given @audit = audit_log()
  given @keys = fake_keys()
  do check_in(parse_phone("11900000001"), event: "Meetup")
  expect result == Ok(Open)
  expect performed @audit.record

example "an unknown attendee is not checked in and is not audited"
  given @registrations = no_registrations(Event(name: "Meetup", remaining: 120))
  given @audit = audit_log()
  given @keys = fake_keys()
  do check_in(parse_phone("11900000001"), event: "Meetup")
  expect result == Err(NotRegistered)
  expect not performed @audit.record

example "an unknown event fails before the phone is used"
  given @registrations = no_events()
  given @audit = audit_log()
  given @keys = fake_keys()
  do check_in(parse_phone("11900000001"), event: "Gala")
  expect fails UnknownEvent
  expect not performed @keys.hmac
```

**`src/events/main.una`**

```una
module events.main

import events.seating (Event, availability, clamp_seats)

pub main() -> Unit
  uses @console: Console[print]
=
  let event = Event(name: "Meetup", remaining: clamp_seats(120))
  let status = availability(event)
  let label = when status
    is SoldOut -> "sold out"
    is Few -> "few seats left"
    is Open -> "open"
  @console.print("{event.name}: {label}")
```

Running it:

```text
$ una check examples/events
$ una test examples/events
$ una run examples/events --grant console
Meetup: open
```

---

## 18. Not in v0.1

These are planned or deliberately absent. Using a reserved word gives U0103 (§1.5).

| Area | Status |
| --- | --- |
| Native binaries (Cranelift), fixed-width integers | v0.2; v0.1 runs on the reference interpreter |
| Memory conventions (`edit`, `take`), regions, concurrency | v0.2 |
| Traits and constraints on generics, effect polymorphism | v0.2 |
| Units of measure, pipelines (`\|>`) | v0.2 |
| FFI | v0.2 |
| Quantified laws, property tests, semantic diff | v0.3 |
| Cost contracts | v0.3 or later |
| External state semantics (idempotency, delivery, `observe`), durable effect journal | v0.4 |
| Services, data access, infrastructure generation | v0.4 |
| Exceptions, null, inheritance, operator overloading, macros, implicit conversions, global mutable state, ambient authority | never |
