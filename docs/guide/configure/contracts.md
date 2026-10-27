---
construct: "[contracts]"
editions: ["0.3"]
spec: ["spec/13 §7", "spec/05 §3"]
codes: [U3023, U7002]
---

# `[contracts]`

## What it does

`[contracts]` in `una.toml` locks the effect authority promised by selected public functions. It sets a maximum on the operations and conditions the function header may grant. If a later edit broadens that header, `una check` reports U3023. This is an authority lock, not a guarantee that an operation has no effect when its condition is true.

The example below says `update` may write only when `dry_run` is false. Its implementation checks that condition, and both runtime examples verify the dry-run and live paths.

## Forms

```text
[contracts]
"<package.module.function>" = ["@<binding>: <Capability>[<op> [where <condition>], ...]", ...]
```

Each key names a public function in the package. Each value is an array of effect grants in the same syntax as a `uses` clause in a function header; an entry must name at least one operation. A condition applies to the operation immediately before its `where` clause.

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

If the header is changed to `Catalog[price, set_price]` while the manifest still locks `where not dry_run`, checking fails with U3023. The diagnostic includes the counterexample `dry_run = true`.

## Edge cases

The checker verifies two separate relations: the function body must stay within its declared header, and the header must stay within the manifest lock. Passing both means the body is checked against the header and the header against the lock. A lock is not a runtime guard; the implementation must express the condition in its control flow, and the checker must prove the body satisfies that contract.

## Diagnostics

- [U3023](../../../crates/una-explain/explain/U3023.md): the function header grants more authority than its lock permits.
- [U7002](../../../crates/una-explain/explain/U7002.md): a malformed or unresolved lock entry.

## Edition

`[contracts]` is available in edition 0.3. It is unavailable in editions 0.1 and 0.2.

## Normative source

- `spec/13 §7`: the `[contracts]` table and its entries.
- `spec/05 §3`: checking a function header against the locked effect contract.
