# Una Language Guide: Syntax Reference (edition 0.2)

**Status.** Informative summary of the normative specification in `docs/spec/`, written for the v0.2 toolchain and packages of edition `0.2`, the default of this guide; the command line (§15) and the diagnostic ranges (§16) are kept current with the toolchain. Where edition `0.1` differs, the text says so. Editions `0.3` and `0.4` are previews, and this page covers only some of what they add: for edition `0.4`, see [Tasks and time](tasks-and-time.md), [Serving HTTP](serving-http.md), [Driven programs](driven-programs.md) and the package manager in [configure/ita](configure/ita/manifest.md). If this guide and `docs/spec/` disagree, the spec wins.

**Conventions.** Code blocks tagged `una` are whole modules. Each one is checked as the only module of a package of edition `0.2`, unless its first lines say `-- edition 0.1`, and `una check` accepts it at tier 1 in canonical form. A block whose first line is `-- fails with U####` yields that code and no other. A block preceded by a file path belongs to a package with the files named around it. `crates/una-cli/tests/guide.rs` checks every block. Diagnostic codes use the form `U####` (see §16). Where this guide says "error", the compiler rejects the program with the code shown.

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
18. [Not in v0.2](#18-not-in-v02)

---

## 0. Una at a glance

Una is a verification-first language for machine-generated software. Its design follows a few visible rules:

- **A function is an equation.** The header is the contract; `=` separates it from the body, the implementation.
- **`@` marks direct capability operations.** Every direct interaction with the outside world goes through a capability written `@name`. A call performs the effects of the function it calls, which the caller's `uses` cover, so a header lists everything its body may do.
- **Words over symbols:** `and`, `or`, `not`, `implies`, `mod`, `if error fail`.
- **One canonical form:** significant indentation of 2 spaces, one formatter, and non-canonical source is an error.
- **Nothing implicit.** No exceptions, no null, no implicit conversions, no ambient authority, no shared mutable state.

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

The compiler proves the `ensures` clause and that the three ranges cover every possible `Seats` value, before the program runs. The same package runs in the interpreter (`una run`) or compiles to a native executable (`una build`).

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
module demo

add_all(first: Int, second: Int) -> Int = first + second

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

Names cannot start with `_` (U0001, with the hint "names start with a letter"). A reserved keyword cannot name a binding: `let result = ...` is U0101, and the message says what the word is reserved for (`result` names a function's result in `ensures`).

### 1.5 Keywords

**Reserved in every edition.** These are never usable as names:

```text
accepts and attempt capability checked confidential derived do else ensures
error example expect fail fails false fn for from given handler if implies
import in internal is it kind let mod module none not op opaque or pass
performed policy predicate pub public reflect requirement requires restricted
result secret then true trusted unit use uses when where with
```

**Keywords of edition `0.2`.** `edit`, `take` and `old` (the memory model, `spec/17`) and `together` (structured concurrency, `spec/19`). In edition `0.1`, `old` is an ordinary name and the others give the U0103 messages of the v0.1 table below.

**Contextual.** These are keywords only inside `requirement` and `policy` blocks, and ordinary names elsewhere (a parameter called `status` or `source` is fine):

```text
active generated human owner proposed purpose regulation source status text to via
```

`Eq`, `Ord` and `Integer` are prelude names of edition `0.2`, the built-in constraints of §3.3, and `I8` to `U64` are built-in kinds (§3.1). A kind, variant or capability with one of these names is U1002, as is one named like any other built-in type: `Int`, `Nat`, `Bool`, `Text`, `Unit`, `Hashed`, `List`, `Option`, `Result` or `Map` (ADR 0079).

**Reserved for later, or deliberately absent** (U0103). The message is `` `<word>` is `` followed by the text below, as in "`macro` is not supported (charter §6)", and names the plan or the Una alternative. In edition `0.2`:

| Word | Message |
| --- | --- |
| `shared`, `iso`, `weak` | reserved for a later edition (aliasing) |
| `parallel`, `actor` | reserved for a later version (concurrency beyond `together`) |
| `simd`, `profile` | reserved for v0.2 (systems profile) |
| `trait`, `impl` | reserved for a later version (user traits) |
| `law`, `all` | reserved for a later edition (laws) |
| `costs` | reserved for a later edition (cost contracts) |
| `traced` | reserved for a later edition (provenance tracing) |
| `service`, `table`, `query`, `observe`, `commit` | reserved for a later edition (platform) |
| `match` | not a keyword in Una; use `when` |
| `return` | not a keyword in Una; the last expression is the result |
| `var` | not a keyword in Una; use `let edit` |
| `while`, `loop` | not a keyword in Una; use `for` or recursion |
| `region` | not a keyword in Una; the backend chooses where values live |
| `type` | not a keyword in Una; use `kind` |
| `unsafe` | not a keyword in Una; use a `trusted` module |
| `macro` | not supported (charter §6) |
| `async`, `await` | not supported; use `together` |

A package of edition `0.1` keeps the table of v0.1 in every version of the compiler: there, `edit`, `take`, `region`, `shared` and `iso` are "reserved for v0.2 (memory model)", `together`, `parallel` and `actor` "reserved for v0.2 (concurrency)", `trait` and `impl` "reserved for v0.2 (constraints on generics)", `var` says "values are immutable, use `let`", `while` and `loop` say "use recursion or `fold`", and `async` and `await` "see structured concurrency in v0.2".

### 1.6 Literals

**Integers**

- Decimal only, with `_` allowed between digits: `0`, `42`, `1_000_000`.
- Negative numbers use unary minus: `-5`. A negated literal counts as one literal, so `-128` fits `I8`.
- `Int` is unbounded and never overflows. The fixed-width types of §3.1 have a range, and a literal takes the fixed-width type its context expects.
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
- A bare `}` outside an interpolation is U0007: write `\}`. So is a `{` that no expression can follow, a quote, a backslash or a `}` after any blanks, as in JSON written in text: write `\{`. Each comes with the edit, and the literal goes on, so one brace is one diagnostic.

**Interpolation**

`"{event.name}: {label}"` embeds an expression between braces.

- The embedded expression cannot contain a text literal (U0006). Bind it with `let` first.
- Any value that holds no function and no `Hashed` can be interpolated (U2001 otherwise). It is written in its Una form, as `una test` shows it: `Event(name: "Gala", remaining: 3)`, `[1, 2]`, `Some(3)`. A `Text` inside it is quoted. A value of a type parameter `T` needs `[T: Eq]` (U2026). In edition 0.1, only integers, `Bool` and `Text` can be interpolated (ADR 0068).
- A missing closing `}` is reported as U0004 at the opening quote.
- Interpolation **propagates labels**: `"id {phone}"` is as secret as `phone` (§8).

```una
module demo

pub kind Status = SoldOut | Few | Open

pub kind Event
  name: Text
  remaining: Int

--- A record is interpolated as it is, or field by field for a shorter text.
pub describe(event: Event) -> Text = "{event.name} ({event.remaining} seats)"

--- The whole record: `Event(name: "Gala", remaining: 3)`.
pub raw(event: Event) -> Text = "{event}"

pub banner(event: Event, status: Status) -> Text
=
  let label = when status
    is SoldOut -> "sold out"
    is Few -> "few seats left"
    is Open -> "open"
  "{describe(event)}: {label}"
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
- **Integer operators take operands of one type.** `a + b` with `a: U8` and `b: U16` is U2001: convert one of them (§3.1).
- **Ranges:** `a..b` includes `b`; `a..<b` excludes it. Ranges appear after `in`, in `when` arms, in kind sugar, and as the source of `for` and `together for` (§5.5, §5.7). They are not values.
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

A package with two modules, one importing from the other:

`src/shop/prices.una`

```una
module shop.prices

pub kind Price = Int in 0..1_000_000

pub kind Item
  name: Text
  price: Price

--- Private: other modules cannot import it (U1004).
cents(item: Item) -> Int = item.price

pub total(items: List[Item]) -> Int = items.map(each: fn(i) -> cents(i)).sum()
```

`src/shop/cart.una`

```una
module shop.cart

import shop.prices (Item, total)

pub kind Cart
  items: List[Item]

pub cart_total(cart: Cart) -> Int = total(cart.items)
```

- **Module path and file path must match** (U1008): module `shop.cart` lives in `src/shop/cart.una`.
- **`trusted module`** marks a module allowed to declare policies (§8.5). It must also be listed in `una.toml`.
- **Imports list names explicitly.** Kinds, functions, capabilities, handlers, policies and predicates are all imported the same way. `std.testing` is the only standard module (§12).
- **The prelude is imported automatically** (§12).
- **Visibility:** declarations are private unless marked `pub`. Using a private declaration from another module is U1004. Import cycles are U1003.
- **Order:** declarations can appear in any order within a module.

---

## 3. Kinds and types

### 3.1 Built-in types

| Type | Values | Notes |
| --- | --- | --- |
| `Int` | mathematical integers | unbounded |
| `I8`, `I16`, `I32`, `I64` | `-2^(n-1)` to `2^(n-1) - 1` | edition `0.2`; overflow is a proof obligation |
| `U8`, `U16`, `U32`, `U64` | `0` to `2^n - 1` | edition `0.2`; overflow is a proof obligation |
| `Nat` | `Int where it >= 0` | prelude alias |
| `Bool` | `true`, `false` | |
| `Text` | Unicode strings | `t.len` counts Unicode scalar values; in edition `0.2`, `split`, `join`, `find`, `contains_text`, `ends_with`, `replace`, `slice`, `chars`, `bytes`, `to_upper`, `to_lower`, `trim_start`, `trim_end` and `parse_int` (`spec/22`) |
| `Unit` | `unit` | |
| `List[T]` or `[T]` | `[1, 2, 3]` | values; `xs.len`; changed through `set` and `append` (§5.6) |
| `Map[K, V]` | `empty_map().with(key: "a", value: 1)` | edition `0.2`; keys satisfy `Ord`, in ascending order; `m.len`, `has`, `get`, `keys`, `values`, `entries`, `with`, `without`, and `edit m.set` and `edit m.remove` (`spec/21`) |
| `Option[T]` | `Some(value)`, `None` | |
| `Result[T, E]` | `Ok(value)`, `Err(failure)` | produced by `attempt` (§6.4) |
| `Hashed` | opaque digests | produced by the `Keys` capability; only `==` and `!=` |
| `fn(A, B) -> C` | lambdas and named functions | not comparable |

**Fixed-width integers** (edition `0.2`, `spec/18`). They exist where an exact width matters, such as bytes, protocols and hashing. `Int` stays where it was: list lengths, indices and `Text.len` are `Int`.

- **Distinct types.** `I8` to `U64` are distinct from `Int` and from one another, with no implicit conversion. An operator takes operands of one type (U2001).
- **Literals** take the fixed-width type their context expects: an annotated `let`, a parameter, a field, a return, the other operand of an operator, or the field of a constructor checked against a known type, as `Some(200)` against `Option[U8]`. A literal that does not fit is U2022.
- **Overflow** of `+`, `-`, `*`, `/`, `mod` and unary `-` is a proof obligation of kind `overflow` (§11), never a silent wrap. Clauses (`where`, `requires`, `ensures`) read every integer type as its mathematical value, so `requires a + b <= 255` computes exactly.
- **Conversions** are built-in methods: `x.to_i8()` to `x.to_u64()` and `x.to_int()`. Widening, where every value fits, needs no proof. Narrowing carries an `overflow` obligation that the value fits.
- **Wrapping and saturating arithmetic** is written out: `a.wrapping_add(by: b)`, `a.saturating_sub(by: b)` and their siblings, never by default. Bit operations are `a.bit_and(mask: m)`, `bit_or`, `bit_xor`, `bit_not()`, `a.shift_right(by: n)` and `a.wrapping_shift_left(by: n)`.

```una
module demo

pub kind Port = U16 where it > 0

--- Literals take the fixed-width type their context expects.
pub defaults() -> List[U8] = [0, 127, 255]

pub maybe_byte(flag: Bool) -> Option[U8] = if flag then Some(200) else None

--- Overflow is an obligation: `a + b` must fit in `U8`. Clauses compute exactly, so `a + b <= 255` is the mathematical sum.
pub add_small(a: U8, b: U8) -> U8
  requires a + b <= 255
=
  a + b

--- Wrapping arithmetic is written out, never implied.
pub checksum(a: U8, b: U8) -> U8 = a.wrapping_add(by: b)

--- Widening needs no proof; narrowing does.
pub widen(x: U8) -> I32 = x.to_i32()

pub narrow(x: I32) -> U8
  requires x >= 0 and x <= 255
=
  x.to_u8()
```

```una
-- fails with U5001
module demo

pub add(a: U8, b: U8) -> U8 = a + b
```

```una
-- fails with U2022
module demo

pub too_big() -> U8 = 300
```

```una
-- fails with U2001
module demo

pub mixed(a: U8, b: U16) -> U16 = a + b
```

The overflow diagnostic proposes, in order: a guard computed on `Int`, as `if a.to_int() + b.to_int() in 0..255`; a wider type through explicit conversions; or a wrapping or saturating method, when that is the arithmetic meant.

### 3.2 Declaring kinds

A `kind` declaration takes one of four forms: an alias, a refinement, a sum and a record. Any of them may take type parameters (§3.3).

```una
module demo

pub kind Label = Text

pub kind Phone = Text where valid_phone(it)

pub kind Seats = Int in 0..5000

pub kind Availability = SoldOut | Few | Open

pub kind Registration
  name: Text
  phone: Phone

pub kind Lookup = Found(registration: Registration) | Missing

pub kind Event
  name: Text
  remaining: Seats

pub kind NonEmpty[T] = List[T] where it.len > 0

pub kind PhoneError = | Malformed

pub predicate opaque valid_phone(t: Text) -> Bool = t.len == 11 and is_digits(t)
```

Rules:

- **Sum or alias.** A right-hand side with `|` at bracket depth zero is a sum; otherwise it is an alias or refinement. A single-variant sum needs a leading bar: `kind PhoneError = | Malformed`.
- **Records and sums are nominal.** Two declarations with the same shape are different types.
- **Aliases and refinements are structural over their base.** A `Seats` value can be used wherever an `Int` is expected. Using an `Int` where `Seats` is expected creates a proof obligation: the compiler must prove the value lies in `0..5000`.
- **Refinements stack.** Refining a refined kind combines both predicates. Fixed-width types can be refined too: `kind Port = U16 where it > 0`.
- **Range sugar:** `T in a..b` means `T where a <= it and it <= b`; `T in a..<b` excludes `b`.

**Recursive kinds** (edition `0.2`, ADR 0060). A kind may contain itself through any field, as long as it has a finite value: some variant (or, for a record, every field) needs no value of a kind without one. Inside a recursion, the kinds of the recursion take only type parameters as arguments.

```una
module demo

--- `Lit` is a finite value, so `Expr` may contain itself through any field.
pub kind Expr = Lit(value: Int) | Add(left: Expr, right: Expr) | Neg(inner: Expr)

pub eval(e: Expr) -> Int
=
  when e
    is Lit(value) -> value
    is Add(left, right) -> eval(left) + eval(right)
    is Neg(inner) -> -eval(inner)

--- A generic recursive kind passes on only its own type parameter.
pub kind Tree[T] = Leaf | Node(left: Tree[T], item: T, right: Tree[T])

pub size[T](t: Tree[T]) -> Int
=
  when t
    is Leaf -> 0
    is Node(left, right) -> size(left) + 1 + size(right)
```

A kind without a finite value is U2002, with a repair:

```una
-- fails with U2002
module demo

pub kind Stream = | Cons(head: Int, tail: Stream)
```

In edition `0.1`, a kind may contain itself only through `List` or `Option` fields:

```una
-- fails with U2002
-- edition 0.1
module demo

pub kind Expr = Lit(value: Int) | Add(left: Expr, right: Expr)
```

### 3.3 Generics and constraints

Kinds and functions can take type parameters, and type arguments are inferred at call sites. Without a constraint, the only operations on a type parameter are those that function parameters provide.

In edition `0.2` (ADR 0066, `spec/20`), a function's type parameter can carry one built-in **constraint**:

| Constraint | Allows on values of `T` | Satisfied by |
| --- | --- | --- |
| `Eq` | `==`, `!=` | every type whose values hold no functions |
| `Ord` | what `Eq` allows, and `<`, `<=`, `>`, `>=` | `Int`, `I8` to `U64`, `Text`, and sums whose variants have no payload |
| `Integer` | what `Ord` allows, and `x.to_int()` | `Int`, `I8` to `U64` |

- **Written on functions only.** `Kind[T: Eq]` is U0101. Each type parameter takes at most one constraint, and an unknown name is U2027.
- **Operators need the constraint** (U2026, with a repair that writes it). This includes `==` on an unconstrained type parameter. Public functions write their constraints; private ones have the least constraint their body needs inferred.
- **Type arguments must satisfy it** (U2025 at the call).
- **No arithmetic on `T`,** constrained or not: convert with `to_int()` and compute on `Int`.

```una
module demo

--- Unconstrained: only what the function parameters provide.
pub first_or[T](xs: List[T], default: T) -> T = xs.first().or_else(default)

--- `Ord` allows comparisons; `max` is generic over `Ord` in edition 0.2.
pub largest[T: Ord](xs: List[T], floor: T) -> T = xs.fold(initial: floor, step: fn(best, x) -> max(best, b: x))

--- `Eq` allows `==` and `!=`.
pub same[T: Eq](a: T, b: T) -> Bool = a == b

pub has[T: Eq](xs: List[T], item: T) -> Bool = xs.contains(item)

--- `Integer` adds `to_int()`; arithmetic happens on `Int`.
pub total[T: Integer](xs: List[T]) -> Int = xs.map(each: fn(x) -> x.to_int()).sum()

pub ranked(names: List[Text]) -> List[Text] = sort(names)
```

```una
-- fails with U2026
module demo

pub same[T](a: T, b: T) -> Bool = a == b
```

```una
-- fails with U2025
module demo

pub kind Shape = Circle(radius: Int) | Square(side: Int)

pub biggest(shapes: List[Shape]) -> List[Shape] = sort(shapes)
```

In edition `0.1`, type parameters are unconstrained, and equality on one is allowed with an inferred bound: the function cannot be instantiated with a type whose values hold functions (U2001).

### 3.4 The refinement language (RL)

`where`, `requires` and `ensures` accept only expressions the verifier can reason about:

| Allowed | Examples |
| --- | --- |
| Integer and boolean literals | `0`, `true` |
| `it`, parameters, `result`, `old(x)` of an `edit` parameter, and field access on them | `it.remaining`, `e.remaining`, `old(t).seen` |
| Length of `Text` and `List` | `it.len`, `xs.len` |
| Linear arithmetic | `a + b`, `a - b`, `-a`, `3 * a`; `/` and `mod` only by a nonzero literal |
| Comparisons and ranges | `==`, `!=`, `<`, `<=`, `>`, `>=`, `x in 1..10` |
| Variant tests, with `is` only | `result is SoldOut`, `not (result is Open)`; `==` or `!=` against a variant, as in `result != Open`, is U2014 |
| Boolean connectives | `and`, `or`, `not`, `implies` |
| Predicate calls | `valid_phone(it)`, `adult(age)` |

Anything else, such as calls to ordinary functions, methods such as `x.to_int()`, or `a * b` with two variables, is U2014, reported on the outermost offending sub-expression. Also outside RL: the ordering of `Text`, `is` with a literal, `==` or `!=` against a variant, `unit` and interpolated text. For `x == V` the diagnostic carries an edit to `x is V`, and for `x is 1` an edit to `x == 1`.

In clauses, every integer type reads as its mathematical `Int`: operators accept any mix of integer types and compute exactly, with no overflow obligation and no U2022. So `ensures result <= 300` on a `U8` result is valid.

### 3.5 Predicates

Predicates are named, pure, total boolean functions usable in RL. There are two forms:

| Form | Body | Seen by the solver | How a value gets the fact |
| --- | --- | --- | --- |
| `predicate reflect` | must be in RL and not recursive | yes, as a definition | proven statically |
| `predicate opaque` | any pure, non-failing expression | no, only its name | a runtime `checked` (§6.5) or a capability operation's `ensures` (§7.1) |

```una
module demo

predicate reflect adult(age: Int) -> Bool = age >= 18

--- Format check only.
pub predicate opaque valid_phone(t: Text) -> Bool = t.len == 11 and is_digits(t)

--- A reflect body computes exactly: no overflow obligation, even on `U32`.
predicate reflect large(x: U32) -> Bool = x * 4294967295 > 4294967295

pub kind Adult = Int where adult(it)
```

- **Opaque predicates** never introduce axioms, so they cannot make the logic inconsistent. They carry validation that the solver cannot reason about (formats, checksums), and the fact travels in the type once established. Their bodies are code, with overflow obligations on fixed-width arithmetic.
- **`reflect` bodies compute exactly.** The body of a `reflect` predicate is typed as code that runs, so its integer types do not mix (U2001), but its arithmetic is exact, as the verifier reads it and as every engine computes it: no `overflow` obligation and no U2022. So `large(4294967295)` above holds.

### 3.6 Equality, ordering and conversions

- `==` and `!=` are structural, for every type except functions.
- `<`, `<=`, `>`, `>=` work on `Int`, on the fixed-width types, on `Text` (lexicographic), and on sums without payloads (declaration order).
- There are **no implicit conversions**, not even between distinct kinds with the same shape, between `Int` and a fixed-width type, or between `Int` and `Text`. Integer conversions are the methods of §3.1.

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
module demo

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

--- Short form.
double(x: Int) -> Int = x * 2

--- Long form.
pub availability(e: Event) -> Availability
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
| `ensures q` | postcondition over parameters, `result` and `old(x)` | proven on every normal return path; assumed by callers |
| `uses @c: C[ops]` | may perform `ops` through capability `@c` | effects of the body must fit (U3003) |
| `uses of p` | may perform the effects of the function argument `p` (edition `0.2`) | U3017 to U3019 for a malformed clause |
| `fails E` | may fail with variants of sum kind `E` | failures of the body must be variants of `E` (U3009) |
| `fails of p` | may fail as the function argument `p` does (edition `0.2`) | as `uses of p` |
| `derived from R1, R2` | traceability to requirements | each requirement must exist (U1006) |

Rules:

- **Canonical order:** every `requires`, then every `ensures`, then every `uses` and `uses of` (one capability or parameter per clause), then at most one `fails` and one `fails of`, then at most one `derived from`. Anything else is U0102.
- **`requires` and `ensures`** must be in RL (U2014).
- **`result`** exists only in `ensures` clauses and example `expect` lines (U2012). **`old(x)`** exists only in `ensures`, on an `edit` parameter (§5.6).
- **`ensures` covers normal returns only;** failures carry no postcondition.
- **Partial correctness:** `ensures` holds if the function returns. Termination is not checked.
- **Pure means no `uses`.** A function without `uses` cannot contain any `@` (U3004).
- **No `fails` means it cannot fail.**

```una
module demo

pub kind Seats = Int in 0..5000

pub kind Event
  name: Text
  remaining: Seats

pub seats_after(e: Event, count: Int) -> Seats
  requires count >= 1
  requires count <= e.remaining
  ensures result == e.remaining - count
=
  e.remaining - count
```

The compiler proves that `e.remaining - count` fits `Seats` (between 0 and 5000) using both `requires` clauses, and proves the `ensures` clause. Every caller must prove both `requires` clauses at the call site.

**Effect polymorphism** (edition `0.2`, ADR 0062). `uses of p` and `fails of p` name a parameter `p` of function type, written without its own `uses` or `fails`. At each call, the callee's effects and failures are its own plus those of the argument passed for `p`. Inside the body, `p` is only called, or passed on to a parameter that the callee's `of` clause names. A callee with its own `fails E` accepts for `p` only an argument that fails with `E`, so each function keeps one error kind.

```una
module demo

pub kind ParseError = | NotANumber

--- `each` may have effects and may fail; `apply_all` has them too, at each call.
pub apply_all(xs: List[Text], each: fn(Text) -> Int) -> List[Int]
  uses of each
  fails of each
=
  xs.map(each: each)
    if error pass

pub parse(t: Text) -> Int
  fails ParseError
=
  t.to_int()
    if none fail NotANumber

pub parse_all(xs: List[Text]) -> List[Int]
  fails ParseError
=
  apply_all(xs, each: parse)
    if error pass

pub shout(names: List[Text]) -> Unit
  uses @console: Console[print]
=
  let printed = names.map(each: fn(n) -> @console.print(n))
  unit
```

### 4.3 Visibility and inference

- `pub` functions must declare every `uses`, their `fails` and the constraints of their type parameters explicitly. They are the reviewed boundary.
- Private functions may omit `uses`, `fails`, `uses of`, `fails of` and constraints; the compiler infers them. `una contract <name>` shows the inferred header.
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
- A variant with fields is not a value on its own: `e == Found` is U2009, with the repair `e is Found`.

```una
module demo

pub kind Registration
  name: Text
  seat: Int

pub kind Lookup = Found(registration: Registration) | Missing

pub kind Event
  name: Text
  remaining: Int

pub sample(r: Registration) -> Lookup
=
  let e = Event(name: "Meetup", remaining: 120)
  let found = Found(registration: r)
  let maybe = Some(r)
  found
```

### 4.5 Method-call syntax

- `e.f(args)` calls `f(e, args)`. The receiver becomes the first, unlabeled argument.
- If `e` has a field `f` of function type, `e.f(args)` calls that field instead.
- `e.f` without arguments is field access, or `f(e)` when `e` has no field `f` and `f` is a function with one parameter.
- `.len` is built in for `Text` and `List`, and the conversions and arithmetic methods of §3.1 for integers.
- `edit xs.set(index: 0, item: v)` passes the receiver as an `edit` argument (§5.6).

```una
module demo

pub kind Event
  name: Text
  remaining: Int

pub open_names(schedule: List[Event]) -> Int
=
  let open_events = schedule.filter(keep: fn(e) -> e.remaining > 0)
  let names = open_events.map(each: fn(e) -> e.name)
  names.len
```

### 4.6 Lambdas and function types

```una
module demo

pub increments(xs: List[Int]) -> List[Int] = xs.map(each: fn(x) -> x + 1)

pub next() -> fn(Int) -> Int = fn(x: Int) -> x + 1
```

- A lambda's parameter types come from the expected type, as in `map`'s `each`; without one, annotate them (`fn(x: Int) -> x + 1`), or it is U2010.
- Function types are written `fn(A, B) -> C`, optionally with `uses` and `fails`: `fn(Text) -> Unit uses @log: Log[write]`.
- A lambda is **pure** unless its expected type declares `uses` or `fails`, or is a parameter named by a `uses of` or `fails of` clause.
- An effectful or fallible lambda is **second-class**: it may be passed as an argument, but not returned, stored or bound with `let` (U3006).
- **A lambda body is one expression, with no room for a handling clause.** So a lambda cannot call an operation or a function that can fail: `fn(k) -> @audit.record(k)`, where `record` declares `fails`, is U3007. A lambda may still fail directly, as in `fn(k) -> if k > 0 then k else fail Bad`, when its expected type declares `fails`. Put fallible work in a named function with handling clauses, and pass the function by name, as `apply_all(xs, each: parse)` in §4.2.
- Lambdas capture variables by value. A lambda captures the value an editable binding has when the lambda is created, and cannot assign it (U2019).
- **The prelude's higher-order functions** (`map`, `filter`, `fold`, `first_where`) declare `uses of` and `fails of` their function parameter in edition `0.2`, so `names.map(each: fn(n) -> @console.print(n))` works (§4.2). In edition `0.1` they accept only pure, non-failing lambdas (U3004 otherwise). Prefer a `for` loop for anything that handles failures, updates bindings or takes several steps.
- Inside a function, call a function parameter only where no secret condition decides the call (U4002).
- A function with an `edit` or `take` parameter cannot be used as a value (U2021).

### 4.7 Recursion and the entry point

- Direct and mutual recursion are allowed. There is no tail-call guarantee, and exhausting the call depth or the stack is runtime diagnostic U6003. Loops (§5.5) do not count toward the call depth.
- The entry point is `pub main() -> Unit` in the module set as `entry` in `una.toml`. It can declare `uses` over host capabilities (§7.7) and a `fails` clause.
- If `main` fails, the runtime prints the variant name and exits with code 1. Payload fields are printed only if they are `public`.

---

## 5. Expressions and control flow

### 5.1 Blocks, `let` and statements

A block is a sequence of statements followed by a final expression, which is the block's value.

```una
module demo

pub kind Seats = Int in 0..5000

pub kind Availability = SoldOut | Few | Open

pub kind Event
  name: Text
  remaining: Seats

clamp_seats(raw: Int) -> Seats = if raw < 0 then 0 else if raw > 5000 then 5000 else raw

availability(e: Event) -> Availability = if e.remaining == 0 then SoldOut else if e.remaining < 20 then Few else Open

pub status(n: Text, raw: Int) -> Availability
=
  let seats = clamp_seats(raw)
  let event: Event = Event(name: n, remaining: seats)
  availability(event)
```

- A plain `let` is immutable. An editable binding is declared `let edit` (§5.6).
- A statement that is not a `let`, an assignment, a `for` loop, a `together` block or an `if` without `else` must have type `Unit` (U2011). To ignore a value, bind it.
- A `let` whose right-hand side is in RL keeps an exact fact that later proofs can use: after `let half = total / 2`, the verifier knows `half == total / 2`. A `let` bound to a function call keeps only what that function's `ensures` promises.

### 5.2 `if`

```una
module demo

pub size(n: Int) -> Text = if n > 100 then "large" else "small"

pub band(n: Int) -> Text
=
  if n > 100
    "large"
  else if n > 10
    "medium"
  else
    "small"
```

- `if` used as an expression always has an `else`.
- In edition `0.2`, a block-form `if` without `else` is a statement of type `Unit`, used to update editable bindings (§5.5).

### 5.3 `when`

```una
module demo

pub kind Seats = Int in 0..5000

pub kind Availability = SoldOut | Few | Open

pub kind Registration
  name: Text

pub kind Lookup = Found(registration: Registration) | Missing

--- Variants.
pub label(status: Availability) -> Text
=
  when status
    is SoldOut -> "sold out"
    is Few -> "few seats left"
    is Open -> "open"

--- A payload bound by its field name, or under another name.
pub name_of(lookup: Lookup) -> Text
=
  when lookup
    is Found(registration) -> registration.name
    is Missing -> "unknown"

pub name_again(lookup: Lookup) -> Text
=
  when lookup
    is Found(registration: r) -> r.name
    is Missing -> "unknown"

--- Ranges, proven to cover every `Seats` value.
pub band(remaining: Seats) -> Availability
=
  when remaining
    in 0..0 -> SoldOut
    in 1..<20 -> Few
    in 20..5000 -> Open

--- Literals, with `else`.
pub reason(code: Int) -> Text
=
  when code
    is 200 -> "ok"
    is 404 -> "not found"
    else -> "other"

--- A block arm.
pub greeting(status: Availability) -> Text
=
  when status
    is SoldOut ->
      let note = "try another date"
      note
    else -> "welcome"
```

Rules:

- **Variant arms** must be exhaustive, or end with `else` (U2007).
- **Range and literal arms** without `else` create an obligation: the compiler must prove the arms cover every value the subject can take. With `Seats = Int in 0..5000`, the three ranges above are proven exhaustive. Arms over a fixed-width value take its type, and a bound that does not fit is U2022.
- **`else` must be the last arm,** and unreachable arms are rejected (U2008).
- **Single-field variants** bind their field under any name: `is Some(r)`.
- **Results of `attempt`** are matched with `is Ok(value)` and `is Err(failure)` (§6.4).

### 5.4 One effect per line

A capability operation, a call to a function that uses capabilities or can fail, or a call with an `edit` or `take` argument, must be the **whole expression** of a statement, a `let` or a final expression, optionally wrapped in `attempt` (U3013). It cannot be nested inside another expression.

```una
module demo

pub kind Availability = SoldOut | Few | Open

pub kind Registration
  name: Text
  status: Availability

pub kind StoreError = | Timeout

pub kind CheckInError = NotRegistered | Unavailable

pub capability Registrations
  op find(key: Text) -> Option[Registration]
    fails StoreError

pub status_of(key: Text) -> Availability
  uses @registrations: Registrations[find]
  fails CheckInError
=
  let registration = @registrations.find(key)
    if error fail Unavailable
    if none fail NotRegistered
  registration.status
```

```una
-- fails with U3013
module demo

pub capability Store
  op get(key: Text) -> Int

double(x: Int) -> Int = x * 2

pub doubled(key: Text) -> Int
  uses @store: Store[get]
=
  double(@store.get(key))
```

This fixes evaluation order, gives every failure a handling site and makes every effect visible on its own line. Pure calls nest freely. Example `do` lines are exempt (§10).

### 5.5 `for` loops (edition `0.2`)

`for x in xs` visits the elements of a list in order, and `for i in a..<b` or `for i in a..b` the integers of a range in increasing order (`spec/17 §1`, `§4`).

```una
module demo

--- A `for` loop over a list, updating a `let edit` binding.
pub total(xs: List[Int]) -> Int
=
  let edit sum = 0
  for x in xs
    sum = sum + x
  sum

--- Over a range: `a..<b` excludes `b`, `a..b` includes it. The bounds are evaluated once.
pub squares(n: Int) -> List[Int]
=
  let edit out: List[Int] = []
  for i in 0..<n
    out = out.append(item: i * i)
  out

--- The loop variable takes the integer type of the bounds: here `U8`.
pub bytes(low: U8, high: U8) -> List[U8]
=
  let edit out: List[U8] = []
  for b in low..high
    out = out.append(item: b)
  out

--- An `if` without `else` is a statement.
pub count_large(xs: List[Int], limit: Int) -> Int
=
  let edit count = 0
  for x in xs
    if x > limit
      count = count + 1
  count
```

- **The bounds,** and the list, are evaluated once, before the first iteration. There is no `break` and no `continue`.
- **The loop variable** is a plain binding (U2019 on assignment). A range's bounds are integers of one type, `Int` or a fixed-width type, and a literal bound takes the type of the other, so `for b in low..high` over `U8` bounds gives `b: U8`. An inclusive range ends at its upper bound without computing its successor, so `0..255` over `U8` does not overflow.
- **As a statement,** a loop has the value `unit`. It updates `let edit` bindings declared before it.
- **Verification.** The refinements of the bindings the loop updates are its invariant: the body is checked once, for any iteration, with those bindings fresh at their declared types, and no fact about their values survives the loop. Inside a range loop, `a <= i < b` (or `<= b`) holds. There is no `invariant` clause.
- **Failures** inside the body end the loop and the function; the bindings it updated keep their values from before the loop (§5.6).
- **Labels.** A loop over a list runs under the top label of the list, and a range loop under the labels of its bounds. In such a body, `fail` and calls that can fail are U4002 when that label is not `public`.
- **Edition `0.4`** adds `stop` and `skip`, which end a loop or an iteration early, `for until c decreases m`, which runs until `c` holds, and `for ever` with `on stop` in a function that says `diverges`, which runs until it is stopped (`spec/26 §3` to `§6`). See [Loops that run until stopped](language/control-flow/for-ever.md).

### 5.6 Mutable value semantics (edition `0.2`)

Una has no references and no lifetimes: values stay values, and a binding declared editable can be rebound (ADR 0056, `spec/17`).

- **`let edit x = e`** declares an editable binding, and `x = e` rebinds it. A **place** is an editable binding or a field path of one: `t.last = value` rebuilds `t` with that field changed. Assigning anything else is U2019.
- **Fixed type.** An editable binding keeps its declared type, refinement and label at every rebind; each rebind carries the refinement obligation of an annotated `let`. Without an annotation, its type is that of its initial value without its refinement.
- **`edit` parameters** are passed in and out: `record(edit t: Tally, ...)` receives the caller's value and returns its final value. The caller writes `edit t` at the argument (U2020 for a missing or extra marker), and the argument is a place. In `ensures`, `t` is the final value and `old(t)` the value on entry.
- **`take` parameters** consume their argument: after `f(take a)`, using `a` is U2017 until an assignment gives it a new value.
- **Exclusivity.** An `edit p` argument together with any other use of the root binding of `p` in the same call is U2018.
- **Lists** change through `set` and `append`: `edit ys.set(index: i, item: v)`, with `requires 0 <= index and index < xs.len`.
- **Transactional values.** A call that fails leaves its `edit` arguments unchanged for the caller. Effects already performed stay performed.
- **Operations and handlers** cannot declare `edit` or `take` parameters (U2020).

```una
module demo

pub kind Tally
  seen: Int
  last: Int

--- An `edit` parameter is passed in and out; `old(t)` is its value on entry.
pub record(edit t: Tally, value: Int) -> Unit
  ensures t.seen == old(t).seen + 1
=
  t.seen = t.seen + 1
  t.last = value

pub tally(xs: List[Int]) -> Tally
=
  let edit t = Tally(seen: 0, last: 0)
  for x in xs
    edit t.record(value: x)
  t

--- `take` consumes its argument: the caller cannot use it afterwards.
pub finish(take t: Tally) -> Int = t.seen

pub summary(xs: List[Int]) -> Int
=
  let t = tally(xs)
  let n = finish(take t)
  n

--- List elements change through `set`.
pub reset_first(xs: List[Int]) -> List[Int]
=
  let edit ys = xs
  if ys.len > 0
    edit ys.set(index: 0, item: 0)
  ys
```

```una
-- fails with U2017
module demo

pub finish(take xs: List[Int]) -> Int = xs.len

pub twice(xs: List[Int]) -> Int
=
  let n = finish(take xs)
  xs.len + n
```

```una
-- fails with U2019
module demo

pub bump(x: Int) -> Int
=
  let n = x
  n = n + 1
  n
```

A backend may update a uniquely held value in place, and reuse the memory of a value passed by `take`. None of this is observable.

### 5.7 Concurrent tasks with `together` (edition `0.2`)

`together` runs independent work as concurrent tasks, and every task ends before its block does (ADR 0065, `spec/19`). A program's result and its trace are those of running the tasks one after another, in order.

```una
module demo

pub capability Sensor
  op reading() -> Int

pub kind FetchError = Missing | Gone

handler fixed(value: Int) for Sensor
  op reading() = value

pub warmest() -> Int
  uses @north: Sensor[reading]
  uses @south: Sensor[reading]
=
  together
    let north = reading() with @sensor = @north
    let south = reading() with @sensor = @south
  max(north, b: south)

reading() -> Int
  uses @sensor: Sensor[reading]
=
  @sensor.reading()

fetch(id: Int) -> Int
  fails FetchError
=
  if id < 0
    fail Missing
  else
    id * 10

--- One task per element; the results come back in list order.
pub pages(ids: List[Int]) -> List[Int]
  fails FetchError
=
  let pages = together for id in ids
    fetch(id)
      if error pass
  pages

--- Over a range, the variable takes the integer type of the bounds.
pub doubled(n: U8) -> List[Int]
=
  let out = together for i in 0..<n
    i.to_int() * 2
  out
```

- **`together`** is a statement whose indented body holds two or more `let` statements, its **branches** (U2024 otherwise). Each branch's expression, with its `with` bindings and its handling clauses, runs as a task. The branches' bindings are in scope after the block.
- **`together for x in xs`** stands where a `let` value can, and runs one task per element of a list or of a range. Its body is a block, and the result is the list of the body's values, in element order. Over a range, `x` takes the integer type of the bounds, as in a `for` loop.
- **Handling clauses** go on their own indented lines below the call, as everywhere (§6.3).
- **Captures.** A task sees the bindings before the block, never its siblings', and captures their values. Assigning a binding declared outside the task, or passing it as `edit` or `take`, is U2023: return new values from the task instead.
- **Capabilities** in scope can be used by every task, and so can `uses of` parameters.
- **Failures.** When a task fails, the tasks after it are canceled, and the block fails with the failure of the lowest-indexed failing task, as running the tasks one after another would. A canceled task stops at its next call, operation or loop iteration; an operation in progress completes.
- **Labels.** `together for` runs its tasks under the top label of its list, or the labels of its range bounds, as a `for` loop does, and its result carries that label.
- **Traces** list the events of the tasks in task order, each with its task's path, as `"task": "2.1"`. A canceled task that had started ends with a `cancelled` event.
- **Runtimes.** The interpreter runs the tasks one after another. A native executable runs the tasks on `--workers` threads, one per core by default, so the lines that tasks print may interleave.

```una
-- fails with U2023
module demo

bump(edit n: Int) -> Int
=
  n = n + 1
  n

pub count(xs: List[Int]) -> Int
=
  let edit seen = 0
  together
    let a = xs.len
    let b = bump(edit seen)
  seen + a + b
```

```una
-- fails with U2024
module demo

pub one() -> Int
=
  together
    let a = 1
  a
```

---

## 6. Errors

Errors are values. There are no exceptions, no panics and no implicit propagation.

### 6.1 Error kinds and `fails`

Any sum kind can serve as an error kind. A function declares at most one, with `fails E`.

```una
module demo

pub kind CheckInError = NotRegistered | Unavailable

pub kind StoreError = Timeout | Corrupted(detail: Text)
```

### 6.2 `fail`

`fail V` or `fail V(field: value)` ends the function with a failure. `V` must be a variant of the function's error kind (U3009).

```una
module demo

pub kind Seats = Int in 0..5000

pub kind Event
  name: Text
  remaining: Seats

pub kind BookingError = | InvalidCount

pub seats_after(e: Event, count: Int) -> Seats
  requires count >= 1
  requires count <= e.remaining
=
  e.remaining - count

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

A statement whose expression can fail must be followed by an indented `if error` clause (U3007), unless it is wrapped in `attempt`. Each clause stands on its own line, one level below the statement: `fetch(id) if error pass` on one line is U0101.

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
module demo

pub kind Registration
  name: Text

pub kind StoreError = Timeout | Corrupted(detail: Text)

pub kind CheckInError = NotRegistered | Unavailable

pub capability Registrations
  op find(key: Text) -> Option[Registration]
    fails StoreError

pub capability Store
  op read(path: Text) -> Text
    fails StoreError

pub capability Cache
  op get(key: Text) -> Option[Text]
    fails StoreError

pub lookup(key: Text) -> Registration
  uses @registrations: Registrations[find]
  fails CheckInError
=
  let registration = @registrations.find(key)
    if error fail Unavailable
    if none fail NotRegistered
  registration

pub load(path: Text) -> Text
  uses @store: Store[read]
  fails StoreError
=
  let detail = @store.read(path)
    if error fail Corrupted(detail: "read failed")
  detail

pub cached(key: Text) -> Option[Text]
  uses @cache: Cache[get]
=
  let found = @cache.get(key)
    if error use None
  found
```

### 6.4 `attempt`

`attempt` turns a single fallible call into a `Result`, so you can branch on the outcome instead of failing:

```una
module demo

pub kind Registration
  name: Text

pub kind StoreError = | Timeout

pub capability Registrations
  op find(key: Text) -> Option[Registration]
    fails StoreError

pub registered(key: Text) -> Bool
  uses @registrations: Registrations[find]
=
  let outcome = attempt @registrations.find(key)
  when outcome
    is Ok(value) -> value.is_some()
    is Err(failure) -> false
```

The operand must be one call (U2001), and `attempt` counts as its handling site.

### 6.5 `checked` (escape hatch)

`checked(v, p) else fail V` tests predicate `p` at runtime, with `it` bound to `v`. The package must allow it in its escape budget:

`una.toml`

```toml
[package]
name = "phones"
version = "0.1.0"
edition = "0.2"

[escape_budget]
checked_per_module = 1
```

`src/phones.una`

```una
module phones

pub kind Phone = Text where valid_phone(it)

pub kind PhoneError = | Malformed

pub predicate opaque valid_phone(t: Text) -> Bool = t.len == 11 and is_digits(t)

pub parse_phone(raw: Text) -> Phone
  fails PhoneError
=
  checked(raw, valid_phone(it)) else fail Malformed
```

- If the test passes, the result is `v` with its type refined by `p`. This is how opaque facts are established.
- If it fails, the function fails with `V`.
- `p` must be RL or an opaque predicate call, so it can run.
- `checked` replaces a proof with a runtime test, so it is **budgeted**: `[escape_budget] checked_per_module` in `una.toml`, default 0 (U7003 when exceeded). Every use is listed in the build report. A narrowing integer conversion can go through `checked` too, on the same budget.

### 6.6 What is not a failure

- **Proof obligations never become runtime failures.** Division by zero, overflow, refinement violations and broken preconditions are proven absent before the program runs.
- **Running out of call depth, stack or memory** is runtime diagnostic U6003, not a Una failure.

---

## 7. Capabilities and effects

### 7.1 Declaring capabilities

A capability describes authority over something outside the program. Its operations are signatures, which are assumptions about the external system.

```una
module demo

pub kind Availability = SoldOut | Few | Open

pub kind Registration
  name: Text
  status: Availability

pub kind StoreError = Timeout | Corrupted

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

Canonical op clause order: every `requires`, every `ensures`, then at most one `fails` and at most one `accepts` (U0102). Operations cannot declare `edit` or `take` parameters (U2020).

### 7.2 Using capabilities

```una
module demo

pub kind Availability = SoldOut | Few | Open

pub kind Registration
  name: Text
  status: Availability

pub kind StoreError = Timeout | Corrupted

pub kind CheckInError = NotRegistered | Unavailable

pub capability Registrations
  op find(key: internal Hashed) -> Option[Registration]
    fails StoreError
    accepts internal

pub capability Audit
  op record(key: internal Hashed, status: Availability) -> Unit
    fails StoreError
    accepts internal

pub check_in(key: internal Hashed) -> Availability
  uses @registrations: Registrations[find]
  uses @audit: Audit[record]
  fails CheckInError
=
  let registration = @registrations.find(key)
    if error fail Unavailable
    if none fail NotRegistered
  @audit.record(key, status: registration.status)
    if error fail Unavailable
  registration.status
```

- `uses @x: C[ops]` grants operations `ops` of capability `C` under the name `@x`. Omitting `[ops]` grants all of them.
- `@x.op(args)` works only if `@x` is in scope (U3001) and `op` is granted (U3002).

### 7.3 Passing capabilities: by name, downward only

A callee whose header says `uses @x: C[ops]` receives the caller's `@x` automatically. The caller must hold `@x` with at least those operations (U3001, U3002). Holding more is fine: the callee is limited to what it declares, so passing a capability attenuates it.

### 7.4 `with`: rebinding and handler instances

```una
module demo

pub kind Registration
  name: Text

pub capability Registrations
  op find(key: Text) -> Option[Registration]

handler in_memory(rows: List[Registration]) for Registrations
  op find(key) = rows.first()

handler none_found() for Registrations
  op find(key) = None

known(key: Text) -> Bool
  uses @registrations: Registrations[find]
=
  let found = @registrations.find(key)
  found.is_some()

pub check(key: Text, rows: List[Registration]) -> Bool
  uses @primary_store: Registrations[find]
=
  let in_store = known(key) with @registrations = @primary_store
  let in_rows = known(key) with @registrations = in_memory(rows: rows)
  let nowhere = known(key) with @registrations = none_found()
  in_store or in_rows or nowhere
```

- **`@x = @y`** makes the callee's `@x` the caller's `@y`. Both must be the same capability.
- **`@x = h(args)`** installs handler `h` as `@x`. The handler's own `uses` become effects of the caller. What happens when the caller lacks them depends on the caller:
  - a `pub` caller that does not declare them gets U3003 at the statement: "handler `h` uses `@inner: Inner[ping]`, but this code declares no `uses`, so its effects would exceed its header";
  - a private caller has them inferred, and the error appears where the capability is really missing. In an example, for instance, it is U3001: "`g` needs `@inner: Inner[ping]`, which no `given` line binds".
- **Scope:** the binding covers only that statement's expression.
- **Statements only.** `with` is part of a statement, so it cannot appear in the short function form: `g() -> Unit = f() with @a = h()` is U0101. Use the long form, with the statement in the body.
- **Several bindings** are separated by commas: `with @a = @b, @c = h()`.

### 7.5 Handlers

A handler implements a capability in Una. `in_memory` and `none_found` above are handlers:

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

Any of these is U3005. A bare `@x` is allowed only on either side of a `with` binding or in `given`. Effectful or fallible lambdas follow the same restrictions, except that they can be passed as arguments (U3006). Such a lambda cannot handle a failure, so it calls only operations that cannot fail (U3007 otherwise, §4.6). Second-class types are not type arguments either (U3016).

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
| `Http` (edition `0.2`) | `get(url: Text, headers: List[Header], key_name: Option[Text]) -> internal Response` | `HttpError` | `internal` |
| `Http` | `post(url: Text, headers: List[Header], body: Text, key_name: Option[Text]) -> internal Response` | `HttpError` | `internal` |
| `Http` | `put(url: Text, headers: List[Header], body: Text, key_name: Option[Text]) -> internal Response` | `HttpError` | `internal` |
| `Http` | `delete(url: Text, headers: List[Header], key_name: Option[Text]) -> internal Response` | `HttpError` | `internal` |

- **Entry.** Host capabilities enter only through `main`'s `uses`, and only if `una run`, or the native executable, grants them. An ungranted capability is U3012 before execution starts.
- **Grant syntax:** `console`, `clock`, `env`, `keys:<keyfile>`, `files:<path>:read`, `files:<path>:write`, and in edition `0.2` `http:<origin>` or `http:<origin>:<methods>` (by default `get`).
- **`Http`** (`spec/24`). `Header` has `name: Text` and `value: internal Text`, and `Response` has `status: Int`, `headers: List[Header]` and `body: Text`. A status is not a failure: `HttpError` is `Denied`, `BadUrl`, `BadRequest`, `Unreachable`, `TimedOut`, `TooLarge`, `Tls` or `Protocol`. `key_name: Some(n)` sends the key `n` of the `keys:` grants as `Authorization: Bearer <key>`; a token another header carries goes in as a header value, through a policy to `internal` when it is `secret`. The guide for agents, §17, has an example with a stand-in handler.
- **`Files` paths.** A path is relative to the grant's prefix: with `files:data:read`, `@files.read("readings.txt")` reads `data/readings.txt`, and `"data/readings.txt"` would look for `data/data/readings.txt` and fail with `NotFound`. A path stays confined to the prefix: an absolute path, or a `..` that leaves it, fails with `Denied`. `FileError` is `NotFound`, `Denied` or `Io`, and `KeyError` is `UnknownKey`.
- **Tests and handlers.** You can write handlers for host capabilities, for example a fake `Keys` in tests (§17).

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
module demo

pub kind Person
  name: Text
  age: secret Int

--- The branch is on secret data, so the result is secret too.
pub adult_flag(r: Person) -> secret Int = if r.age > 17 then 1 else 0
```

- **Returns.** A function's result label must fit its declared return type (U4001).
- **Editable bindings** keep their declared label: a rebind under a pc above it is U4001.

### 8.3 Failing and calling under a raised pc

- `fail` inside a branch on secret data leaks information through the failure, so it is rejected (U4002). Return a labeled value instead.
- Calling a function that performs effects or can fail is allowed under a raised pc only if every sink it reaches accepts that pc (U4002).

```una
-- fails with U4002
module demo

pub kind PinError = | Rejected

pub check(pin: secret Int) -> secret Int
  fails PinError
=
  if pin == 1234
    1
  else
    fail Rejected
```

### 8.4 Sinks

- **Operation arguments are sinks.** Every capability operation argument is a sink bounded by the operation's `accepts` label: `pc ⊔ label(args) ⊑ accepts`, otherwise U4003.
- **Console is public.** `Console.print` accepts `public` only:

```una
-- fails with U4003
module demo

pub show(phone: secret Text) -> Unit
  uses @console: Console[print]
=
  @console.print("phone {phone}")
```

- **Tests are exempt.** `expect` lines in examples observe values for the test harness and are exempt from sink checks.

### 8.5 Declassification: policies in trusted modules

Labels are lowered only through a declared policy, inside a trusted module:

`una.toml`

```toml
[package]
name = "events"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["PII"]

[trusted]
modules = ["events.keys"]
```

`src/events/types.una`

```una
module events.types

pub kind Phone = Text where valid_phone(it)

pub predicate opaque valid_phone(t: Text) -> Bool = t.len == 11 and is_digits(t)
```

`src/events/keys.una`

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

Una guarantees **termination-insensitive noninterference up to declassification**. The channels in the model are values, failures, operation arguments and the occurrence of effects. Timing, termination and resource usage are outside the model, and so is the relative order of the effects of concurrent tasks (§5.7); the build report says so.

---

## 9. Requirements

Requirements record human intent with origin and owner, so contracts can be traced to it.

```una
module demo

requirement EVENTS_3
  text "An event with no remaining seats is always shown as sold out"
  source policy "Box Office Handbook 2.1"
  owner box_office
  status active

pub kind Availability = SoldOut | Open

pub availability(remaining: Int) -> Availability
  ensures remaining < 1 implies result is SoldOut
  derived from EVENTS_3
=
  if remaining < 1 then SoldOut else Open
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
module demo

import std.testing (fake_hash)

pub kind Availability = SoldOut | Few | Open

pub kind Registration
  name: Text
  status: Availability

pub kind StoreError = Timeout | Corrupted

pub kind CheckInError = NotRegistered | Unavailable

pub capability Registrations
  op find(key: internal Hashed) -> Option[Registration]
    fails StoreError
    accepts internal

pub capability Audit
  op record(key: internal Hashed, status: Availability) -> Unit
    fails StoreError
    accepts internal

pub check_in(key: internal Hashed) -> Availability
  uses @registrations: Registrations[find]
  uses @audit: Audit[record]
  fails CheckInError
=
  let registration = @registrations.find(key)
    if error fail Unavailable
    if none fail NotRegistered
  @audit.record(key, status: registration.status)
    if error fail Unavailable
  registration.status

handler one_registration(r: Registration) for Registrations
  op find(key) = Some(r)

handler no_registrations() for Registrations
  op find(key) = None

handler audit_log() for Audit
  op record(key, status) = unit

example "a registered attendee is checked in and audited"
  given @registrations = one_registration(Registration(name: "Ana", status: Open))
  given @audit = audit_log()
  do check_in(fake_hash(1))
  expect result is Open
  expect performed @audit.record

example "an unknown attendee fails and is not audited"
  given @registrations = no_registrations()
  given @audit = audit_log()
  do check_in(fake_hash(1))
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
- **`do` lines** may nest fallible and effectful calls, and any failure ends the example. They need no handling clauses. A `do` line has no editable binding, so a call with `edit` or `take` there is U3013: exercise it through a function that edits a local copy.
- **Names** are unique per module.
- **Isolation.** Each example runs in a fresh abstract machine. `una test --json` reports the outcome and the full effect trace, with labeled arguments redacted.
- **`std.testing`** (`fake_hash`) is usable only in examples and in handlers installed only by `given`, and only by calling it, never as a function value (U3015).

---

## 11. Verification

### 11.1 What the compiler proves

| Obligation | Created at |
| --- | --- |
| `refinement` | every point where a value flows into a refined type: arguments, returns, fields, annotated `let`, and every rebind of an editable binding |
| `precondition` | every call and capability operation with `requires` |
| `postcondition` | every normal return of a function or handler operation with `ensures` |
| `division` | `/` and `mod` by a non-literal |
| `exhaustive` | `when` with range or literal arms and no `else` |
| `overflow` | each `+`, `-`, `*`, `/`, `mod` and unary `-` on a fixed-width type in code, and each narrowing conversion (edition `0.2`) |

When proving an obligation, the compiler may assume:
- refinements of values in scope;
- the enclosing function's `requires`;
- the branch conditions on the current path;
- the `ensures` of functions already called;
- operation `ensures`;
- facts from `checked`;
- the ranges of fixed-width values, and of a range loop's variable.

### 11.2 Statuses

| Status | Meaning | Build |
| --- | --- | --- |
| `proven` | the solver proved the goal | ok |
| `refuted` | a counterexample exists | error U5001, with the counterexample |
| `unknown` | the solver could not decide | error U5002: unknown is never accepted |

A U5001 or U5002 comes with the ranked repair candidates that apply at its site, each with an escape cost. None has a mechanical edit, and none is applied automatically. For a `refinement` or `precondition` obligation, with goal `G`, the compiler words them, in this order:

| Repair | When it is proposed | Escape cost |
| --- | --- | --- |
| strengthen the type of the parameter `p` to a refined kind that implies `G` | the value is the parameter `p` of the enclosing function | 0 |
| add `requires G` to the enclosing function; its callers then prove it | the enclosing function is not `pub`, and `G` names only its parameters | 0 |
| add a guard (`if` or a `when` arm) that handles the values where `G` does not hold | always | 0 |
| check the value with `checked(value, G) else fail <Variant>` | always | 1 |

`checked` is an escape: its repair carries `"escape": {"kind": "checked", "cost": ...}` in `--json`, and the human output says `escape: checked` with its cost. Escapes always come after the repairs that keep the proof, and so do the other escapes, a raised `checked_per_module` (U7003) and a trusted module or policy (U4003, U4004, U7005) (ADR 0095).

Inside an example there is no parameter and no `requires`, so only the guard and `checked` remain. The other kinds have their own repairs, all with escape cost 0:

- a `postcondition`: add a guard or an arm so that the failing path returns a value for which the `ensures` holds, or adjust the `ensures` clause;
- a `division`: guard the division with `if b != 0`;
- an `exhaustive` obligation: add an `else` arm, or arms for the values the others miss;
- an `overflow`: guard the operation with a test computed on `Int`, widen the values, or use a wrapping or saturating method (§3.1).

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

**Prelude types:** `Int`, the fixed-width integers `I8` to `U64`, `Nat`, `Bool`, `Text`, `Unit`, `List`, `Option`, `Result`, `Hashed`, the constraints `Eq`, `Ord` and `Integer`, plus the host capabilities of §7.7 with `FileError` and `KeyError`.

**Prelude functions.** All are pure. Their contracts are available to proofs. The signatures are those of edition `0.2`:

| Function | Contract |
| --- | --- |
| `abs(x: Int)` | `abs(x) >= 0`; `Int` only, since `abs` of a fixed-width value can overflow |
| `min[T: Ord](a, b:)`, `max[T: Ord](a, b:)` | exact bounds: the result is one of the arguments, and below or above both |
| `sum[T: Integer](xs) -> Int` | adds the values as `Int`, so it never overflows |
| `sort[T: Ord](xs)` | same length; ascending and stable |
| `contains[T: Eq](xs, item:)` | |
| `is_digits(t)`, `starts_with(t, prefix:)`, `trim(t)`, `to_int(t) -> Option[Int]` | `trim(t).len <= t.len` |
| `is_empty(xs)`, `first(xs)`, `at(xs, index:)` | `is_empty(xs) == (xs.len == 0)`; `first` and `at` return `None` exactly when out of range |
| `append(xs, item:)` | length grows by 1 |
| `set(edit xs, index:, item:)` | `requires 0 <= index and index < xs.len`; length unchanged |
| `map(xs, each:)` | same length; `uses of each`, `fails of each` |
| `filter(xs, keep:)` | length ≤ original; `uses of keep`, `fails of keep` |
| `first_where(xs, keep:)`, `fold(xs, initial:, step:)` | `uses of` and `fails of` their function parameter |
| `is_some(o)`, `or_else(o, default:)`, `is_ok(r)` | |

In edition `0.1`, `min`, `max`, `contains` and `sum` take `Int` (or any type, for `contains`), `sort` and `set` do not exist, and the higher-order functions take only pure, non-failing functions.

**Result labels.** Structural functions (`append`, `first`, `map`, `filter` and the like) label each position of the result with what it holds. The others (`contains`, `sum`, `min`, `max`, `sort`) read the element values, so their results carry the labels of every argument, element types included.

```una
module demo

pub stats(xs: List[Int], tags: List[Text]) -> Int
=
  let sorted = sort(xs)
  let biggest = max(1, b: 2)
  let first = sorted.first().or_else(default: 0)
  let doubled = xs.map(each: fn(x) -> x * 2)
  let positive = xs.filter(keep: fn(x) -> x > 0)
  let found = tags.first_where(keep: fn(t) -> t.starts_with(prefix: "a"))
  let has_zero = xs.contains(item: 0)
  let longer = xs.append(item: 7)
  let digits = "123".is_digits()
  let parsed = "42".to_int().or_else(default: 0)
  doubled.sum() + positive.len + first + biggest + parsed

--- `sum` takes any integer type and returns an `Int`, so it never overflows.
pub byte_total(bytes: List[U8]) -> Int = bytes.sum()

pub ordered(names: List[Text]) -> List[Text] = sort(names)
```

**`std.testing`:** `fake_hash(seed: Int) -> internal Hashed`. Equal seeds give equal hashes. From edition `0.4`, it also has `elapse`, `test_request` and `test_exchange` ([Tasks and time](tasks-and-time.md), [Serving HTTP](serving-http.md)). Edition `0.4` also adds the modules `std.time`, `std.drive` and `std.limit` (`spec/26 §2`, `spec/27 §10`).

---

## 13. Canonical form

Every file has one canonical form. `una fmt` rewrites a file into it, and `una check` reports every difference as U7001, an error with a mechanical repair. Code that a program generates should go through `una fmt` before `una check`. A file with a syntax error has no canonical form: it is neither formatted nor checked for U7001. The rules, from `spec/15`:

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

`una.toml` describes the package. The package of §17 shows every section:

| Key | Meaning |
| --- | --- |
| `[package]` `name`, `version`, `edition` | required; `edition` is `"0.1"`, `"0.2"` or `"0.3"` (a preview); edition `0.4` is described by `ita.una`, not `una.toml`, and `una migrate` moves a package there |
| `[labels]` `categories` | the label categories of §8.1 |
| `[trusted]` `modules` | the trusted modules of §8.5 |
| `[escape_budget]` `checked_per_module` | how many `checked` each module may use (default 0) |
| `[run]` `entry`, `grants` | the module whose `main` runs, and the grants of `una run` and of the native executable |

- **Editions.** The edition decides the language of the package: edition `0.2` has the memory model (§5.5, §5.6), fixed-width integers (§3.1), recursive kinds (§3.2), constraints (§3.3), effect polymorphism (§4.2) and `together` (§5.7). Edition `0.1` keeps the language of v0.1 in every version of the compiler. A single `.una` file has the default settings, and so edition `0.1`. Edition `0.3` is a preview: it has everything of edition `0.2`, and the rules of ADRs 0092 to 0094 join it as they land, which only ever adds errors. Edition `0.4`, in `ita.una`, is the preview of this release: it adds tasks and time, `for ever`, driven programs, HTTP servers and dependencies through Itá ([Edition](configure/package/edition.md)).
- `verification.lock` pins the solver so that proofs are reproducible.
- Unknown keys in `una.toml` are U7002. A missing or invalid lock is U7004.

---

## 15. Command line

```text
una [--json] [--color never|auto|always] <COMMAND>
```

| Command | Purpose |
| --- | --- |
| `una check [path]` | tiers 0 and 1; `--tier 0` skips the solver; `--dump-core <module>` prints the Core IR |
| `una fmt [path]` | rewrite to canonical form; `--check` reports U7001 without writing |
| `una fix [path]` | apply the mechanical repairs of `una check`, then format, and report what remains; `--tier`; `--dry-run` writes nothing |
| `una core [path]` | print the package's Core IR; with `--json`, as `una.core/1`; `--with-rejected`, with `--json` only |
| `una run [path]` | run `main` of the `[run] entry` module in the interpreter; `--grant <spec>`, repeatable; `--max-depth <n>`; `--max-memory <MiB>`; the other flags are listed below |
| `una build [path]` | compile `main` into a native executable, written to `target/una/<name>`; `--out <file>`; `--debuggable` |
| `una test [path]` | run all tests (`example` in editions 0.1 and 0.2); `--max-depth <n>`; `--max-memory <MiB>`; `--workers`, `--task-depth`, `--io-threads`, `--file-threads` |
| `una migrate [path]` | move a package from `una.toml` and `verification.lock` to `ita.una` and `ita.lock`, edition `0.4`; `--dry-run`; `--name <name>` |
| `una ita <command>` | the package manager of edition `0.4`: `lock`, `update`, `add`, `remove`, `tree`, `fetch`, `vendor`, `verify`, `publish`, `yank`, `audit`, `watch`, `bundle` and `unbundle`; in this release it reads path, workspace and mirror sources only ([Itá commands](configure/ita/commands.md)) |
| `una obligations [path]` | list every obligation and its status |
| `una contract <module.function>` | full header, declared or inferred |
| `una trace <REQUIREMENT_ID>` | requirement → contracts → proof status |
| `una report [path]` | build report: proofs, authority, policies, escapes, assumptions, trusted code |
| `una explain <code>` | explanation of a diagnostic, with a failing and a fixed example |
| `una lsp` | the language server, for editors |

`[path]` is a package directory or a single `.una` file, and defaults to the current directory.

**Global flags**

- `--json` prints one JSON document on stdout, and nothing else there.
- `--color never|auto|always` sets colors in human output; `auto`, the default, uses them when the output is a terminal.

**Checking before running**

- `una run`, `una build` and `una test` first check the package at tier 1, like `una check`, so the Z3 built into `una` must be the exact version of `verification.lock`; `una` never runs a `z3` from the path.
- If the solver is missing or has another version, they stop with U5003. If the checks report errors, nothing runs and no executable is written.

**Solver flags.** The commands that check at tier 1 (`check`, `fix`, `run`, `build`, `test`, `obligations`, `trace`, `report`) take `--solver-wallclock <seconds>` and `--solver-memory <MiB>`.

- `--solver-wallclock` is a wall-clock guard against a hanging solver query, 60 seconds by default.
- A query that hits it is `unknown` with reason `wallclock` (U5002), marked non-deterministic in the build report.
- `--solver-memory` is the memory limit of each solver query, from 256 to 1048576 MiB, 4096 by default; another value is a usage error.

**`una run`**

- It runs `pub main() -> Unit` of the `[run] entry` module, or of the module of a single file.
- **Grants.**
  - `--grant` may repeat, and the flags are combined with `[run] grants` of `una.toml` into the final grants.
  - Grant specs: `console`, `clock`, `env`, `keys:<keyfile>`, `files:<path>:read`, `files:<path>:write` and the combined `files:<path>:read,write`.
  - From edition `0.2`, also `http:<origin>` and `http:<origin>:<methods>`, which grant `Http`. From edition `0.4`, an `http:` spec may end with limit suffixes such as `;10/second` or `;waiting=5` (`spec/27 §9`), and `serve:<scheme>://<address>:<port>` with `;<item>` suffixes lets the run accept connections there and grants `HttpServer` ([Serving HTTP](serving-http.md)).
  - Relative paths in grants resolve against the package root when they come from `una.toml`, and against the current directory when they come from `--grant`.
  - A `uses` of `main` that the final grants do not cover is U3012, before anything runs.
- **Depth and memory.** `--max-depth <n>` sets the maximum call depth, 10,000 by default, on a stack capped at 1 GiB. `--max-memory <MiB>` bounds the heap of the run, 4096 MiB by default. Exceeding either aborts with U6003.
- **Other flags.** `--max-response <MiB>` and `--http-timeout <seconds>` bound `Http` requests (`spec/24 §4`). The task flags `--workers`, `--task-depth`, `--io-threads`, `--file-threads` and `--serve-threads` size the threads of a run (`spec/27 §3`, `spec/31 §6`). `--stop-after-iterations <n>`, `--virtual-time`, `--cost-clock`, `--deadline <seconds>` and `--stop-grace <seconds>` control `for ever` loops, the clock and host stops (`spec/26 §6`, `§8`). `--trace <file>` writes the trace as a JSON array. `--serve-script <file>` and `--serve-out <file>` run only in the toolchain's own test builds: a shipped `una` refuses them as a usage error (`spec/31 §8`). `una run --help` lists every flag with its range and default.
- **Usage errors (exit code 2):**
  - a package without an entry, or an entry without `pub main() -> Unit`;
  - a `--grant` value that is not a grant spec;
  - an unreadable or invalid key file;
  - a `--max-depth` or `--max-memory` that is not a number, or a `--max-depth` whose stack cannot be allocated.
- **`--json` output.** `una run` prints a `una.run/1` document whose `status` is one of:
  - `ok`;
  - `failed`: `main` failed, U6001;
  - `aborted`: U6003 or U8001;
  - `rejected`: the checks reported errors, which the document's `diagnostics` holds, and nothing ran.

**`una build`** (ADR 0055)

- It checks the package at tier 1, then compiles it through Cranelift and links it, with the linker built into `una` on `aarch64-apple-darwin` and the system linker (`cc`) elsewhere, into an executable for the host: `aarch64-apple-darwin`, `aarch64-unknown-linux-gnu` or `x86_64-unknown-linux-gnu`. There is no cross-compilation. The release archives of 0.4.0 to 0.5.2 are for `aarch64-apple-darwin` and `aarch64-unknown-linux-gnu` only (`scripts/release-targets.txt`); on `x86_64-unknown-linux-gnu`, `una` is built from source.
- It writes `target/una/<name>`, where `<name>` is the `[package] name` (or the module name for a single file), and prints `built <output>`. With `--json` it prints a `una.build/1` document whose `status` is `built` or `rejected`.
- It embeds `[run] grants` in the executable as its default grants. Exit codes: `0` when the executable is written, `1` when the checks report errors, `2` for a usage error (as for `una run`, or a failure of the linker), `3` for U8001.
- **The executable** behaves as `una run` in human mode: the lines `main` prints go to stdout, U6001 and U6003 go to stderr with exit code 1, and U3012 is checked against its grants before `main` starts. It takes `--grant <spec>`, `--max-depth <n>`, `--max-memory <MiB>` and `--trace <file>`, which writes the trace as a JSON array, and also the flags of the **Other flags** item of `una run` above, except `--cost-clock`. It has no `--json` output.
- **Equivalence.** The executable's output, exit status, failure and trace are ones the interpreter produces; the interpreter is the reference. This is tested in both engines, not proven. With `together`, the lines that tasks print may interleave (§5.7).

**`una lsp`** (ADR 0063) serves the Language Server Protocol 3.17 on stdin and stdout, for editors. It publishes the diagnostics of `una check` with their repairs as quick fixes, formats documents, shows contracts, types and obligation statuses on hover, and goes to definitions. It checks at tier 0 on every change and at tier 1 on save, with the proof cache of the CLI. It never runs programs.

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
| U1001 to U1010 | names: resolution, duplicates, cycles, visibility, conventions, requirements, file paths, reserved module names, a module named like a dependency |
| U2001 to U2040 (no U2036) | types: mismatches, labels, exhaustiveness, constructors, RL, predicates, conventions, literals, tasks, constraints, units, floats, loops, `first of` and `start`, decimals, tensors, `simd required` |
| U3001 to U3033 (no U3030, U3031) | authority: capabilities, effects, failures, handlers, one effect per line, `of` clauses, boundaries, `diverges`, `HttpServer` in a branch |
| U4001 to U4008 (no U4006) | security: flows, implicit flows, sinks, policies, categories, interpolation in a pattern |
| U5001 to U5003 | verification: refuted, unknown, solver missing |
| U6001, U6003 to U6009 (no U6008) | runtime: `main` failed, call depth or memory limit exceeded, header values, UTF-8, the floating-point environment, a host stop, no random bytes for keyed hashing |
| U7001 to U7005, U7007 to U7015 | project: formatting, `una.toml`, escape budget, lock, trusted modules, forms a preview changed, `ita.una`, `ita.lock`, verification of packages, resolution, audits, executable layout |
| U8001 | internal compiler error (always a bug) |

**Codes of edition `0.2`**

| Code | Meaning |
| --- | --- |
| U2017 | use of a binding after it was passed by `take` |
| U2018 | an `edit` argument together with another use of its binding in the same call |
| U2019 | assignment to a binding that is not editable |
| U2020 | a parameter convention (`edit`, `take`) not matched at the call site, or declared where none is allowed |
| U2021 | a function with `edit` or `take` parameters used as a value |
| U2022 | an integer literal does not fit its type |
| U2023 | a binding declared outside a `together` task assigned, or passed as `edit` or `take`, inside it |
| U2024 | a `together` body with a statement other than a binding statement, or fewer than two branches |
| U2025 | a type argument does not satisfy the constraint of its parameter |
| U2026 | an operation on a type parameter that its constraint does not allow |
| U2027 | an unknown constraint name |
| U3017 to U3019 | an `of` clause that names no parameter, a parameter not of function type, or a function type with its own `uses` or `fails` |

**Common mistakes**

| You wrote | Code | Fix |
| --- | --- | --- |
| `return x * 2` | U0103 | the last expression is the result |
| `match x` | U0103 | use `when` |
| `0 < x < 10` | U0101 | `x in 1..<10` |
| `x;` | U0001 | remove the `;` |
| `if !done` | U0001 | `if not done` |
| `let result = ...` | U0101 | choose another name, such as `outcome` |
| `fetch(id) if error pass` on one line | U0101 | the handling clause on its own indented line |
| `ensures` before `requires` | U0102 | canonical clause order |
| `seats_after(e, 2)` | U2004 | `seats_after(e, count: 2)` |
| a `when` missing a variant | U2007 | add the arm or `else` |
| `e == Found` where `Found` has fields | U2009 | `e is Found` |
| `f(x)` as a statement, returning `Int` | U2011 | bind it with `let` |
| `n = n + 1` on a plain `let` | U2019 | `let edit n` |
| `let b: U8 = 300` | U2022 | a value in range, or a wider type |
| `a == b` on an unconstrained `T` | U2026 | `[T: Eq]` |
| `@log.write(...)` without `uses` | U3001 | add `uses @log: Log[write]` |
| `@` inside a function with no `uses` | U3004 | declare the capability |
| `let db = @db` | U3005 | pass it by name through `uses` |
| a fallible call with no `if error` | U3007 | add a handling clause or `attempt` |
| `classify(@store.get(k))` | U3013 | bind the effect on its own line |
| `fail` inside a branch on secret data | U4002 | return a labeled value |
| printing a secret value | U4003 | declassify through a policy, or don't print |
| `ensures` the code does not satisfy | U5001 | read the counterexample |
| `a + b` on `U8` without a bound | U5001 | `requires`, a guard, or a wrapping method |
| a proof the solver cannot decide | U5002 | strengthen types, add `requires` or a guard |
| a non-canonical layout | U7001 | run `una fmt` |

---

## 17. Complete example package

The events package: a phone number validated at the boundary, a trusted policy that hashes it, a check-in with two capabilities and its examples, and a `main` that prints. `una check`, `una test`, `una run` and `una build` all accept it, and the native executable prints what `una run` prints.

`una.toml`

```toml
[package]
name = "events"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["PII", "Location"]

[trusted]
modules = ["events.keys"]

[escape_budget]
checked_per_module = 1

[run]
entry = "events.main"
grants = ["console"]
```

`verification.lock`

```toml
encoding_version = "una-smt-1"
solver           = "z3"
solver_version   = "5.1.0"
logic            = "QF_UFLIA"
step_budget      = 5000000
random_seed      = 0
```

`src/events/types.una`

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

`src/events/seating.una`

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

--- Shows an event in its Una form.
pub describe(e: Event) -> Text = "{e}"

--- Shows a list of events in their Una forms.
pub describe_many(events: List[Event]) -> Text = "{events}"

example "an event with no seats is sold out"
  do availability(Event(name: "Launch", remaining: 0))
  expect result is SoldOut

example "an event record can be interpolated"
  do describe(Event(name: "Launch", remaining: 0))
  expect result == "Event(name: \"Launch\", remaining: 0)"

example "a list of event records can be interpolated"
  do describe_many([Event(name: "Launch", remaining: 0)])
  expect result == "[Event(name: \"Launch\", remaining: 0)]"

example "an event with many seats is open"
  do availability(Event(name: "Meetup", remaining: 120))
  expect result is Open

example "raw counts are clamped"
  do clamp_seats(9000)
  expect result == 5000
```

`src/events/keys.una`

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

`src/events/checkin.una`

```una
module events.checkin

import events.keys (attendee_key)
import events.seating (Availability, Event, Registration, availability)
import events.types (Phone, parse_phone)
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

`src/events/main.una`

```una
module events.main

import events.seating (Availability, Event, availability, clamp_seats, describe)

label(status: Availability) -> Text
=
  when status
    is SoldOut -> "sold out"
    is Few -> "few seats left"
    is Open -> "open"

pub main() -> Unit
  uses @console: Console[print]
=
  let events = [Event(name: "Meetup", remaining: clamp_seats(120)), Event(name: "Launch", remaining: 0)]
  for event in events
    let status = availability(event)
    @console.print("{describe(event)}: {label(status)}")
```

Running it:

```text
$ una check .
$ una test .
6 examples: 6 passed, 0 failed
$ una run .
Meetup (120 seats): open
Launch (0 seats): sold out
$ una build .
built target/una/events
$ ./target/una/events
Meetup (120 seats): open
Launch (0 seats): sold out
```

---

## 18. Not in v0.2

These are planned or deliberately absent. Using a reserved word gives U0103 (§1.5).

| Area | Status |
| --- | --- |
| Shared and isolated references (`shared`, `iso`, `weak`), cyclic structures | not in any edition yet; the words are reserved for a later edition; use arenas and indices |
| User traits and implementations (`trait`, `impl`), laws of constraints | later; edition `0.2` has `Eq`, `Ord` and `Integer` |
| Task values, detached tasks, `race`, `select`, timeouts, channels, actors | in editions `0.2` and `0.3`, `together` is the only concurrency; edition `0.4` (preview) adds started tasks (`tasks` and `start`), deadlines (`within`) and races (`first of`) ([spec/27](../spec/27-tasks-and-time.md)); `select`, channels and actors are in no edition |
| `break`, `continue`, an `invariant` clause for loops | not in v0.2 |
| A checked left shift | not in v0.2; use `wrapping_shift_left` or a multiplication |
| Cross-compilation of native executables | not in v0.2; each host builds for itself |
| Units of measure | edition `0.3` preview ([spec/25](../spec/25-units-of-measure.md), ADR 0104) |
| Pipeline operator (`\|>`) | not added; method-call chains already cover this form (ADR 0110) |
| Foreign code and FFI | not permitted in any edition (ADR 0121, rule 17) |
| Quantified laws and property tests | deferred: verification tiers 2 and 3 ([spec/12](../spec/12-verification.md), the maintainers' backlog) |
| Semantic diff | deferred; in no release yet ([availability](availability.md)) |
| Cost contracts | a later release, not yet scheduled (the maintainers' backlog) |
| External state semantics (idempotency, delivery, `observe`), durable effect journal | not in 0.5.2, and no edition plans them yet (ADR 0139, rule 6); `observe` is only a reserved word |
| Services, data access, infrastructure generation | not in 0.5.2, and no edition plans them yet; edition `0.4` (preview) has HTTP servers only ([spec/31](../spec/31-http-servers.md)) |
| Exceptions, null, inheritance, operator overloading, macros, implicit conversions, global mutable state, ambient authority | never |
