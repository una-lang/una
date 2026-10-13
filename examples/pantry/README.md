# pantry

A kitchen pantry: items with amounts, a shopping list of what ran out, and totals per measure.

## What it shows

`src/pantry/stock.una`:

- [`kind`](../../docs/guide/language/types/kind.md) declarations of every form: a choice (`Measure = Grams | Pieces | Milliliters`), an alias (`Label = Text`), a record (`Item`), and a generic record (`Bin[T]`).
- A refinement with [`where`](../../docs/guide/language/types/where.md): `NonEmpty[T] = List[T] where it.len > 0`, so `average` divides by a length the solver knows is not zero.
- Contracts: `restock` and `use_up` state, with [`ensures`](../../docs/guide/language/functions/ensures.md), how the amount changes, and `use_up` [`requires`](../../docs/guide/language/functions/requires.md) enough of the item. `missing`, `shopping_list` and `count_where` promise a result no longer than their input.
- `Nat` amounts, which the solver proves never go below zero.
- `find` returns an `Option[Item]`, and `amount_of` matches on it with [`when`](../../docs/guide/language/control-flow/when.md).
- A generic function, `count_where[T]`, over a `Bin[T]`, which takes a function value as its `keep` argument.
- Seven [`example`](../../docs/guide/language/examples/example.md) blocks.

`src/pantry/main.una` builds this morning's stock, prints the shopping list and the grams in stock. It may only print.

## What `una check` proves

- Each `ensures` above, for every input.
- Every `Nat` stays at or above zero, including `item.amount - used`, which `use_up`'s `requires` makes safe.
- `average` never divides by zero.
- Every call meets its callee's `requires`, such as the call to `use_up` in its example.

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

```text
to buy: rice, oil
grams in stock: 700
```

## Edition

Edition `0.1` ([edition](../../docs/guide/configure/package/edition.md)).
