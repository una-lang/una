---
construct: "[contracts]"
editions: ["0.3"]
spec: ["spec/13 §7", "spec/05 §3", "spec/30 §2"]
codes: [U3023, U7002]
---

# `[contracts]`

## What it does

`[contracts]` in `una.toml` locks the `uses` clauses of selected public functions. The lock is the most authority the function header may grant: its bindings, their operations, and the conditions on those operations. If a later edit broadens that header, `una check` reports U3023. The lock constrains the header only; the body is checked against the header, so together they bound what the function may do.

The example below says `update` may run `set_price` only when `dry_run` is false. Its implementation guards the operation with that condition, and the two examples run the dry-run and live paths.

Learn more: [Locking a contract: contracts](/learn/authority/locking-a-contract-contracts/) and [Limits on authority](/learn/authority/).

## Forms

```text
[contracts]
"<module path>.<function>" = ["@<binding>: <Capability>[<op> [where <condition>], ...]", ...]
```

Each key is the qualified name of a public function of the package: its module path, then the function name. Each value is an array of `uses` clauses written as a header writes them, with the operations listed in brackets; an entry must list at least one operation. A condition applies to the operation immediately before its `where`, and is a `Bool` of the refinement language over the function's parameters.

## Examples

`una.toml`

```toml
[package]
name = "demo"
version = "0.1.0"
edition = "0.3"

[contracts]
"demo.update" = ["@catalog: Catalog[price, set_price where not dry_run]"]
```

`src/demo.una`

```una
module demo

pub capability Catalog
  op price(sku: Int) -> Int
  op set_price(sku: Int, cents: Int) -> Unit

pub update(sku: Int, cents: Int, dry_run: Bool) -> Unit
  uses @catalog: Catalog[price, set_price where not dry_run]
=
  if not dry_run
    @catalog.set_price(sku, cents)
  unit

handler catalog() for Catalog
  op price(sku) = 0
  op set_price(sku, cents) = unit

example "dry run has no writes"
  given @catalog = catalog()
  do update(1, cents: 10, dry_run: true)
  expect not performed @catalog.set_price

example "live writes"
  given @catalog = catalog()
  do update(1, cents: 10, dry_run: false)
  expect performed @catalog.set_price
```

If the header is changed to `Catalog[price, set_price]` while the manifest still locks `set_price where not dry_run`, the `contract_lock` obligation that the header's condition implies the lock's is refuted, and checking fails with U3023 at the operation. Adding an operation or a binding that the lock does not list is U3023 too. A header that drops an operation or strengthens a condition grants less, and passes.

## Edge cases

The checker verifies two separate relations: the function body must stay within its declared header (`spec/05 §3` rule 4), and the header must stay within the lock (rule 5). A lock is not a run-time guard; the implementation expresses the condition in its control flow, and the verifier proves that each operation runs only where its condition holds.

Only public functions are locked. A handler is not locked: its operations are checked against its own header. The repair of U3023 brings the header back to its lock; changing the lock is never the repair.

## Diagnostics

- [U3023](../../../crates/una-explain/explain/U3023.md): the function header grants more authority than its lock permits.
- [U7002](../../../crates/una-explain/explain/U7002.md): an invalid lock entry: a key that names no public function, an entry that does not parse or lists no operations, a binding locked twice, an unknown capability or operation, or a condition that is not a `Bool` of the refinement language over the parameters. In editions `0.1` and `0.2`, `[contracts]` is an unknown table, also U7002.

## Edition

`[contracts]` is available in edition `0.3`, a preview. In editions `0.1` and `0.2` it is an unknown table (U7002). Edition `0.4` keeps the rule, but a package of that edition describes itself in `ita.una`, where the lock is the `contracts` section, with one locked `uses` clause per line; its errors are U7008 at the line. `una migrate` converts `[contracts]` to that section.

## Normative source

- `spec/13 §7`: the `[contracts]` table and its entries.
- `spec/05 §3` rule 4: conditions on operations; rule 5: checking a header against its lock.
- `spec/30 §2`: the `contracts` section of `ita.una` in edition `0.4`.
- Decision record: [ADR 0092](../../adr/0092-conditions-on-allowed-effects.md), rule 8 and its amendments.
