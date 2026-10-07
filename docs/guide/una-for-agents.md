# Una for agents

A compact reference for writing Una with the v0.2 toolchain, in packages of edition `0.2`. The specification in `docs/spec/` is normative; this guide summarizes it for an agent that writes, checks and repairs code.

How to read the blocks: every `una` block is a whole module that `una check` accepts in canonical form, as the only module of a package of edition `0.2`, unless its first line is `-- fails with U####`, in which case checking it yields that code and no other. A block whose first line is `-- edition 0.1` is checked in a package of edition `0.1`. A block preceded by a file path belongs to a package with the other files named around it. Every `json` block is a document the CLI prints, and every `sh` block runs from the repository root, or in a fresh copy of the directory its `# copy` comment names. `crates/una-cli/tests/guide.rs` checks all of them.

## 1. The loop

1. Write or edit `.una` files, then run `una fmt <path>`. The canonical form is part of the language: `una check` reports any other layout as U7001, so generated code goes through `una fmt` before `una check`.
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

`una build` compiles a package into a native executable that behaves as `una run` (§15):

```sh
una build .  # copy examples/hello
./target/una/hello
```

Exit codes: `0` success; `1` errors in the program, an unproven obligation or a failed example; `2` a usage error, a problem of the invocation reported on stderr without a diagnostic document; `3` an internal compiler error (U8001), which is a bug to report. Problems of `una.toml` (U7002) and `verification.lock` (U7004) are diagnostics and exit with `1`. With `--json`, stdout holds exactly one JSON document and stderr stays empty.

## 2. Files and packages

- A package is a directory with `una.toml`, `verification.lock` and sources under `src/`. Module `shop.cart` lives in `src/shop/cart.una`, and a file holds exactly one module.
- A single `.una` file is also a package, with default settings and the default lock.
- `una.toml` has `[package]` (`name`, `version`, `edition = "0.2"`), and optionally `[labels] categories`, `[trusted] modules`, `[escape_budget] checked_per_module` (default 0) and `[run]` (`entry` module, `grants`). Unknown keys are U7002.
- **Editions.** `edition = "0.2"` enables the constructs of v0.2: editable values and `for` loops (§11), fixed-width integers (§12), recursive kinds (§4), `uses of` and `fails of` (§8), constraints on generics (§13) and `together` (§14). A package of edition `0.1` keeps the language of v0.1 in every later compiler, and a single `.una` file outside a package is edition `0.1`. §17 lists what changes between them.
- `verification.lock` pins the solver: Z3 at the exact version it names must be on the path, or tier 1 reports U5003. `scripts/install-z3.sh` installs it.

## 3. Syntax in brief

- Indentation is significant: two spaces per level, no tabs. A line break inside brackets continues the line, but the canonical form joins such lines, so `una check` reports the split as U7001 (and `una fmt` joins it) unless a comment sits inside the brackets. Write each call on one line.
- `--` starts a comment, `---` a doc comment attached to the next declaration.
- Names: `lower_snake` for values, functions, fields, modules, handlers and operations; `UpperCamel` for kinds, variants, capabilities and label categories; `@name` for capability references; `UPPER_SNAKE_1` for requirements.
- Literals: integers without leading zeros (`1_000` is fine), `true`, `false`, `unit`, and single-line text with `{expression}` interpolation; a literal `}` in text is `\}`.
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

Interpolation writes any value that holds no function in its Una form, the one `una test` shows: `Point(x: 1, y: -2)`, `[1, 2]`, `Some(3)`, `Err(NotFound)`. Text inside a structure is quoted. A field that holds a function is U2001. A value of a type parameter `T` needs `[T: Eq]`. The text keeps the labels of every field, so a record with a `secret` field cannot be printed (U4003) (ADR 0068).

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
- Blocks bind with `let`; values are immutable, and a `let edit` binding can be rebound (§11). `if c then a else b` fits on a line; `if` and `when` also take indented blocks.
- `when` branches on variants (`is Some(x)`, `is Err`), literals (`is 0`) and ranges (`in 1..<20`); an `else` arm comes last. Arms over variants must cover every variant (U2007).
- The prelude has `abs`, `min`, `max`, `is_digits`, `starts_with`, `trim`, `to_int`, `is_empty`, `first`, `at`, `append`, `contains`, `map`, `filter`, `first_where`, `fold`, `sum`, `is_some`, `or_else` and `is_ok`, and in edition `0.2` also `sort` and `set`; their contracts are hypotheses for the solver (`spec/14`, `spec/20 §6`). Loop with `for` (§11), recursion, `map`, `filter` or `fold`.
- The result of `filter` and `first_where` carries the labels of the predicate's result at its top, since which elements remain depends on it: over `secret` elements, a predicate that reads them gives a list whose top is `secret` (`spec/14`, result labels, ADR 0067). A predicate with effects whose result is labeled is U4002 in `first_where`.
- A call of a generic function raises each position of its result only by the type arguments that position depends on; `una contract` shows them as `-- result labels: ...`. A list whose top depends on comparisons of `T` values carries `T`'s labels, and a parameter of a user function still takes only what its type allows, so build such lists with the prelude (`fold`, `append`), not by passing them to your own generic functions.

```una
-- fails with U2004
module demo

pub add(a: Int, b: Int) -> Int = a + b

pub three() -> Int = add(1, 2)
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

## 11. Editable values and loops

Values stay immutable and there are no references; what changes is which value a binding holds (`spec/17`).

- `let edit x = e` declares an editable binding, and `x = e` or `x.field = e` rebinds it. Assigning anything else is U2019. An editable binding keeps its declared type, refinement and label at every rebind.
- A parameter declared `edit p: T` is passed in and out: the caller writes `edit place` and gets the final value back; `old(p)` in `ensures` is its value on entry. A method receiver is marked before it: `edit tally.record(amount)`.
- A parameter declared `take p: T` consumes its argument: the caller writes `take x`, and `x` cannot be used afterwards (U2017) until it is assigned again.
- Markers match declarations exactly (U2020); an `edit` argument and another use of the same binding in one call is U2018; a function with `edit` or `take` parameters cannot be used as a value (U2021). A call with a marker is the whole expression of its statement.
- `for x in xs` visits a list in order, and `for i in a..b` or `a..<b` the integers of a range; the bounds are evaluated once, before the first iteration. The loop variable has the element type, or the integer type of the bounds. There is no `break` or `continue`. The refinements of the bindings a loop updates are its invariant; no other fact about them survives the loop.
- `if c` with an indented block and no `else` is a statement. `edit xs.set(index: i, item: v)` replaces a list element.
- Known limit of v0.2.0 (#260): a loop forgets the length of a list it updates, so `for i in 0..<xs.len` with `edit xs.set(index: i, item: v)` in its body is U5001, refuted with a counterexample where `xs.len` is 0. Guard the update with a statement `if i < xs.len` around it.
- A call that fails leaves its `edit` arguments unchanged for the caller; operations it performed stay performed.

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

--- The guard keeps the precondition of `set` provable inside the loop (#260).
pub zeroed(xs: List[Int]) -> List[Int]
=
  let edit ys = xs
  for i in 0..<ys.len
    if i < ys.len
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

## 14. Concurrent tasks: `together`

`together` runs independent work at once with the result and trace of a sequential run (`spec/19`).

- A `together` statement holds two or more `let` branches (U2024 otherwise). Each branch runs as a task, sees the bindings before the block but not its siblings, and its binding is in scope after the block. Each branch's handling clauses stand on their own lines, as for any `let`.
- `let ys = together for x in xs` runs its indented body once per element, or per integer of a range, and builds the list of results in order.
- A task cannot assign, `edit` or `take` a binding declared outside it (U2023): return new values instead. Tasks use the capabilities of the enclosing function, and a parameter named by `uses of` or `fails of`.
- When a task fails, the tasks after it are cancelled, and the block fails with the failure of the first task that failed: the outcome of running the tasks one after another.
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

## 15. Native executables and the language server

- `una build [path]` checks the package at tier 1, then writes a native executable to `target/una/<name>` (`spec/16`), or to `--out <file>`. It refuses a package with errors, as `una run` does; `--json` prints `una.build/1`.
- The executable runs `main` of the entry module with the output, failures, exit codes and grants of `una run`: the `[run] grants` of `una.toml` are built in, and `--grant`, `--max-depth` and `--max-memory` work as for `una run`. Its `--trace <file>` writes the trace as a JSON array, the trace that `una --json run` reports in `una.run/1`. Its output and trace match the interpreter's.
- Tasks of `together` run on their own threads in a native executable, and one at a time in `una run`.
- `una lsp` is a language server (LSP 3.17 over stdin and stdout) for editors: it checks, formats and solves, and never runs programs.

## 16. The Agent ABI

Every command takes `--json` and prints one document whose `schema` names its shape:

| Command | Document |
| --- | --- |
| `una check`, `una fmt --check` | `una.diagnostics/1` |
| `una obligations` | `una.obligations/1`, every obligation with `status`, `goal` and `hypotheses` |
| `una test` | `una.test/1`, every example with its outcome and effect trace |
| `una run` | `una.run/1`, the program's output and trace |
| `una build` | `una.build/1`, the native executable's path, or the diagnostics that rejected the package (v0.2, `spec/16`) |
| `una contract <module.function>` | `una.contract/1`, the header, declared or inferred |
| `una trace <REQUIREMENT_ID>` | `una.trace/1`, derived functions and their obligations |
| `una report` | `una.report/2`, proofs, authority, policy calls, escapes, assumptions, trusted code, requirements, coverage, native target |
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

## 17. Migrating from edition 0.1

Change `edition = "0.1"` to `"0.2"` in `una.toml`, then check. What can change:

- **New keywords.** `edit`, `take`, `old` and `together` are keywords, and `for` starts a loop: rename bindings that use them.
- **Messages.** `var`, `while`, `loop`, `async` and `await` point to `let edit`, `for` and `together`.
- **Equality on type parameters** needs `Eq`: a public generic function that compares values of `T` writes `[T: Eq]` (U2026). Edition `0.1` inferred the bound.
- **The prelude.** `min`, `max`, `contains` and `sum` are generic over constrained types, `sort` and `set` are new, and `map`, `filter`, `first_where` and `fold` take effectful and failing functions (`uses of`, `fails of`).
- **Kinds.** Recursive kinds no longer need a `List` or `Option` field, but must have a finite value; `I8` to `U64` become built-in types, so a user kind with one of those names is U1002.
- **Integers.** Values of a fixed-width type now carry overflow obligations; `Int` code is unchanged.

The same module can stay in edition `0.1`, where for example `type` keeps its hint and `edit` is still reserved:

```una
-- edition 0.1
-- fails with U0103
module demo

pub change(edit: Int) -> Int = edit
```
