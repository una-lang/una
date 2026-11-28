# Una for agents

A compact reference for writing Una in packages of edition `0.2`, written for the v0.2 toolchain; later toolchains serve edition `0.2` unchanged. Editions `0.3` and `0.4` are previews: this guide notes where they differ and §22 moves a package to edition `0.4`, but it does not teach their new constructs (for edition `0.4`, see [Tasks and time](tasks-and-time.md), [Serving HTTP](serving-http.md) and [Driven programs](driven-programs.md)). The specification in `docs/spec/` is normative; this guide summarizes it for an agent that writes, checks and repairs code.

How to read the blocks: every `una` block is a whole module that `una check` accepts in canonical form, as the only module of a package of edition `0.2`, unless its first line is `-- fails with U####`, in which case checking it yields that code and no other. A block whose first line is `-- edition 0.1` is checked in a package of edition `0.1`. A block preceded by a file path belongs to a package with the other files named around it. Every `json` block is a document the CLI prints, and every `sh` block runs from the repository root, or in a fresh copy of the directory its `# copy` comment names. `crates/una-cli/tests/guide.rs` checks all of them.

## Editions

`una --version` prints the editions the installed compiler serves and which of them are previews (ADR 0139, ADR 0121). A package names its edition in `ita.una` (`edition 0.4` or `edition 0.5`) or in `una.toml` (`edition = "0.2"`); a single file is edition `0.1`.

| Edition | Status | Where it is taught |
| --- | --- | --- |
| `0.1` | stable | §21 of this guide, and the blocks that start with `-- edition 0.1` |
| `0.2` | stable | this guide, §1 to §20 |
| `0.3` | preview | [edition reference](configure/package/edition.md); this guide notes where it differs |
| `0.4` | preview | §22 of this guide, [Tasks and time](tasks-and-time.md), [Serving HTTP](serving-http.md), [Driven programs](driven-programs.md), [Itá](configure/ita/manifest.md) |
| `0.5` | preview | not taught in this guide: reference in the [edition reference](configure/package/edition.md), [`const` and `var`](language/types/const.md), [collections](../spec/28-collections.md) and [numerics](../spec/29-numerics.md) |

A preview edition may still change under the terms of ADR 0121: a later release may ask for edits to a package of a preview edition.

## Index by task

- Install `una`: [Getting started, §1](getting-started.md#1-install).
- First check of a package: [§1, the loop](#1-the-loop).
- Fix a diagnostic: [§1, the loop](#1-the-loop), step 3 (`una fix`, `una explain`), and [§7, failures](#7-failures).
- Dependencies and Itá: [§22](#22-migrating-to-edition-04), which moves a package to `ita.una`, and the [Itá reference](configure/ita/manifest.md).
- Proofs and limits: [§6, contracts and proofs](#6-contracts-and-proofs) and [Known limits](known-limits.md#assurance-in-this-release).

## First route

1. Detect what you were given: a directory with `ita.una` (edition `0.4` or later), a directory with `una.toml`, or a single `.una` file.
2. Run `una --version`. It prints the release and the editions it serves, for example `editions: 0.1, 0.2, 0.3, 0.4, 0.5` and `preview: 0.3, 0.4, 0.5`.
3. Read the `edition` line of the manifest and pick the guide for it in the table above. A single file is edition `0.1`.
4. Run `una fmt <path>`, `una check <path>` and `una fix <path>`: the loop of §1.

## 1. The loop

1. Write or edit `.una` files, then run `una fmt <path>`. The canonical form is part of the language: `una check` reports any other layout as U7001, so generated code goes through `una fmt` before `una check`.
2. Check: `una --json check <path>`. Exit code 0 means the package parses, is canonical, types, respects authority and information flow, and every proof obligation is proven by the solver.
3. Otherwise read the diagnostics. Each has a `code`, a one-line `message`, a `primary` position and often ranked `repairs`; a repair with an `edit` is mechanical: replace the range with `replacement`.
   `una fix <path>` applies the mechanical ones for you, in safe rounds, then formats and prints what remains (`--dry-run` writes nothing). It never applies an edit that widens authority or visibility (a `uses` clause, an operation, `pub`): those are listed as `skipped`, for you to decide.
4. When unsure, `una explain <code>` prints the rule with a failing and a fixed example.
5. Run the examples with `una --json test <path>`, list every proof with `una --json obligations <path>`, and read the build report with `una --json report <path>`.

```sh
una check examples/events
una --json check examples/toolshelf
una fmt --check examples/pantry
una fix --dry-run examples/pantry
una --json test examples/journal
una --json obligations examples/greenhouse
una --json report examples/shortlinks
una explain U3013
una --json explain U5001
una check tests/conformance/verification/postcondition_refuted.una  # exit 1
una trace EVENTS_3  # in examples/events
una contract events.checkin.check_in  # in examples/events
```

`una build` compiles a package into a native executable that behaves as `una run` (§19):

```sh
una build .  # copy examples/hello
./target/una/hello
```

Exit codes: `0` success; `1` errors in the program, an unproven obligation or a failed example; `2` a usage error, a problem of the invocation reported on stderr without a diagnostic document; `3` an internal compiler error (U8001), which is a bug to report. Problems of `una.toml` (U7002) and `verification.lock` (U7004) are diagnostics and exit with `1`. With `--json`, stdout holds exactly one JSON document and stderr stays empty.

## 2. Files and packages

- A package is a directory with `una.toml`, `verification.lock` and sources under `src/`. Module `shop.cart` lives in `src/shop/cart.una`, and a file holds exactly one module.
- **Imports** name a module of the same package by its path and list what they take from it: `import shop.prices (Item, total)`. Only `pub` declarations can be imported (U1004, with an edit that adds `pub`). Importing a sum kind brings its variants too. The only other modules are the standard ones, `std.host` and `std.testing` (§8). The prelude is always in scope and is never imported, and no package imports another. (Edition `0.4` also has the standard modules `std.time`, `std.drive` and `std.limit`, and dependencies on other packages through `ita.una`.)
- A single `.una` file is also a package, with default settings and the default lock.
- `una.toml` has `[package]` (`name`, `version`, `edition = "0.2"`), and optionally `[labels] categories`, `[trusted] modules`, `[escape_budget] checked_per_module` (default 0) and `[run]` (`entry` module, `grants`). Unknown keys are U7002.
- **Editions.** `edition = "0.2"` enables the constructs of v0.2: editable values and `for` loops (§11), fixed-width integers (§12), recursive kinds (§4), `uses of` and `fails of` (§8), constraints on generics (§13), maps (§14), JSON (§15), text functions (§16), HTTP (§17) and `together` (§18). A package of edition `0.1` keeps the language of v0.1 in every later compiler, and a single `.una` file outside a package is edition `0.1`. §21 lists what changes between them.
- `verification.lock` pins the solver: tier 1 runs the Z3 built into `una`, which must be the exact version the lock names, or it reports U5003. `una` never looks for a `z3` on the path, and installs no separate Z3; `una --version` prints the version it has. U5003 also reports a development build with no solver built in and a solver that could not be confined, such as one run as root (`una explain U5003`).

## 3. Syntax in brief

- Indentation is significant: two spaces per level, no tabs. A line break inside brackets continues the line, but the canonical form joins such lines, so `una check` reports the split as U7001 (and `una fmt` joins it) unless a comment sits inside the brackets. Write each call on one line.
- `--` starts a comment, `---` a doc comment attached to the next declaration.
- Names: `lower_snake` for values, functions, fields, modules, handlers and operations; `UpperCamel` for kinds, variants, capabilities and label categories; `@name` for capability references; `UPPER_SNAKE_1` for requirements.
- Literals: integers without leading zeros (`1_000` is fine), `true`, `false`, `unit`, and single-line text with `{expression}` interpolation. Text escapes are `\"`, `\\`, `\n`, `\t`, `\{`, `\}` and `\u{H}`: a literal brace is `\{` or `\}`, so JSON written in text reads `"\{\"id\": 7\}"`. A bare brace that cannot start or end an interpolation is U0007, with the edit that escapes it. There is no `\r`: a carriage return is `\u{D}`.
- Operators, tightest first: calls and `.field`; unary `-`; `*` `/` `mod`; `+` `-`; comparisons `==` `!=` `<` `<=` `>` `>=`, `is Variant` and `in 0..5`, which do not chain; `not`; `and`; `or`; `implies`.
- Edition `0.2` adds the keywords `edit`, `take`, `old` and `together`; `for` also starts a loop.
- Words that are keywords in other languages are errors with a hint (U0103): `match` (use `when`), `return` (the last expression is the result), `var` (use `let edit`), `while` and `loop` (use `for` or recursion), `type` (use `kind`), `async` and `await` (use `together`). Reserved for later versions: `shared`, `iso`, `weak`, `all`, `table`, `query` and others.
- A reserved word cannot name a binding: `let result = ...` is U0101, since `result` names a function's result in `ensures`. Pick another name, such as `outcome`.
- A handling clause (`if error ...`, `if none ...`) stands on its own line, indented under the expression it handles, never on the same line.

```una
-- fails with U0103
module demo

pub size(type: Text) -> Int = type.len
```

## 4. Kinds

```una
module demo

--- An alias: a Label is a Text.
pub kind Label = Text

--- A refinement with range sugar; an Int flows into it only with a proof.
pub kind Percent = Int in 0..100

--- A record.
pub kind Tool
  name: Text
  deposit: Nat

--- Sums, with and without payloads. A sum of one variant starts with `|`.
pub kind CheckoutError = NotOnShelf(tool: Text) | ShelfDown

pub kind Oops = | TooLarge

--- Generic kinds; parameters are unconstrained.
pub kind Bin[T]
  label: Label
  entries: List[T]

pub kind NonEmpty[T] = List[T] where it.len > 0
```

**A sum of one variant keeps its leading `|`:** write `kind Oops = | TooLarge`. Without it, `kind Oops = TooLarge` is an alias of a type named `TooLarge`. That is U1001 when no such type exists, or U2002 when `TooLarge` is a variant of some other kind. A sum of two or more variants has no leading `|`.

Built-in types: `Int` (unbounded), `Nat` (`Int` at least 0), `Bool`, `Text`, `Unit`, `List[T]` (`[a, b]`, `xs.len`), `Option[T]` (`Some(value)`, `None`), `Result[T, E]` (`Ok(value)`, `Err(failure)`), `Hashed` (from `Keys` only) and function types `fn(A) -> B`. Records and sums are nominal; aliases and refinements are their base type. Recursive kinds go only through `List` or `Option` in edition `0.1`; edition `0.2` allows any recursion with a base variant, as `kind Expr = Lit(value: Int) | Add(left: Expr, right: Expr)` (ADR 0060).

Recursive kinds contain themselves through any field, as long as the kind has a finite value: some variant must end the recursion, or the field must be an `Option` or a `List`. A kind without a finite value is U2002.

```una
module demo

--- A kind may contain itself when it has a finite value: `Lit` ends the recursion.
pub kind Expr = Lit(value: Int) | Add(left: Expr, right: Expr) | Neg(inner: Expr)

pub eval(e: Expr) -> Int
=
  when e
    is Lit(value) -> value
    is Add(left, right) -> eval(left) + eval(right)
    is Neg(inner) -> 0 - eval(inner)
```

```una
-- fails with U2002
module demo

pub kind Stream = | Next(head: Int, tail: Stream)
```

Interpolation writes any value that holds no function in its Una form, the one `una test` shows: `Point(x: 1, y: -2)`, `[1, 2]`, `Some(3)`, `Err(NotFound)`. Text inside a structure is quoted. A record or variant with one field prints it without its name, as `Some(3)` or `Offline("south")`; with two or more fields, each is named. A field that holds a function is U2001. A value of a type parameter `T` needs `[T: Eq]`. The text keeps the labels of every field, so a record with a `secret` field cannot be printed (U4003) (ADR 0068).

```una
module demo

pub kind Point
  x: Int
  y: Int

--- Interpolation writes the record itself: `at Point(x: 1, y: -2)`.
pub describe(p: Point) -> Text = "at {p}"
```

## 5. Functions

A function is `header = body`. Clauses come in this order: `requires`, `ensures`, `uses`, `fails`, `derived from`. With no clause and a one-line body, the short form is canonical.

```una
module demo

pub kind Item
  name: Text
  amount: Nat

--- Short form.
pub double(x: Int) -> Int = x * 2

--- Long form: the header is the contract, the body follows `=`.
pub restock(item: Item, added: Nat) -> Item
  ensures result.amount == item.amount + added
=
  Item(name: item.name, amount: item.amount + added)

--- Method calls and lambdas: `items.filter(keep: ...)` is `filter(items, keep: ...)`.
pub missing(items: List[Item]) -> List[Text]
  ensures result.len <= items.len
=
  items.filter(keep: fn(i) -> i.amount == 0).map(each: fn(i) -> i.name)

--- Generic functions take function parameters for what they cannot do themselves.
pub count_where[T](xs: List[T], keep: fn(T) -> Bool) -> Nat
  ensures result <= xs.len
=
  xs.filter(keep).len

example "restocking adds to the amount"
  do restock(Item(name: "rice", amount: 2), added: 3)
  expect result.amount == 5
```

- Arguments after the first are labeled, `f(a, b: 2)`; from the second argument on, a bare name equal to the parameter's name counts as labeled (`f(a, b)` when the parameter is `b`). The first argument always binds the first parameter, so `minus(b, a)` with parameters `(a, b)` is U2006, not a swap. Constructors with two or more fields label every field.
- Blocks bind with `let`; values are immutable, and a `let edit` binding can be rebound (§11). A name is bound once in its scope: a `let`, a pattern or a lambda parameter named like a binding already in scope, a parameter or an outer `let`, is U1002 ("`items` is already bound in this scope"). Rename the inner one; there is no shadowing. Two sibling blocks, as the arms of one `when`, may each bind the same name. `if c then a else b` fits on a line; `if` and `when` also take indented blocks.
- `when` branches on variants (`is Some(x)`, `is Err`), literals (`is 0`) and ranges (`in 1..<20`); an `else` arm comes last. Arms over variants must cover every variant (U2007).
- The prelude has `abs`, `min`, `max`, `is_digits`, `starts_with`, `trim`, `to_int`, `is_empty`, `first`, `at`, `append`, `contains`, `map`, `filter`, `first_where`, `fold`, `sum`, `is_some`, `or_else` and `is_ok`, and in edition `0.2` also `sort`, `set`, `empty_map` (§14), the JSON functions (§15) and the text functions (§16); their contracts are hypotheses for the solver (`spec/14`, `spec/20 §6`). Loop with `for` (§11), recursion, `map`, `filter` or `fold`.
- The result of `filter` and `first_where` carries the labels of the predicate's result at its top, since which elements remain depends on it: over `secret` elements, a predicate that reads them gives a list whose top is `secret` (`spec/14`, result labels, ADR 0067). A predicate with effects whose result is labeled is U4002 in `first_where`.
- A call of a generic function raises each position of its result only by the type arguments that position depends on; `una contract` shows them as `-- result labels: ...`. A list whose top depends on comparisons of `T` values carries `T`'s labels, and a parameter of a user function still takes only what its type allows, so build such lists with the prelude (`fold`, `append`), not by passing them to your own generic functions.

```una
-- fails with U2004
module demo

pub add(a: Int, b: Int) -> Int = a + b

pub three() -> Int = add(1, 2)
```

```una
-- fails with U1002
module demo

pub total(items: List[Int]) -> Int
=
  let items = items.filter(keep: fn(n) -> n > 0)
  items.sum()
```

## 6. Contracts and proofs

`requires` and `ensures` are written in the refinement language: literals, parameters, `result`, fields, `.len`, `+`, `-`, `*` by a literal, `/` and `mod` by a nonzero literal, comparisons, `in`, `is`, `and`, `or`, `not`, `implies` and predicate calls. Every call proves the callee's `requires`, every return proves the `ensures`, every value that flows into a refined type proves the refinement, and every division by a variable proves a nonzero divisor. The solver answers `proven`, `refuted` (U5001, with a counterexample) or `unknown` (U5002).

- The solver knows a call only through its contract. Prelude functions without one, such as `fold`, `sum` and `to_int`, tell it nothing, and refinements are not inferred through generic calls: a `fold` that adds naturals still returns an `Int`.
- `predicate reflect` bodies are in the refinement language; the solver sees their definition. Their integer arithmetic is exact on every type, so fixed-width arithmetic in them has no overflow obligation and an out-of-range literal there is not U2022; their integer types still do not mix (U2001).
- `predicate opaque` bodies may call any pure function; the solver sees only the name, and `checked(value, p(it)) else fail V` establishes one at run time. Each `checked` counts against the module's escape budget.

```una
module demo

pub kind Member
  name: Text
  holding: Int in 0..3

pub predicate reflect can_check_out(m: Member) -> Bool = m.holding < 3

pub check_out(m: Member) -> Member
  requires can_check_out(m)
  ensures result.holding == m.holding + 1
=
  Member(name: m.name, holding: m.holding + 1)

--- A caller proves the precondition: here, from the literal.
pub first_checkout() -> Member = check_out(Member(name: "Ana", holding: 0))
```

A false contract is refuted, with the path and the values that break it:

```una
-- fails with U5001
module demo

pub double(x: Int) -> Int
  ensures result > x
=
  x * 2
```

```json
{
  "schema": "una.diagnostics/1",
  "diagnostics": [
    {
      "schema": "una.diagnostic/1",
      "code": "U5001",
      "severity": "error",
      "domain": "verification",
      "message": "postcondition of `double` is refuted",
      "primary": { "file": "double.una", "start": { "line": 6, "col": 3 }, "end": { "line": 6, "col": 8 } },
      "related": [
        { "file": "double.una", "start": { "line": 4, "col": 11 }, "end": { "line": 4, "col": 21 }, "message": "ensures clause" }
      ],
      "obligation": { "id": "ob-17058da2cfca", "kind": "postcondition", "goal": "result > x", "status": "refuted" },
      "counterexample": { "bindings": [ { "name": "result", "value": 0 }, { "name": "x", "value": 0 } ] },
      "repairs": [
        { "rank": 1, "escape_cost": 0, "description": "add a guard or an arm so that the failing path returns a value for which `result > x` holds", "edit": null },
        { "rank": 2, "escape_cost": 0, "description": "adjust the `ensures` clause", "edit": null }
      ],
      "explain": "una explain U5001"
    }
  ]
}
```

The counterexample's values are the solver's, not data from a run. When the solver refutes an obligation but gives no values, the counterexample has no bindings and an `unavailable` key with the solver's message, printed as `counterexample: not available (...)`: the obligation is still refuted, and the message points at a defect of the compiler, worth reporting. A binding whose declared type is labeled, such as `pin: secret Int`, also carries `"label": "secret"`; its value is still shown, since it is hypothetical. In `una test`, a package of edition `0.2` shows an example's result and failure reason with every field, since example data come from the source; edition `0.1` redacts labeled fields there, and the document's `redacted` key says which rule applies. Traces are redacted in both.

A package that establishes an opaque fact once, at its boundary, and declares the budget for it:

`una.toml`
```toml
[package]
name = "slugs"
version = "0.1.0"
edition = "0.2"

[escape_budget]
checked_per_module = 1
```

`src/slugs.una`
```una
module slugs

pub kind Slug = Text where valid_slug(it)

pub kind SlugError = | Malformed

pub predicate opaque valid_slug(t: Text) -> Bool = t.len >= 3 and not starts_with(t, prefix: "-")

--- The only way to obtain a Slug from untrusted text.
pub parse_slug(raw: Text) -> Slug
  fails SlugError
=
  checked(trim(raw), valid_slug(it)) else fail Malformed

example "a slug with a leading dash is rejected"
  do parse_slug("-x")
  expect fails Malformed
```

## 7. Failures

Errors are values of a sum kind. A function declares at most one, `fails E`, and ends with `fail V(...)`. Every statement that can fail is handled on the lines below it, or wrapped in `attempt`:

| Clause | Meaning |
| --- | --- |
| `if error fail V(...)` | any failure becomes `V`; `error` names the original |
| `if error pass` | propagate unchanged; the callee's error kind is the function's |
| `if error use e` | recover with the value `e` |
| `if none fail V(...)` | `None` becomes `V`, and the statement's value is the payload |
| `if none use e` | `None` becomes `e` |

One effect per line: a capability operation, or a call of a function that has `uses` or `fails`, is the whole expression of its statement, `let` or final expression. `attempt f(x)` turns a failure into `Err`, so the result is a `Result`; when it is `Ok`, the callee's `ensures` hold for the value.

```una
module demo

pub kind Oops = | TooLarge

pub bump(n: Int) -> Int
  ensures result > n
  fails Oops
=
  if n > 100 then fail TooLarge else n + 1

pub bump_twice(n: Int) -> Int
  fails Oops
=
  let once = bump(n)
    if error pass
  bump(once)
    if error pass

pub bump_or_same(n: Int) -> Int
  ensures result >= n
=
  let outcome = attempt bump(n)
  when outcome
    is Ok(value) -> value
    is Err -> n
```

An arm of `when` or a branch of `if` may `fail`, so a binding takes the value of the arms that do not:

```una
module demo

pub kind Unreadable = | NotANumber

--- The integer a JSON value holds.
pub number(j: Json) -> Int
  fails Unreadable
=
  let n = when j
    is Number(value) -> value
    else -> fail NotANumber
  n
```

```una
-- fails with U3007
module demo

pub kind Oops = | TooLarge

pub bump(n: Int) -> Int
  fails Oops
=
  if n > 100 then fail TooLarge else n + 1

pub bump_twice(n: Int) -> Int
  fails Oops
=
  let once = bump(n)
  bump(once)
    if error pass
```

```una
-- fails with U3013
module demo

capability Clock
  op now() -> Int

pub later() -> Int
  uses @clock: Clock[now]
=
  1 + @clock.now()
```

## 8. Capabilities

There is no ambient authority. A capability is declared with operations; a function may use one only through a `uses` clause, and a `pub` function declares all of them. Capabilities pass down by name to callees; `with` rebinds a name (`with @x = @y`) or installs a handler (`with @x = handler(...)`) for one statement. Capability references are never values: they cannot be bound with `let`, passed as arguments, returned or stored (U3005).

- Operation clauses: `requires` (a proof at each call), `ensures` (an assumption the build report lists), `fails E`, and `accepts label` (the highest label its arguments may carry).
- A handler implements every operation in Una. Its bodies are verified against the operations' `ensures`, and it may have its own `uses`.
- Host capabilities come from the runtime into `main` only, as `una run` grants them: `Console[print]`, `Clock[now]`, `Env[get]`, `Files[read, write]` and `Keys[hmac]` (`spec/14 §3`). Grants are `console`, `clock`, `env`, `keys:<keyfile>` and `files:<path>:read`, `:write` or `:read,write`, in `[run] grants` or with `--grant`.
- Their signatures (`spec/14 §3`; `Http` is in §17):
  - `print(line: Text) -> Unit`;
  - `now() -> Int`, in milliseconds since 1970;
  - `get(name: Text) -> Option[internal Text]`;
  - `read(path: Text) -> internal Text` and `write(path: Text, content: Text) -> Unit`, which fail with `FileError = NotFound | Denied | Io`;
  - `hmac(key_name: Text, data: Text) -> internal Hashed`, which fails with `KeyError = | UnknownKey`. Its result is `internal`, joined with every label of `data`. A `Hashed` supports only `==` and `!=`: compare hashes, never the text they were made from.
- **A `Files` path is relative to the grant's prefix.** With the grant `files:data:read`, `@files.read("readings.txt")` reads the file `data/readings.txt`. `@files.read("data/readings.txt")` would look for `data/data/readings.txt` and fail with `NotFound`. An absolute path, or a `..` that leaves the prefix, fails with `Denied`.
- **Stand-ins in tests.** An example cannot use a host capability, so it installs a handler with `given`. A stand-in for `Keys` makes hashes with `fake_hash(seed: Int) -> internal Hashed` from `std.testing`, where equal seeds give equal hashes. A handler of `Keys` sees `data` with every label, so it may branch on it but not pass it to `fake_hash`, as the handler `fake_keys` below does.

```una
module demo

pub kind StoreError = | Offline

pub kind CheckoutError = NotOnShelf | ShelfDown

pub kind Tool
  name: Text

pub capability Shelf
  op pick(name: Text) -> Option[Tool]
    fails StoreError

pub check_out(name: Text) -> Tool
  uses @shelf: Shelf[pick]
  fails CheckoutError
=
  let tool = @shelf.pick(name)
    if error fail ShelfDown
    if none fail NotOnShelf
  tool

--- Tries the main shelf, then the backup one.
pub check_out_anywhere(name: Text) -> Tool
  uses @main_shelf: Shelf[pick]
  uses @backup: Shelf[pick]
  fails CheckoutError
=
  let outcome = attempt check_out(name) with @shelf = @main_shelf
  when outcome
    is Ok(tool) -> tool
    is Err ->
      check_out(name) with @shelf = @backup
        if error pass

pub handler shelf_with(tools: List[Tool]) for Shelf
  op pick(name) = tools.first_where(keep: fn(t) -> t.name == name)

example "the backup shelf has what the main one lacks"
  given @main_shelf = shelf_with(tools: [])
  given @backup = shelf_with(tools: [Tool(name: "saw")])
  do check_out_anywhere("saw")
  expect result.name == "saw"
  expect performed @backup.pick
```

```una
module allowlist

import std.testing (fake_hash)

--- Whether an address is on a list kept only as HMAC hashes with the key `pepper`.
pub allowlisted(address: secret Text, hashes: List[internal Hashed]) -> secret Bool
  uses @keys: Keys[hmac]
  fails KeyError
=
  let hashed = @keys.hmac(key_name: "pepper", data: address)
    if error pass
  hashes.filter(keep: fn(h) -> h == hashed).len > 0

--- A stand-in for the key service: one known address, the others another hash.
handler fake_keys() for Keys
  op hmac(key_name, data) = if data == "ana@example.com" then fake_hash(1) else fake_hash(2)

example "a listed address is found"
  given @keys = fake_keys()
  do allowlisted("ana@example.com", hashes: [fake_hash(1)])
  expect result == true
```

```una
module demo

pub main() -> Unit
  uses @console: Console[print]
  uses @clock: Clock[now]
=
  let now = @clock.now()
  @console.print("milliseconds since 1970: {now}")
```

A function without `uses` is pure, and a `pub` function declares everything its callees use. Missing clauses come with a mechanical edit:

```json
{
  "schema": "una.diagnostics/1",
  "diagnostics": [
    {
      "schema": "una.diagnostic/1",
      "code": "U3003",
      "severity": "error",
      "domain": "authority",
      "message": "`stamp` uses `@clock: Clock[now]`, but this code declares no `uses`, so its effects would exceed its header",
      "primary": { "file": "stamp.una", "start": { "line": 13, "col": 11 }, "end": { "line": 13, "col": 18 } },
      "related": [],
      "repairs": [
        {
          "rank": 1,
          "escape_cost": 0,
          "description": "declare `uses @clock: Clock[now]` in the header",
          "edit": { "file": "stamp.una", "start": { "line": 11, "col": 19 }, "end": { "line": 11, "col": 19 }, "replacement": "\n  uses @clock: Clock[now]" }
        }
      ],
      "explain": "una explain U3003"
    }
  ]
}
```

```una
-- fails with U3003
module demo

capability Clock
  op now() -> Int

pub stamp() -> Int
  uses @clock: Clock[now]
=
  @clock.now()

pub later() -> Int
=
  let t = stamp()
  t + 1
```

A function that takes a function parameter can run it with whatever effects and failures its caller chose: `uses of p` and `fails of p` give the function the `uses` and `fails` of the argument for `p`. The prelude's `map`, `filter`, `first_where` and `fold` declare them, so they take effectful and failing functions. Inside such a function, `p` is only called, or passed on for another `of` parameter (U3006 otherwise).

```una
module demo

pub kind ParseError = | NotANumber

pub parse(t: Text) -> Int
  fails ParseError
=
  let n = to_int(t)
    if none fail NotANumber
  n

--- `apply_all` uses and fails with whatever its argument `each` does.
pub apply_all(xs: List[Text], each: fn(Text) -> Int) -> List[Int]
  uses of each
  fails of each
=
  xs.map(each)
    if error pass

pub parse_all(xs: List[Text]) -> List[Int]
  fails ParseError
=
  apply_all(xs, each: parse)
    if error pass
```

## 9. Labels and information flow

Every type may carry a label: a level, `public` (the default), `internal`, `confidential`, `secret` or `restricted`, and categories declared in `una.toml`, as in `secret[PII] Text`. Values flow up, never down: the result of an operation joins its operands' labels; branching on a labeled value raises the program counter, so what the branches produce is labeled too, and they may not fail; each operation argument is a sink bounded by its `accepts` (`Console.print` accepts `public`). `Env.get` and `Files.read` return `internal` text. A label labels one position of a type: in `internal Option[Int]` which variant the value is, `Some` or `None`, is `internal`, but the `Int` inside is `public`, so a function that returns a labeled element writes `internal Option[internal Int]`.

**Printing what you read from `Env` or `Files` takes a policy.** That text, and anything computed from it, even its length, is `internal`, so printing it is U4003, as in the second block below. No label of your program can be lowered to fix that: declassify the value through a `policy` in a trusted module, as the `journal` package after it does, and call the policy where you print. `una explain U4003` shows the same pattern for a name read from `Env`. Whether a variable is set is `public`, though, since `Env.get` returns a `public` `Option`: treat which variables are set as public configuration, and never encode a secret in it.

**Writes to a labeled path make file failures labeled.** The files a run finds depend on where earlier writes landed, in this run or an earlier one. So when a package writes to an `internal` path, or writes under an `internal` condition, the failure of every `Files.read` and `Files.write` in it is `internal`: handle it with `attempt` or `if error use`, not `if error pass` (U4002). A package whose writes go to public paths, at a public program counter, keeps public failures (S-14).

```una
module demo

pub kind Note
  day: Int
  body: secret Text

pub kind Mood = Calm | Busy

--- The answer depends on the text, so it is as secret as the text.
pub mood(note: Note) -> secret Mood = if note.body.len > 280 then Busy else Calm
```

```una
-- fails with U4003
module demo

pub main() -> Unit
  uses @console: Console[print]
  uses @env: Env[get]
=
  let name = @env.get("USER")
    if none use "nobody"
  @console.print("hello {name}")
```

Labels go down only through a `policy` in a module that its header and `una.toml` both mark trusted. The policy is the only entry to its private `via` function, and every call of it is listed in the build report with its purpose and owner:

`una.toml`
```toml
[package]
name = "journal"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["Private"]

[trusted]
modules = ["release"]
```

`src/release.una`
```una
trusted module release

pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how much was written without showing what"
  owner journal_owner

hundreds(text: confidential[Private] Text) -> public Nat = text.len / 100
```

`src/show.una`
```una
module show

import release (length_band)

pub size(text: confidential[Private] Text) -> Nat = length_band(text)
```

A generic function may decide something by comparing values of a type parameter. It must not hand the decided value to a function value it was given, because that function value was checked only against values of `T` passed through, and its result, effects or failures would reveal the comparison. The checker rejects the call where the decided value meets the function value, here at `each: k`, whether a prelude function such as `map`, `filter`, `first_where` or `fold` calls it or a generic function of the program does (U4001):

```una
-- fails with U4001
module demo

pub capability Log
  op write(n: Int) -> Unit

count_chosen[T: Eq](a: T, b: T, p: T, q: T, k: fn(T) -> Int) -> Unit
  uses @log: Log[write]
=
  let ns = [if a == b then p else q].map(each: k)
  @log.write(ns.sum())
```

Pass the values of `T` through unchanged (`[p, q].map(each: k)` is accepted), or give the decision to the caller as a `Bool` result. Edition `0.1` reports the same U4001 at `each: k`, and also the code where the decision would come out, U4003 at the sink or U4002 at a call with effects (#346, #350, #354). A written annotation that mentions `T` cannot hold such a value either; see section 13.

## 10. Requirements and examples

A requirement records a statement with its source and owner; `derived from` traces functions to it, and `una trace <ID>` lists them with their obligations. A `generated` requirement stays `proposed` until a person changes its source.

Examples are executable evidence next to the code: `given` installs handlers (never host capabilities), `do` runs one expression, and each `expect` checks `result`, `fails V`, `performed @x.op` or `not performed @x.op`. `una test` runs them with their effect traces. A `do` line whose call fails is checked with `expect fails V`, as `parse_slug` in §6 shows; `do attempt f(x)` with `expect result == Err(V)` checks the same through the `Result`.

```una
module demo

requirement SEATS_1
  text "An event with no seats left is shown as sold out"
  source human
  owner box_office
  status active

pub kind Availability = SoldOut | Open

pub availability(remaining: Int) -> Availability
  ensures remaining < 1 implies result is SoldOut
  derived from SEATS_1
=
  if remaining < 1 then SoldOut else Open

example "no seats left is sold out"
  do availability(0)
  expect result is SoldOut
```

## 11. Editable values and loops

Values stay immutable and there are no references; what changes is which value a binding holds (`spec/17`).

- `let edit x = e` declares an editable binding, and `x = e` or `x.field = e` rebinds it. Assigning anything else is U2019. An editable binding keeps its declared type, refinement and label at every rebind.
- Without an annotation, the positions that its initial value leaves open, as the key and value of `empty_map()` or the element of `[]`, take the labels of everything assigned there, so a generic `tally[T: Ord](xs: List[T]) -> Map[T, Int]` built with `let edit m = empty_map()` checks, and its counts carry the labels of `T` (ADR 0075). The binding's own top label stays the one its initial value gives it.
- A parameter declared `edit p: T` is passed in and out: the caller writes `edit place` and gets the final value back; `old(p)` in `ensures` is its value on entry. `old` takes the parameter itself, so write `old(xs).len`, not `old(xs.len)` (U2012). A method receiver is marked before it: `edit tally.record(amount)`.
- A parameter declared `take p: T` consumes its argument: the caller writes `take x`, and `x` cannot be used afterwards (U2017) until it is assigned again.
- Markers match declarations exactly (U2020); an `edit` argument and another use of the same binding in one call is U2018; a function with `edit` or `take` parameters cannot be used as a value (U2021). A call with a marker is the whole expression of its statement.
- `for x in xs` visits a list in order, and `for i in a..b` or `a..<b` the integers of a range; the bounds are evaluated once, before the first iteration. The loop variable has the element type, or the integer type of the bounds. There is no `break` or `continue`. The refinements of the bindings a loop updates are its invariant; no other fact about them survives the loop.
- `if c` with an indented block and no `else` is a statement. `edit xs.set(index: i, item: v)` replaces a list element.
- A loop keeps the length of a list it changes only with `set`, or with a helper whose `ensures` is exactly `xs.len == old(xs).len`, so `for i in 0..<xs.len` with `edit xs.set(index: i, item: v)` verifies. After an `append` or any other change, the loop forgets the length, and a check that needs it is U5002, "the loop forgets every fact about the list `xs` ...": guard it with `if i < xs.len`.
- **A contract about what a loop builds goes on the helper one iteration calls.** The loop forgets every fact about the bindings it updates but their refinements, so an `ensures` that needs one after the loop, such as `counts.has(key: vote)` after a loop of `counts.set`, is U5002, and neither an `if` guard nor `checked` proves it. Move the update into a helper with an `edit` parameter and give the helper the contract, as `add_vote` below: the solver proves it once, for every call, and a caller has it right after each call. The helper's `ensures` names the parameter's value on entry as `old(p)`, as `counts.len <= old(counts).len + 1`. The loop's own `ensures` about everything it built, such as `result.len <= votes.len`, still cannot be proven, since the loop keeps no fact from one iteration to the next: state what callers need on the helper.
- A call that fails leaves its `edit` arguments unchanged for the caller; operations it performed stay performed.
- **Cost.** A list, map or text that nothing else holds is updated in place, so `acc = acc.append(item: x)` in a loop, or through helpers that take `acc` and return it, is linear (ADR 0072).
- **Known limit: keeping the old value on failure.** `acc = step(acc, item: x)` followed by `if error use acc` reads `acc` after the call, so the call cannot take it over and copies it: every iteration copies the whole list, and the loop is quadratic. The old value is kept because a failed call changes nothing for its caller (`spec/17`). For a linear loop, check first and then update, or make the update unable to fail and return a status with the new value.

```una
module demo

pub kind Tally
  count: Int
  total: Int

--- An `edit` parameter goes in and comes back out; `old(t)` is its value on entry.
pub record(edit t: Tally, amount: Int) -> Unit
  ensures t.count == old(t).count + 1
=
  t.count = t.count + 1
  t.total = t.total + amount

pub summarize(amounts: List[Int]) -> Tally
=
  let edit tally = Tally(count: 0, total: 0)
  for amount in amounts
    edit tally.record(amount)
  tally

pub sum_to(n: Nat) -> Int
=
  let edit total = 0
  for i in 1..n
    total = total + i
  total

--- `set` keeps the length, so every index of the range stays valid (ADR 0071).
pub zeroed(xs: List[Int]) -> List[Int]
  ensures result.len == xs.len
=
  let edit ys = xs
  for i in 0..<ys.len
    edit ys.set(index: i, item: 0)
  ys

pub cleared(xs: List[Int], index: Int) -> List[Int]
  requires 0 <= index and index < xs.len
  ensures result.len == xs.len
=
  let edit copy = xs
  edit copy.set(index: index, item: 0)
  copy
```

```una
module tally

--- One vote counted. The key is in the map afterwards: a fact a loop would forget, which
--- each call of the helper keeps.
add_vote(edit counts: Map[Text, Int], vote: Text) -> Unit
  ensures counts.has(key: vote)
=
  let n = counts.get(key: vote)
    if none use 0
  edit counts.set(key: vote, value: n + 1)

--- The votes counted, one call of the helper per vote.
pub tally(votes: List[Text]) -> Map[Text, Int]
=
  let edit counts: Map[Text, Int] = empty_map()
  for vote in votes
    add_vote(edit counts, vote)
  counts

--- A vote counted on top of the others: its key is there, by the helper's contract.
pub with_vote(votes: List[Text], vote: Text) -> Map[Text, Int]
  ensures result.has(key: vote)
=
  let edit counts = tally(votes)
  add_vote(edit counts, vote)
  counts
```

```una
module demo

pub kind Limit = | Full

pub kind Added
  items: List[Int]
  full: Bool

--- Fails when the list is full.
pub bounded(acc: List[Int], item: Int) -> List[Int]
  fails Limit
=
  if acc.len >= 100
    fail Full
  acc.append(item: item)

--- Quadratic: `if error use acc` reads `acc` after the call, so each call copies it.
pub kept(items: List[Int]) -> List[Int]
=
  let edit acc: List[Int] = []
  for item in items
    acc = bounded(acc, item: item)
      if error use acc
  acc

--- Linear: check first, then update.
pub checked_first(items: List[Int]) -> List[Int]
=
  let edit acc: List[Int] = []
  for item in items
    if acc.len < 100
      acc = acc.append(item: item)
  acc

--- Never fails: the new list, and whether it was full.
pub add(acc: List[Int], item: Int) -> Added
=
  if acc.len >= 100
    Added(items: acc, full: true)
  else
    Added(items: acc.append(item: item), full: false)

--- Linear: the update cannot fail, and returns its status with the new value.
pub with_status(items: List[Int]) -> List[Int]
=
  let edit acc: List[Int] = []
  for item in items
    let added = add(acc, item: item)
    acc = added.items
  acc

example "the three keep the first 100 items"
  do kept([1, 2, 3]) == checked_first([1, 2, 3]) and checked_first([1, 2, 3]) == with_status([1, 2, 3])
  expect result == true
```

```una
-- fails with U2019
module demo

pub count_up(n: Int) -> Int
=
  let total = 0
  total = n
  total
```

## 12. Fixed-width integers

`Int` stays unbounded. `I8`, `I16`, `I32`, `I64`, `U8`, `U16`, `U32` and `U64` exist where the width matters (`spec/18`).

- A literal takes the fixed-width type its context expects, including inside a constructor or a list (`Some(3)` against `Option[U8]`); otherwise it is an `Int`. A literal that does not fit is U2022.
- Operands of an operator have one integer type; there is no implicit conversion (U2001). Each `+`, `-`, `*`, `/`, `mod` and unary `-` on a fixed-width type, and each narrowing conversion, is an `overflow` obligation: the exact result fits. A refuted one is U5001 with a counterexample.
- Conversions are methods: `to_i8()` to `to_u64()` and `to_int()`. Widening always succeeds; narrowing carries an `overflow` obligation.
- Wrapping, saturating and bit arithmetic exist only by name: `wrapping_add(by: b)`, `saturating_sub(by: b)`, `bit_and(mask: m)`, `shift_right(by: n)` and the others of `spec/18 §5`.
- Clauses (`where`, `requires`, `ensures`) and the bodies of `reflect` predicates compute exactly, with no overflow obligation: `ensures result <= 300` on a `U8` result is proven.

```una
module demo

--- A literal takes the type it is checked against: `Some(3)` here is an `Option[U8]`.
pub default_level() -> Option[U8] = Some(3)

--- The guard on `Int` proves that `a + b` fits a `U8`.
pub add_bytes(a: U8, b: U8) -> U8 = if a.to_int() + b.to_int() <= 255 then a + b else 255

--- Wrapping arithmetic only by name.
pub hash_step(h: U32, byte: U8) -> U32 = h.wrapping_mul(by: 31).wrapping_add(by: byte.to_u32())

--- A `reflect` body computes exactly, with no overflow obligation.
pub predicate reflect large(x: U32) -> Bool = x * 4294967295 > 4294967295
```

```una
-- fails with U5001
module demo

pub add(a: U8, b: U8) -> U8 = a + b
```

## 13. Constraints on generics

A type parameter of a function may carry one built-in constraint (`spec/20`): `Eq` allows `==` and `!=`, `Ord` also allows `<`, `<=`, `>` and `>=`, and `Integer` also allows `x.to_int()`. Arithmetic on a type parameter stays undefined. An operator the constraint does not allow is U2026, with a repair that writes the constraint; a type argument that does not satisfy it is U2025. Private functions get the least constraint their body needs; public ones write it. Kinds take no constraints.

```una
module demo

pub largest[T: Ord](xs: List[T], floor: T) -> T = xs.fold(initial: floor, step: fn(best, x) -> max(best, b: x))

pub total[T: Integer](xs: List[T]) -> Int = xs.map(each: fn(x) -> x.to_int()).sum()

pub tidy(names: List[Text]) -> List[Text] = names.sort()
```

```una
-- fails with U2026
module demo

pub same[T](a: T, b: T) -> Bool = a == b
```

A generic function can build a structure by comparing values. Lists built by comparisons carry `T`'s labels at their top, so combine them with the prelude (`append`, `fold`), whose parameters take labels of any kind (ADR 0067):

```una
module demo

pub insert[T: Ord](sorted: List[T], item: T) -> List[T]
=
  let ahead = sorted.filter(keep: fn(s) -> s < item)
  let behind = sorted.filter(keep: fn(s) -> not (s < item))
  behind.fold(initial: ahead.append(item: item), step: fn(acc, s) -> acc.append(item: s))
```

Known limits of edition `0.2`, which need label polymorphism (#40):
- Passing such a list to a parameter of your own generic function, as `concat(ahead.append(item: item), back: behind)`, is U4001: the parameter's top label is public.
- A `fold` whose step returns such a list, as an insertion sort built from `insert`, is U4001: the accumulator's label rises after the step was checked. Use `sort`.

A value chosen by comparing values of `T` carries that decision in its label, and a written annotation that mentions `T` cannot hold it: `let picked: T = if a == b then p else q` is U4001, and so are `List[T]` and a kind applied to `T`. Leave the annotation out. Inference keeps the decision, and each call raises its result by the type argument's label. The return type `-> T` is fine, and both editions apply the same rule (#354):

```una
module demo

pub choose[T: Eq](a: T, b: T, p: T, q: T) -> T
=
  let picked = if a == b then p else q
  picked
```

```una
-- edition 0.1
-- fails with U4001
module demo

pub choose[T](a: T, b: T, p: T, q: T) -> T
=
  let picked: T = if a == b then p else q
  picked
```

## 14. Maps

`Map[K, V]` maps keys to values, in ascending key order (`spec/21`). It is a value, as a list is.

- Keys satisfy `Ord`: `Int`, the fixed-width types, `Text`, or a sum whose variants have no payload. Any other key type is U2025, and a public generic function writes `[K: Ord]`.
- `empty_map()` takes its type from its context, as `[]` does: `let edit m: Map[Text, Int] = empty_map()`.
- Reads: `m.len`, `m.has(key: k)`, `m.get(key: k)`, which is `None` for a missing key and is handled with `if none`, and `m.keys()`, `m.values()` and `m.entries()`, lists in key order. An `Entry` has the fields `key` and `value`.
- Updates: `edit m.set(key: k, value: v)` and `edit m.remove(key: k)` on an editable binding, each the whole statement. `m.with(key: k, value: v)` and `m.without(key: k)` give a new map instead.
- Cost: `len` is O(1); `get`, `has`, `set` and `remove` are O(log n), in place when nothing else holds the map; `with` and `without` on a map another binding still holds copy it, O(n).
- Proofs: `m.len` and `m.has(key: k)` may appear in contracts and `where` clauses. After `set`, the key is present with its value; after `remove`, it is absent. Nothing is known of the other keys.
- Labels: which keys a map holds is part of its shape, so `len`, `has`, `get` and the listings carry the labels of the keys with the map's top label. Setting a `secret` key in a map whose top label is public is U4001 at the key: declare it `secret Map[Text, Int]`.
- Type arguments are never refined: a refined kind such as `Nat`, or `Int in 0..5`, cannot instantiate a type parameter, so `Map[Text, Nat]` and `List[Nat]` are U2002, as is any generic kind given one. Write the base type, `Map[Text, Int]`, and refine where a value is used: a function that returns a count as `-> Nat` checks it there, for instance with `if n >= 0 then n else 0`. The values a map holds are then any `Int`, so a contract about them needs that check too.
- A map prints as `{"a": 1, "b": 2}`, and `{}` when empty.

```una
module words

--- How often each word occurs.
pub counts(words: List[Text]) -> Map[Text, Int]
=
  let edit counts: Map[Text, Int] = empty_map()
  for word in words
    let n = counts.get(key: word)
      if none use 0
    edit counts.set(key: word, value: n + 1)
  counts

--- The words seen at least `least` times, in ascending order.
pub frequent(counts: Map[Text, Int], least: Int) -> List[Text]
=
  let kept = counts.entries().filter(keep: fn(e) -> e.value >= least)
  kept.map(each: fn(e) -> e.key)

--- The count of a word the map has.
pub count_of(counts: Map[Text, Int], word: Text) -> Int
  requires counts.has(key: word)
=
  let n = counts.get(key: word)
    if none use 0
  n
```

```una
-- fails with U2002
module demo

pub empty_counts() -> Map[Text, Nat] = empty_map()
```

## 15. JSON

`parse_json`, `to_json` and `quote_json` read and write JSON (`spec/23`, edition 0.2).

- **The value** is the prelude kind `Json`: `Null`, `Boolean(value)`, `Number(value: Int)` for an exact integer, `Decimal(text)` for any other number as written, `Str(value)`, `Array(items: List[Json])` and `Object(fields: Map[Text, Json])`, whose keys are in ascending order.
- **Parsing.** `t.parse_json()` fails with `JsonError`: `Syntax(at)`, `TooDeep(at)` past 512 levels, `DuplicateKey(at)`, or `NumberTooLong(at)` for an integer over 4300 digits. `at` counts scalar values. Handle it with `if error`, or `attempt` for the `Result`.
- **Writing.** `j.to_json()` writes the canonical compact form, keys in order, and an integer of any size in full, though parsing reads back at most 4300 digits. `t.quote_json()` writes one JSON string, for building JSON text by interpolation.
- **Decimals.** A `Decimal` holds a `JsonNumber`, a number with a fraction or an exponent, which parsing makes, or `checked(t, is_json_number(it))`.
- **Labels.** A value parsed from a labeled text, its fields, and whether and where the parse failed, all carry the text's labels. In edition 0.4 a constructor takes labeled data and its result carries their join: `Str(value: token)` with a `secret` token is a `secret Json`, its `to_json()` a `secret Text`, and every value a `when` binds from it is `secret`; it is U4001 or U4003 only where it reaches a lower declared type or a sink. In editions 0.2 and 0.3 a `secret` cannot enter a `Json` constructor (U4001): build the text with `quote_json` and interpolation instead, and its result is `secret`.
- **Cost.** Parsing is O(n + Σ k log k), and O(n) when keys arrive in order. Writing is O(n). Neither recurses on depth.
- **Parse, then branch.** After `attempt` on a labeled text, the branches of `when r` run under the text's label, so a call that can fail inside them is U4002. Give the parse a default with `if error use`, then branch on the value, and make fallible calls outside the branches:

```una
module config

--- The port of a configuration, or 8080 when it is missing or not a number.
pub port(text: internal Text) -> internal Int
=
  let doc = text.parse_json()
    if error use Null
  when doc
    is Object(fields) ->
      let value = fields.get(key: "port")
        if none use Null
      when value
        is Number(n) -> n
        else -> 8080
    else -> 8080

--- The configuration written back, with the port replaced.
pub with_port(fields: Map[Text, Json], port: Int) -> Text = Object(fields.with(key: "port", value: Number(port))).to_json()
```

## 16. Text

Edition `0.2` adds functions that take a text apart, search it, change it and parse it (`spec/22`). Each is a prelude function called with method syntax.

- **Positions** count Unicode scalar values from 0, as `.len` does; there are no byte offsets or graphemes.
- **The functions:**
  - `split(on:)`, which keeps empty parts, and `join(separator:)`, its inverse.
  - `find(text:)`, `Some` of the first position or `None`, which is `Some` exactly when `contains_text(part:)` holds; the position and the match lie in the text, so it bounds a `slice`. `ends_with(suffix:)`.
  - `replace(pattern:, by:)`, every non-overlapping occurrence, left to right.
  - `slice(start:, end:)`, whose precondition `0 <= start and start <= end and end <= t.len` you prove at each call.
  - `chars()`, one text per scalar value; `bytes()`, the UTF-8 encoding as `List[U8]`.
  - `to_upper()` and `to_lower()`, which change ASCII letters only and keep the length.
  - `trim_start()` and `trim_end()`, as `trim` at one end.
  - `parse_int()`, which fails with `ParseError`: `Empty`, `NoDigits` for `-` alone, `NotADigit(at: i)` at the first wrong position, or `TooLong` for more than 4300 digits. Handle it as any failure. The prelude's `to_int(t)` gives an `Option[Int]` instead, `None` for any text that is not an integer or has more than 4300 digits.
- **Cost:** `find`, `split` and `replace` are linear in the text; `.len` is O(1).
- **Proofs:** the contracts are hypotheses after each call, such as `split`'s `result.len >= 1` and `slice`'s `result.len == end - start`. There is no theory of strings beyond them.
- **Labels:** every result carries every label of every argument, including the top and the elements of `split`'s and `chars`' lists. Whether a secret parses is secret too.

```una
module csv

--- The fields of a line, without the spaces around them.
pub fields(line: Text) -> List[Text]
=
  let comma = ","
  let parts = line.split(on: comma)
  parts.map(each: fn(part) -> part.trim())

--- The number at the start of a field, or 0.
pub leading_number(field: Text) -> Int
=
  let space = " "
  let end = field.find(text: space)
    if none use field.len
  let digits = field.slice(start: 0, end: end)
  let n = digits.parse_int()
    if error use 0
  n
```

## 17. HTTP

The host capability `Http` sends HTTP/1.1 requests (`spec/24`, edition 0.2).

- **Operations.** One per method, so `uses @http: Http[get]` can never post:
  - `get(url: Text, headers: List[Header], key_name: Option[Text]) -> internal Response`;
  - `post(url: Text, headers: List[Header], body: Text, key_name: Option[Text]) -> internal Response`;
  - `put`, with the parameters of `post`;
  - `delete`, with the parameters of `get`.

  Each fails with `HttpError` and accepts `internal`. `Header` has `name: Text` and `value: internal Text`, and `Response` has `status: Int`, `headers: List[Header]` and `body: Text`.
- **Statuses are not failures.** A 404 is a `Response`. `HttpError` is for requests that got no response: `Denied`, `BadUrl`, `BadRequest`, `Unreachable`, `TimedOut`, `TooLarge`, `Tls` or `Protocol`.
- **Grants.** `http:<origin>` or `http:<origin>:<methods>`, by default `get`: `http:https://api.example.com:get,post`. A request outside them fails with `Denied`, before any connection. Plain `http` only where a grant says so.
- **Credentials.** `key_name: Some("name")` sends the key `name` of the run's `keys:` grant as `Authorization: Bearer <key>`, to that origin only. The program never sees it, and no trace shows it. An API that wants its token in another header, as `X-Api-Token`, takes it as a header value: a `secret` token needs a policy to `internal` first (§9).
- **Labels.** Arguments accept `internal` data: a `secret` URL, body or header value needs a policy first. Responses are `internal`, so a function that returns what it read declares an `internal` result, and its `Option` or `Result` too, since which one it returns depends on the response.
- **Failures are `internal`.** The server receives the URL, the headers with their values and the body, and may decide whether the request fails by them, so the failure of every request is `internal`, as its response is (S-15). Handle it with `attempt` and return a labeled value: `if error pass` or `if error fail` is U4002.
- **Header values.** A header value with a control character other than a tab, such as a newline, aborts the run with U6004, since the value is `internal`. Remove them from values built from data first: `value.replace(pattern: "\n", by: "")`.
- **Limits.** `--max-response <MiB>` (default 16) and `--http-timeout <seconds>` (default 30, the total time of a request). Tests give `Http` a handler, since examples cannot use it. As any handler, it implements every operation of the capability, all four (U3011), as `fixed_answer` below.

```una
module weather

--- The temperature of a city, from a JSON API, or `None` when the request or its answer fails.
--- The token is the key `weather` of the run's `keys:` grant; the program never sees it.
pub temperature(city: Text) -> internal Option[internal Int]
  uses @http: Http[get]
=
  let accept = Header(name: "Accept", value: "application/json")
  let r = attempt @http.get("https://api.example.com/v1/now?city={city}", headers: [accept], key_name: Some("weather"))
  when r
    is Ok(response) -> if response.status == 200 then degrees(response.body) else None
    is Err(e) -> None

--- The `degrees` field of a JSON object, when it is an integer.
degrees(body: internal Text) -> internal Option[internal Int]
=
  let doc = body.parse_json()
    if error use Null
  when doc
    is Object(fields) ->
      let value = fields.get(key: "degrees")
        if none use Null
      when value
        is Number(n) -> Some(n)
        else -> None
    else -> None
```

A test of a function that uses `Http` installs a stand-in with `given`. Its parameter is named `answer`, not `body`, since an operation's parameter of the same name would clash with it (U1002):

```una
module stand_in

--- The status of a GET, or `None` when the request gets no response.
pub status_of(url: Text) -> internal Option[internal Int]
  uses @http: Http[get]
=
  let r = attempt @http.get(url, headers: [], key_name: None)
  when r
    is Ok(response) -> Some(response.status)
    is Err(e) -> None

--- A stand-in for an API: every GET answers 200 with `answer`, and the other methods 405.
pub handler fixed_answer(answer: Text) for Http
  op get(url, headers, key_name) = Response(status: 200, headers: [], body: answer)
  op post(url, headers, body, key_name) = Response(status: 405, headers: [], body: "")
  op put(url, headers, body, key_name) = Response(status: 405, headers: [], body: "")
  op delete(url, headers, key_name) = Response(status: 405, headers: [], body: "")

example "the stand-in answers every GET"
  given @http = fixed_answer(answer: "ok")
  do status_of("https://api.example.com/v1/now")
  expect result == Some(200)
```

## 18. Concurrent tasks: `together`

`together` runs independent work at once with the result and trace of a sequential run (`spec/19`).

- A `together` statement holds two or more `let` branches (U2024 otherwise). Each branch runs as a task, sees the bindings before the block but not its siblings, and its binding is in scope after the block. Each branch's handling clauses stand on their own lines, as for any `let`.
- `let ys = together for x in xs` runs its indented body once per element, or per integer of a range, and builds the list of results in order.
- A task cannot assign, `edit` or `take` a binding declared outside it (U2023): return new values instead. Tasks use the capabilities of the enclosing function, and a parameter named by `uses of` or `fails of`.
- When a task fails, the tasks after it are canceled, and the block fails with the failure of the first task that failed: the outcome of running the tasks one after another.
- The trace lists each task's events in task order, with the task's path, as `"task": "2"`; the order in which tasks really performed effects, such as the lines they print, may differ.

```una
module demo

pub kind ReadError = | Unreachable

pub capability Sensor
  op reading() -> Int
    fails ReadError

pub warmest() -> Int
  uses @north: Sensor[reading]
  uses @south: Sensor[reading]
  fails ReadError
=
  together
    let n = @north.reading()
      if error pass
    let s = @south.reading()
      if error pass
  max(n, b: s)

pub squares(n: Int) -> List[Int]
=
  let squares = together for i in 0..<n
    i * i
  squares
```

```una
-- fails with U2023
module demo

pub count(xs: List[Int]) -> Int
=
  let edit seen = 0
  let doubled = together for x in xs
    seen = seen + 1
    x * 2
  seen
```

## 19. Native executables and the language server

- `una build [path]` checks the package at tier 1, then writes a native executable to `target/una/<name>` (`spec/16`), or to `--out <file>`. It refuses a package with errors, as `una run` does; `--json` prints `una.build/1`.
- The executable runs `main` of the entry module with the output, failures, exit codes and grants of `una run`: the `[run] grants` of `una.toml` are built in, and `--grant`, `--max-depth` and `--max-memory` work as for `una run`. Its `--trace <file>` writes the trace as a JSON array, the trace that `una --json run` reports in `una.run/1`. That its output and trace match the interpreter's is tested, not proven: the toolchain's tests run the runnable programs in both engines and compare them. With tasks on the real clock the order of the effects of different tasks may differ; `--virtual-time` makes the executable follow the interpreter's schedule (`spec/27 §8` rules 3 and 4).
- Tasks of `together` run on `--workers` threads, one per core by default, in a native executable, and one at a time in `una run`.
- `una lsp` is a language server (LSP 3.17 over stdin and stdout) for editors: it checks as you type, proves a file when it is opened or saved, formats on request, and never runs programs.

## 20. The Agent ABI

Every command but `una lsp` takes `--json` and prints one document whose `schema` names its shape:

| Command | Document |
| --- | --- |
| `una check`, `una fmt --check` | `una.diagnostics/1` |
| `una fix` | `una.fix/1`, the edits applied and skipped, and the diagnostics that remain (v0.2) |
| `una obligations` | `una.obligations/1`, every obligation with `status`, `goal` and `hypotheses` |
| `una test` | `una.test/1`, every example with its outcome and effect trace; `una.test/2` for a package of edition `0.3` or later, the same document with the key `tests` |
| `una run` | `una.run/1`, the program's output and trace |
| `una build` | `una.build/1`, the native executable's path, or the diagnostics that rejected the package (v0.2, `spec/16`) |
| `una contract <module.function>` | `una.contract/1`, the header, declared or inferred |
| `una trace <REQUIREMENT_ID>` | `una.trace/1`, derived functions and their obligations |
| `una report` | `una.report/2`, proofs, authority, policy calls, escapes, assumptions, host premises, trusted code, requirements, coverage, native target |
| `una explain <code>` | `una.explain/1` |
| `una core` | `una.core/1`, the package's Core IR (ADR 0083) |
| `una migrate` | `una.migrate/1`, the files written for edition `0.4` and the diagnostics (`spec/30 §8`) |
| `una ita <command>` | `una.ita/1`, the command, the files written and the diagnostics (`spec/30 §6`) |

When the checks reject a package, `contract`, `trace`, `obligations` and `report` print the `una.diagnostics/1` of `una check`. Diagnostics are sorted by file, position and code; outputs are identical from run to run.

```json
{
  "schema": "una.obligations/1",
  "obligations": [
    {
      "id": "ob-17058da2cfca",
      "kind": "postcondition",
      "domain": "semantic",
      "status": "refuted",
      "source": { "file": "double.una", "start": { "line": 4, "col": 11 }, "end": { "line": 4, "col": 21 } },
      "declaration": "demo.double",
      "goal": "result > x",
      "hypotheses": [],
      "cached": true
    }
  ]
}
```

The build report counts trivial `ensures`, clauses the solver proves with no hypothesis at all such as `ensures result == result`, as missing contracts: write contracts that say something about the result.

## 21. Migrating from edition 0.1

Change `edition = "0.1"` to `"0.2"` in `una.toml`, then check. What can change:

- **New keywords.** `edit`, `take`, `old` and `together` are keywords, and `for` starts a loop: rename bindings that use them.
- **Messages.** `var`, `while`, `loop`, `async` and `await` point to `let edit`, `for` and `together`.
- **Equality on type parameters** needs `Eq`: a public generic function that compares values of `T` writes `[T: Eq]` (U2026). Edition `0.1` inferred the bound.
- **The prelude.** `min`, `max`, `contains` and `sum` are generic over constrained types, `sort`, `set`, `empty_map` and the type `Map` are new (a user kind named `Map` or `Entry` must be renamed), and `map`, `filter`, `first_where` and `fold` take effectful and failing functions (`uses of`, `fails of`).
- **Kinds.** Recursive kinds no longer need a `List` or `Option` field, but must have a finite value; `I8` to `U64` become built-in types, and a kind, variant or capability named like any built-in type, `Int`, `Nat`, `Bool`, `Text`, `Unit`, `Hashed`, `List`, `Option`, `Result`, `Map` or `I8` to `U64`, is U1002 (ADR 0079). A package that declares its own `kind Result` renames it before it moves to edition `0.2`.
- **Integers.** Values of a fixed-width type now carry overflow obligations; `Int` code is unchanged.

The same module can stay in edition `0.1`, where for example `type` keeps its hint and `edit` is still reserved:

```una
-- edition 0.1
-- fails with U0103
module demo

pub change(edit: Int) -> Int = edit
```

## 22. Migrating to edition 0.4

From edition `0.4`, a package is described by `ita.una` and pinned by `ita.lock`, in Una's own syntax, in place of `una.toml` and `verification.lock` (`spec/30`). Edition `0.4` is a preview. Collections and the numeric types after `F64` are in edition `0.5`, also a preview and also named in `ita.una`, as `edition 0.5` (ADR 0139); `una migrate` writes edition `0.4`, or keeps `0.5` when `una.toml` names it. The steps, in the package's directory:

1. `una migrate --dry-run` prints the two files it would write and every diagnostic the new edition causes, and writes nothing.
2. `una migrate` writes `ita.una` and `ita.lock`, rewrites each module in the canonical form of edition `0.4`, removes `una.toml` and `verification.lock`, and prints the same diagnostics, such as a function named `within`, a word edition `0.4` reserves (U0103). It exits with code 0: the diagnostics are the next steps. A `[package] name` that is not a package name, such as `events-example`, becomes `events_example`; `--name` gives another.
3. `una fix` applies the mechanical repairs, such as renaming `within`, and `una check` checks the result.
4. Commit `ita.una`, `ita.lock` and the rewritten modules together.

What changes:

- **The manifest.** `ita <name>` and its `version` and `edition` lines head the file; the tables of `una.toml` become sections with one entry per line, and a grant is a line such as `files "data/" read`. `edition = "0.4"` in `una.toml` is U7002, with `una migrate` as its repair.
- **Dependencies.** A package may need another one from a directory, `util from path "../util"` with `accept name`, or from a registry its `sources` section declares and pins, `acme/ledger 0.3 from acme`, which this release reads only from a mirror directory that Itaoca's `mirrors.una` lists as a `file://` URL, since it has no https transport, and import its modules as `util.text` or `ledger.ledger`. `una ita lock` resolves and writes the lock; `una ita add` and `una ita update` change the needs and lock. A need of `itaty`, the default registry, has no resolution in this release (U7013).
- **Authority.** A dependency receives a capability only at a call, with `with`, within the `may` lines its need writes, and `una ita lock --accept-authority <package>` records each wider ceiling. A dependency's trusted modules count only with a `trust` line and `--accept-trust <package>@<digest>`.
- **Offline.** `una check`, `una run`, `una build` and `una test` never read a registry: `una ita fetch` fills Itaoca, in this release from a `file://` mirror directory only, `una ita vendor` writes `vendor/ita/` to commit, and `una ita bundle` and `una ita unbundle` carry the packages to a machine with no network. A locked package that is not there is U7011; one whose files changed is U7010.
- **Never by hand.** `ita.lock` is written by `una ita lock`; `una fix` never edits `ita.una` or `ita.lock`. CI runs `una ita lock --check --fresh` and `una ita audit`.

A package of edition `0.4`, with its manifest, its lock and one module:

`ita.una`

```una
ita demo
  version 0.1.0
  edition 0.4
```

`ita.lock`

```una
lock 1

solver
  encoding "una-smt-1"
  name "z3"
  version "5.1.0"
  logic "QF_UFLIA"
  steps 5000000
  seed 0
```

`src/demo.una`

```una
module demo

pub total(xs: List[Int]) -> Int
=
  let edit sum = 0
  for x in xs
    sum = sum + x
  sum
```

## 23. Changes between releases

A release can reject a program that an earlier one accepted, where the earlier answer was not sound. Such changes are listed here, newest first.

- **`result`, `it` and `error` inside a lambda** (#1989). A lambda's body runs wherever the lambda is called, so `result`, `it` and `error` name no value in it, even when the lambda is written in an `expect` line, a `where` clause, the predicate of `checked` or an `if error` handler (`spec/02 §8`). Before v0.4.0, a test whose `expect` line used `result` inside a lambda was accepted, and the lambda saw a placeholder in place of the result; v0.4.0 and v0.5.0 reject it with an internal error, U8001, and v0.5.1 and later with U2012 at the `result` (U2013 for `it` and `error`). The same holds for a lambda given to `map`, `filter`, `fold`, `first_where` or `update`, or for a `uses of` or `fails of` parameter. The fix: pass the value into the lambda as an argument, `expect (fn(x: Int) -> x + 1)(result) == 2`, or write the check without a lambda.

```una
-- fails with U2012
module demo

pub one() -> Int = 1

example "the lambda cannot see the result"
  do one()
  expect (fn(x: Int) -> x == result)(1)
```
