# Una for agents

A compact reference for writing Una with the v0.1 toolchain. The specification in `docs/spec/` is normative; this guide summarizes it for an agent that writes, checks and repairs code.

How to read the blocks: every `una` block is a whole module that `una check` accepts, unless its first line is `-- fails with U####`, in which case checking it yields that code and no other. A block preceded by a file path belongs to a package with the other files named around it. Every `json` block is a document the CLI prints, and every `sh` block runs from the repository root. `crates/una-cli/tests/guide.rs` checks all of them.

## 1. The loop

1. Write or edit `.una` files.
2. Check: `una --json check <path>`. Exit code 0 means the package parses, is canonical, types, respects authority and information flow, and every proof obligation is proven by the solver.
3. Otherwise read the diagnostics. Each has a `code`, a one-line `message`, a `primary` position and often ranked `repairs`; a repair with an `edit` is mechanical: replace the range with `replacement`.
4. When unsure, `una explain <code>` prints the rule with a failing and a fixed example.
5. Run the examples with `una --json test <path>`, list every proof with `una --json obligations <path>`, and read the build report with `una --json report <path>`.

```sh
una check examples/events
una --json check examples/toolshelf
una fmt --check examples/pantry
una --json test examples/journal
una --json obligations examples/greenhouse
una --json report examples/shortlinks
una explain U3013
una --json explain U5001
una check tests/conformance/verification/postcondition_refuted.una  # exit 1
una trace EVENTS_3  # in examples/events
una contract events.checkin.check_in  # in examples/events
```

Exit codes: `0` success; `1` errors in the program, an unproven obligation or a failed example; `2` a usage error, a problem of the invocation reported on stderr without a diagnostic document; `3` an internal compiler error (U8001), which is a bug to report. Problems of `una.toml` (U7002) and `verification.lock` (U7004) are diagnostics and exit with `1`. With `--json`, stdout holds exactly one JSON document and stderr stays empty.

## 2. Files and packages

- A package is a directory with `una.toml`, `verification.lock` and sources under `src/`. Module `shop.cart` lives in `src/shop/cart.una`, and a file holds exactly one module.
- A single `.una` file is also a package, with default settings and the default lock.
- `una.toml` has `[package]` (`name`, `version`, `edition = "0.1"`), and optionally `[labels] categories`, `[trusted] modules`, `[escape_budget] checked_per_module` (default 0) and `[run]` (`entry` module, `grants`). Unknown keys are U7002.
- `verification.lock` pins the solver: Z3 at the exact version it names must be on the path, or tier 1 reports U5003. `scripts/install-z3.sh` installs it.

## 3. Syntax in brief

- Indentation is significant: two spaces per level, no tabs. A line break inside brackets continues the line, but the canonical form joins such lines, so `una check` reports the split as U7001 (and `una fmt` joins it) unless a comment sits inside the brackets. Write each call on one line.
- `--` starts a comment, `---` a doc comment attached to the next declaration.
- Names: `lower_snake` for values, functions, fields, modules, handlers and operations; `UpperCamel` for kinds, variants, capabilities and label categories; `@name` for capability references; `UPPER_SNAKE_1` for requirements.
- Literals: integers without leading zeros (`1_000` is fine), `true`, `false`, `unit`, and single-line text with `{expression}` interpolation; a literal `}` in text is `\}`.
- Operators, tightest first: calls and `.field`; unary `-`; `*` `/` `mod`; `+` `-`; comparisons `==` `!=` `<` `<=` `>` `>=`, `is Variant` and `in 0..5`, which do not chain; `not`; `and`; `or`; `implies`.
- Words that are keywords in other languages are errors with a hint (U0103): `match` (use `when`), `return` (the last expression is the result), `var` (values are immutable), `while` and `loop` (use recursion or `fold`), `type` (use `kind`). Reserved for later versions: `take`, `edit`, `shared`, `all`, `table`, `query` and others.

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

Built-in types: `Int` (unbounded), `Nat` (`Int` at least 0), `Bool`, `Text`, `Unit`, `List[T]` (`[a, b]`, `xs.len`), `Option[T]` (`Some(value)`, `None`), `Result[T, E]` (`Ok(value)`, `Err(failure)`), `Hashed` (from `Keys` only) and function types `fn(A) -> B`. Records and sums are nominal; aliases and refinements are their base type. Recursive kinds go only through `List` or `Option`.

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
- Blocks bind with `let`; values are immutable. `if c then a else b` fits on a line; `if` and `when` also take indented blocks.
- `when` branches on variants (`is Some(x)`, `is Err`), literals (`is 0`) and ranges (`in 1..<20`); an `else` arm comes last. Arms over variants must cover every variant (U2007).
- The prelude has `abs`, `min`, `max`, `is_digits`, `starts_with`, `trim`, `to_int`, `is_empty`, `first`, `at`, `append`, `contains`, `map`, `filter`, `first_where`, `fold`, `sum`, `is_some`, `or_else` and `is_ok`; their contracts are hypotheses for the solver (`spec/14`). There are no loops: use recursion, `map`, `filter` or `fold`.

```una
-- fails with U2004
module demo

pub add(a: Int, b: Int) -> Int = a + b

pub three() -> Int = add(1, 2)
```

## 6. Contracts and proofs

`requires` and `ensures` are written in the refinement language: literals, parameters, `result`, fields, `.len`, `+`, `-`, `*` by a literal, `/` and `mod` by a nonzero literal, comparisons, `in`, `is`, `and`, `or`, `not`, `implies` and predicate calls. Every call proves the callee's `requires`, every return proves the `ensures`, every value that flows into a refined type proves the refinement, and every division by a variable proves a nonzero divisor. The solver answers `proven`, `refuted` (U5001, with a counterexample) or `unknown` (U5002).

- The solver knows a call only through its contract. Prelude functions without one, such as `fold`, `sum` and `to_int`, tell it nothing, and refinements are not inferred through generic calls: a `fold` that adds naturals still returns an `Int`.
- `predicate reflect` bodies are in the refinement language; the solver sees their definition.
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

A package that establishes an opaque fact once, at its boundary, and declares the budget for it:

`una.toml`
```toml
[package]
name = "slugs"
version = "0.1.0"
edition = "0.1"

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
- Host capabilities come from the runtime into `main` only, as `una run` grants them: `Console[print]`, `Clock[now]`, `Env[get]`, `Files[read, write]` and `Keys[hmac]` (`spec/14 §3`). Grants are `console`, `clock`, `env`, `keys:<file>` and `files:<path>:read`, `:write` or `:read,write`, in `[run] grants` or with `--grant`.

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

## 9. Labels and information flow

Every type may carry a label: a level, `public` (the default), `internal`, `confidential`, `secret` or `restricted`, and categories declared in `una.toml`, as in `secret[PII] Text`. Values flow up, never down: the result of an operation joins its operands' labels; branching on a labeled value raises the program counter, so what the branches produce is labeled too, and they may not fail; each operation argument is a sink bounded by its `accepts` (`Console.print` accepts `public`). `Env.get` and `Files.read` return `internal` text.

```una
module demo

pub kind Entry
  day: Int
  body: secret Text

pub kind Mood = Calm | Busy

--- The answer depends on the text, so it is as secret as the text.
pub mood(entry: Entry) -> secret Mood = if entry.body.len > 280 then Busy else Calm
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
edition = "0.1"

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

## 10. Requirements and examples

A requirement records a statement with its source and owner; `derived from` traces functions to it, and `una trace <ID>` lists them with their obligations. A `generated` requirement stays `proposed` until a person changes its source.

Examples are executable evidence next to the code: `given` installs handlers (never host capabilities), `do` runs one expression, and each `expect` checks `result`, `fails V`, `performed @x.op` or `not performed @x.op`. `una test` runs them with their effect traces.

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

## 11. The Agent ABI

Every command takes `--json` and prints one document whose `schema` names its shape:

| Command | Document |
| --- | --- |
| `una check`, `una fmt --check` | `una.diagnostics/1` |
| `una obligations` | `una.obligations/1`, every obligation with `status`, `goal` and `hypotheses` |
| `una test` | `una.test/1`, every example with its outcome and effect trace |
| `una run` | `una.run/1`, the program's output and trace |
| `una contract <module.function>` | `una.contract/1`, the header, declared or inferred |
| `una trace <REQUIREMENT_ID>` | `una.trace/1`, derived functions and their obligations |
| `una report` | `una.report/1`, proofs, authority, policy calls, escapes, assumptions, trusted code, requirements, coverage |
| `una explain <code>` | `una.explain/1` |

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
