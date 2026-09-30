# Una Language Guide: Syntax Reference (v0.1)

**Status.** Informative summary of the normative specification in `docs/spec/`, as of 2026-09-29. It includes the accepted decisions on reserved keywords, lexical rules and the final name (Una). Decisions the implementation agent took provisionally after that date are not reflected. If this guide and `docs/spec/` disagree, the spec wins.

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
| Line endings | `\n` or `\r\n` (normalized to `\n`); a lone `\r` is rejected | U0001 |
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
- **Inside `( )` and `[ ]`**, line breaks and indentation are ignored, so long argument lists can span lines.
- **Brackets must be balanced** within a declaration (U0101).

```una
pub total(a: Int, b: Int) -> Int
  requires a >= 0
=
  let sum = add_all(
    first: a,
    second: b)
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

Convention checks (U1005):
- lower identifiers use `snake_case`, without `__` and without a trailing `_`;
- kinds, variants, capabilities and categories use `UpperCamelCase`;
- requirement IDs use upper snake case ending in a number: `EVENTS_3`, `CHECKIN_12`.

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

**Reserved for later, or deliberately absent** (U0103). The message names the plan or the Una alternative:

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
| `macro` | not supported |
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

import events.seating (Availability, Event, availability)
import events.keys (attendee_key)
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

A `kind` declaration takes one of five forms:

```una
kind Seats = Int in 0..5000                                  -- range refinement (sugar)
kind Phone = Text where valid_phone(it)                      -- refinement
kind Availability = SoldOut | Few | Open                     -- sum
kind Lookup = Found(registration: Registration) | Missing    -- sum with payload
kind NonEmpty[T] = List[T] where it.len > 0                  -- generic refinement

kind Event                                                   -- record
  name: Text
  remaining: Seats
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
| Comparisons, ranges and variant tests | `==`, `!=`, `<`, `<=`, `>`, `>=`, `x in 1..10`, `result is SoldOut` |
| Boolean connectives | `and`, `or`, `not`, `implies` |
| Predicate calls | `valid_phone(it)`, `adult(age)` |

Anything else, such as calls to ordinary functions or `a * b` with two variables, is U2014, reported on the offending sub-expression.

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

- **Short form:** no clauses, and a body that is a single-line expression.
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
| Punning: a bare identifier equal to the parameter name counts as labeled | `record(key, status)` |
| Unknown label: U2005; duplicate argument: U2006 | |
| Arguments are evaluated left to right | |

Constructors follow related rules:

- Records and variants with two or more fields need every field labeled, in any order; the formatter sorts them into declaration order. Missing or unknown fields are U2009.
- A single-field constructor may omit the label: `Some(r)`, `Ok(v)`.

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
- Lambdas capture variables by value.
- The prelude's higher-order functions (`map`, `filter`, `fold`, `first_where`) accept only pure lambdas in v0.1.

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
- **`@x = h(args)`** installs handler `h` as `@x`. The handler's own `uses` must be available to the caller.
- **Scope:** the binding covers only that statement's expression.
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

Any of these is U3005. A bare `@x` is allowed only on either side of a `with` binding or in `given`. Effectful or fallible lambdas follow the same restrictions, except that they can be passed as arguments (U3006).

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
| `Keys` | `hmac(key_name: Text, data: Text) -> internal Hashed` | `KeyError` | any label |

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
- **`std.testing`** (`fake_hash`) is usable only in handlers and examples (U3015).

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

Every error comes with ranked repairs:
- strengthen a type;
- add a `requires`;
- add a guard;
- as a last resort, `checked`, which carries an escape cost.

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

`una fmt` defines the one valid layout, and `una check` reports deviations as U7001.

- 2-space indentation and no trailing whitespace.
- Header clauses and op clauses in canonical order (§4.2, §7.1).
- The short function form when there are no clauses and the body is one line; the long form otherwise.
- Constructor fields in declaration order.
- One `uses` clause per capability.
- Handling clauses on their own indented lines, directly below the statement.

The v0.1 formatter enforces no maximum line width.

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

| Command | Purpose |
| --- | --- |
| `una check [path]` | tiers 0 and 1; `--tier 0` skips the solver; `--dump-core <module>` prints the Core IR |
| `una fmt [path]` | rewrite to canonical form; `--check` only reports |
| `una run [path] --grant console` | run `main` of the `[run] entry` module |
| `una test [path]` | run all examples |
| `una obligations [path]` | list every obligation and its status |
| `una contract <module.function>` | full header, declared or inferred |
| `una trace <REQUIREMENT_ID>` | requirement → contracts → proof status |
| `una report [path]` | build report: proofs, authority, policies, escapes, assumptions, trusted code |
| `una explain <code>` | explanation of a diagnostic, with a failing and a fixed example |

Output and exit codes:

- **Output streams.** `--json` prints machine output on stdout and leaves stderr empty. In human mode, diagnostics go to stderr and command results to stdout.
- **Exit codes:** `0` success; `1` program errors or failed tests; `2` usage or configuration error; `3` internal compiler error.
- **Interim behavior before tier 1 lands.** Until proof checking is implemented (milestone M8), `una run` and `una test` perform tier 0 checks only and print a note saying so.

---

## 16. Diagnostics

Every problem has a stable code, precise positions (1-based line and column, columns counted in Unicode scalar values), and JSON output for agents.

| Range | Domain |
| --- | --- |
| U0001 to U0103 | syntax: characters, indentation, literals, tokens, clause order, reserved words |
| U1001 to U1008 | names: resolution, duplicates, cycles, visibility, conventions, requirements, file paths |
| U2001 to U2015 | types: mismatches, labels, exhaustiveness, constructors, RL, predicates |
| U3001 to U3015 | authority: capabilities, effects, failures, handlers, one effect per line |
| U4001 to U4007 | security: flows, implicit flows, sinks, policies, categories |
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
| `decide(@store.get(k))` | U3013 | bind the effect on its own line |
| `fail` inside a branch on secret data | U4002 | return a labeled value |
| printing a secret value | U4003 | declassify through a policy, or don't print |
| `ensures` the code does not satisfy | U5001 | read the counterexample |
| a proof the solver cannot decide | U5002 | strengthen types, add `requires` or a guard |

---

## 17. Complete example package

**`src/events/types.una`**

```una
module events.types

--- A Brazilian mobile number: 11 digits, validated once at the boundary.
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

import events.types (Phone, parse_phone)
import events.seating (Availability, Event, Registration, availability)
import events.keys (attendee_key)
import std.testing (fake_hash)

pub kind CheckInError = NotRegistered | Unavailable

pub kind StoreError = Timeout | Corrupted

pub capability Registrations
  op find(key: internal Hashed) -> Option[Registration]
    fails StoreError
    accepts internal

pub capability Audit
  op record(key: internal Hashed, status: Availability) -> Unit
    fails StoreError
    accepts internal

--- Finds a registration by phone, reports the event's availability and audits the check-in.
--- The phone number never leaves this function: only its keyed hash does.
pub check_in(phone: secret[PII] Phone) -> Availability
  uses @registrations: Registrations[find]
  uses @audit: Audit[record]
  uses @keys: Keys[hmac]
  fails CheckInError
=
  let key = attendee_key(phone)
    if error fail Unavailable
  let registration = @registrations.find(key)
    if error fail Unavailable
    if none fail NotRegistered
  let status = availability(registration.event)
  @audit.record(key, status)
    if error fail Unavailable
  status

handler one_registration(r: Registration) for Registrations
  op find(key) = Some(r)

handler no_registrations() for Registrations
  op find(key) = None

handler audit_log() for Audit
  op record(key, status) = unit

handler fake_keys() for Keys
  op hmac(key_name, data) = fake_hash(1)

example "a registered attendee is checked in and audited"
  given @registrations = one_registration(Registration(name: "Ana", event: Event(name: "Meetup", remaining: 120)))
  given @audit = audit_log()
  given @keys = fake_keys()
  do check_in(parse_phone("11900000001"))
  expect result is Open
  expect performed @audit.record

example "an unknown attendee fails and is not audited"
  given @registrations = no_registrations()
  given @audit = audit_log()
  given @keys = fake_keys()
  do check_in(parse_phone("11900000001"))
  expect fails NotRegistered
  expect not performed @audit.record
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
