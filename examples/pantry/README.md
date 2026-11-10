# pantry

A kitchen pantry: items with amounts, a shopping list of what ran out, and totals per measure.

This example is about data and contracts, with no outside world beyond the console. It declares every form of `kind`, gives several functions a contract, and lets the solver prove that no amount goes below zero and that no division is by zero. Read it when you want to see how types and contracts replace the checks that other languages leave to tests.

What it demonstrates:

- [`kind`](../../docs/guide/language/types/kind.md) declarations of every form: a choice, an alias, a record and a generic record;
- refinements with [`where`](../../docs/guide/language/types/where.md): `Nat`, and a list that is never empty;
- contracts with [`requires`](../../docs/guide/language/functions/requires.md) and [`ensures`](../../docs/guide/language/functions/ensures.md);
- `Option` and an exhaustive [`when`](../../docs/guide/language/control-flow/when.md);
- a generic function that takes a function value;
- seven [`example`](../../docs/guide/language/examples/example.md) blocks.

## Run it

From this directory:

```sh
una check .   # copy examples/pantry
una test .
una run .
una build .
./target/una/pantry-example
```

`una run .` and the native executable print:

```text una="run ."
to buy: rice, oil
grams in stock: 700
```

The pantry holds no rice and no oil, so they are on the list. Rice (0 g) and flour (700 g) are the items measured in grams. `una test .` runs the seven examples of `src/pantry/stock.una`:

```text una="test ."
example pantry.stock "restocking adds to the amount" ... ok
example pantry.stock "using up takes from the amount" ... ok
example pantry.stock "the shopping list names what ran out" ... ok
example pantry.stock "totals count one measure only" ... ok
example pantry.stock "an item the pantry does not have amounts to zero" ... ok
example pantry.stock "a bin counts the entries that match" ... ok
example pantry.stock "the average of three counts is rounded down" ... ok

7 examples: 7 passed, 0 failed
```

## How it works

The package has two modules. `pantry.stock` holds the types and the functions, with their examples. `pantry.main` builds this morning's stock and prints the report. It may only print.

### The types

```text file="src/pantry/stock.una"
pub kind Measure = Grams | Pieces | Milliliters

...
pub kind Label = Text

pub kind Item
  name: Text
  amount: Nat
  measure: Measure

...
pub kind Bin[T]
  label: Label
  entries: List[T]

...
pub kind NonEmpty[T] = List[T] where it.len > 0
```

- `Measure` is a choice: a value is exactly one of the three variants.
- `Label` is an alias: another name for `Text`, which documents what the text means.
- `Item` is a record. Its `amount` is a `Nat`, an `Int` that is never negative. The solver checks that fact at every place that builds an `Item`.
- `Bin[T]` is a generic record: a labeled group of anything.
- `NonEmpty[T]` is a refinement: a `List[T]` with a fact that the solver must prove wherever a value of that type is made.

### Contracts that keep amounts honest

```text file="src/pantry/stock.una"
pub use_up(item: Item, used: Nat) -> Item
  requires used <= item.amount
  ensures result.amount == item.amount - used
=
  Item(name: item.name, amount: item.amount - used, measure: item.measure)
```

`item.amount - used` must be a `Nat`, since `amount` is one. The solver proves it from the `requires`: whoever calls `use_up` must show that there is enough. `restock` has the matching `ensures` for addition. `missing` and `shopping_list` promise a result no longer than their input (`ensures result.len <= items.len`), and `count_where` a count no larger than its bin (`ensures result <= bin.entries.len`). A caller can rely on these without reading the bodies.

### A division that cannot be by zero

```text file="src/pantry/stock.una"
pub average(counts: NonEmpty[Int]) -> Int = counts.sum() / counts.len
```

Every division carries an obligation: the divisor is not zero. The parameter type says the list has at least one entry, so the solver proves it here. The caller pays instead: the example `average([2, 4, 10])` must show that its list is not empty, and the solver proves that too.

### Absence and generics

`find` returns an `Option[Item]` from `first_where`, and `amount_of` matches on it:

```text file="src/pantry/stock.una"
pub amount_of(items: List[Item], name: Text) -> Nat
=
  when find(items, name)
    is Some(item) -> item.amount
    is None -> 0
```

`count_where[T]` works on a `Bin` of any type. It takes the test to apply as a function value, `keep: fn(T) -> Bool`, and passes it straight on to `filter`.

## What `una check` proves

`una obligations .` lists 27 obligations, all proven. Among them:

- each `ensures` above, for every input;
- every `Nat` stays at or above zero, including `item.amount - used`, which `use_up`'s `requires` makes safe;
- `average` never divides by zero;
- every call meets its callee's `requires`, such as the call to `use_up` in its example, `4 <= Item(name: "eggs", amount: 6, measure: Pieces).amount`.

The type checker adds the rest without the solver: every `when` on a choice covers all its variants, and every value has the type its place requires.

## Common mistakes

### Taking more than there is

Delete the `requires used <= item.amount` line of `use_up`. `una check .` reports that the new amount may be negative (its location and repairs shown; the notes after them are left out):

```text una="check ." after="mistakes/1"
[U5001] Error: refinement `(item.amount - used) >= 0` is refuted
    ╭─[ src/pantry/stock.una:32:33 ]
    │
 32 │   Item(name: item.name, amount: item.amount - used, measure: item.measure)
    │                                 ─────────┬────────
    │                                          ╰────────── refinement `(item.amount - used) >= 0` is refuted
    │
    │ Help 1: repair 1: add a guard (`if` or a `when` arm) that handles the values where `(item.amount - used) >= 0` does not hold
    │
    │ Help 2: repair 2 (escape: checked; a runtime test replaces the static proof: the program fails at run time where the proof would have rejected it; it counts against [escape_budget] and is listed in the build report): check the value with `checked(value, (item.amount - used) >= 0) else fail <Variant>`
...
```

The diagnostic points at `item.amount - used` on line 32. Its repairs are a guard, or a `checked` that tests the value at run time. Here the right fix is the `requires`: running out is the caller's decision, not something `use_up` can repair.

### Dividing by the length of any list

Change the parameter of `average` to `counts: List[Int]`. The solver finds the empty list:

```text una="check ." after="mistakes/2"
[U5001] Error: division obligation `counts.len != 0` is refuted
    ╭─[ src/pantry/stock.una:67:41 ]
    │
 67 │ pub average(counts: List[Int]) -> Int = counts.sum() / counts.len
    │                                         ────────────┬────────────
    │                                                     ╰────────────── division obligation `counts.len != 0` is refuted
    │
    │ Help: repair 1: guard the division with `if counts.len != 0`
    │
    │ Note 1: obligation ob-57d6a6e35832 (division) is refuted: counts.len != 0
    │
    │ Note 2: counterexample: counts = List#0, counts.len = 0
    │
    │ Note 3: run `una explain U5001` for an explanation
────╯
```

The repair offers a guard. The example prefers a type, `NonEmpty[Int]`, because it states the rule once, in the header, where every caller sees it.

### Forgetting a case

Delete the `is None -> 0` arm of `amount_of`:

```text una="check ." after="mistakes/3"
[U2007] Error: `when` does not match variant `None` of `Option`
    ╭─[ src/pantry/stock.una:56:3 ]
    │
 56 │   when find(items, name)
    │   ──┬─
    │     ╰─── `when` does not match variant `None` of `Option`
    │
    │ Help: repair 1: add an arm for each missing variant, such as `is None`, or an `else` arm
    │
    │ Note: run `una explain U2007` for an explanation
────╯
```

## Extend it

1. **Name the units.** Add `pub unit_name(measure: Measure) -> Text`, a `when` with one arm per measure (`"g"`, `"pcs"`, `"ml"`). Then add a variant, `Kilograms`, to `Measure`. `una check .` reports a U2007 at your `when`: the checker finds every match that the new case affects.
2. **Warn before it runs out.** Add `pub running_low(items: List[Item], below: Nat) -> List[Item]` with `ensures result.len <= items.len`, and an example where eggs at 2 are below 3 and flour at 700 is not. *Hint:* `missing` has the same shape.
3. **Use edition `0.2`.** Set `edition = "0.2"` in `una.toml`. The package still checks. Then replace the `fold` in `main` that joins the names with `join(needed, separator: ", ")`, a prelude function of edition `0.2`. The output does not change.

## Learn more

- Construct pages: [`kind`](../../docs/guide/language/types/kind.md), [`where`](../../docs/guide/language/types/where.md), [`requires`](../../docs/guide/language/functions/requires.md), [`ensures`](../../docs/guide/language/functions/ensures.md), [`when`](../../docs/guide/language/control-flow/when.md), [`example`](../../docs/guide/language/examples/example.md).
- Learn, the tutorial on the Una website: "Kinds" (`/learn/kinds/`); "Collections" (`/learn/collections/`); "Contracts and the verifier" (`/learn/contracts/`).
- Specification: [kinds (spec/03 §2)](../../docs/spec/03-types.md), [refinements (spec/03 §3)](../../docs/spec/03-types.md), [clauses (spec/04 §2)](../../docs/spec/04-functions-and-contracts.md), [obligation generation (spec/12 §3)](../../docs/spec/12-verification.md), [the prelude (spec/14 §1)](../../docs/spec/14-standard-library.md).

## Edition

Edition `0.1` ([edition](../../docs/guide/configure/package/edition.md)). It also checks unchanged under edition `0.2`, as the third exercise shows.
