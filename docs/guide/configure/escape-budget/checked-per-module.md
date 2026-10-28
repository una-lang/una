---
construct: "[escape_budget] checked_per_module"
editions: ["0.1", "0.2"]
spec: ["spec/13 §7", "spec/06 §6", "spec/13 §3", "spec/13 §6"]
codes: [U7002, U7003]
---

# `[escape_budget] checked_per_module`

## What it does

Una proves the facts a program relies on before it runs, such as a shelf number being between 0 and 9. When a fact cannot be proved, as with a number a customer types, `checked` tests it while the program runs instead, and fails when it does not hold. That gives up a proof, so it is an escape, and `checked_per_module` is the escape budget: how many times each module of the package may use `checked`. The default is 0, so a package that wants run-time tests says so here, and reviewers see it.

Learn more: [When no proof can decide: checked and opaque predicates](/learn/failure/when-no-proof-can-decide-checked-and-opaque-predicates/).

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[escape_budget]
checked_per_module = 1
```

`src/shop.una`

```una
module shop

pub kind Shelf = Int in 0..9

pub kind ShelfError = | NoSuchShelf

pub shelf(n: Int) -> Shelf
  fails ShelfError
=
  checked(n, it >= 0 and it <= 9) else fail NoSuchShelf
```

## Forms

```text
[escape_budget]
checked_per_module = <an integer of at least 0>
```

The default is 0, and a single file has the default.

## Examples

A use beyond the budget is rejected, here with the default of 0:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"
```

`src/shop.una`

```una
-- fails with U7003
module shop

pub kind Shelf = Int in 0..9

pub kind ShelfError = | NoSuchShelf

pub shelf(n: Int) -> Shelf
  fails ShelfError
=
  checked(n, it >= 0 and it <= 9) else fail NoSuchShelf
```

## Edge cases

**At least 0**:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[escape_budget]
checked_per_module = -1
```

`src/shop.una`

```una
-- fails with U7002
module shop

pub price(cents: Int) -> Int = cents
```

Further rules:

- **Per module.** The budget counts each module separately.
- **Reported.** Every use of `checked` is listed in the build report.
- **Last resort.** When a proof is refuted, the repairs list a stronger type, a `requires` clause or a guard first; `checked` and raising the budget are escapes, marked as such, and come last. See [`checked`](../../language/errors/checked.md).

## Diagnostics

- [U7002](../../../../crates/una-explain/explain/U7002.md): a budget that is not an integer of at least 0.
- [U7003](../../../../crates/una-explain/explain/U7003.md): more `checked` uses in a module than the budget allows.

## Edition

The escape budget is the same in editions `0.1` and `0.2`.

## Normative source

- `spec/13 §7`: `[escape_budget]`.
- `spec/06 §6`: `checked` and the budget.
- `spec/13 §3`: repairs; `spec/13 §6`: the build report.
