---
construct: checked
editions: ["0.1", "0.2"]
spec: ["spec/06 §6", "spec/02 §7", "spec/02 §8", "spec/03 §3", "spec/03 §4", "spec/13 §3", "spec/13 §6", "spec/13 §7", "spec/07 §3", "spec/10 §7", "spec/10 §13.2", "spec/18 §4", "spec/06 §2"]
codes: [U2013, U2014, U2016, U3009, U4002, U7003]
---

# `checked` and the escape budget

## What it does

`checked(v, p) else fail V` tests the predicate `p` (a condition on `it` that is true or false) at run time, with `it` bound to `v`:

- when `p` holds, the result is `v`, with its type refined by `p` (its type now carries the condition `p`);
- otherwise the function fails with `V`.

This is how a value obtains an opaque fact, one the solver (the program that `una check` asks to prove each fact) cannot prove. `checked` replaces a proof made before the program runs with a test while it runs, so it is an escape hatch. Each module may use it only as many times as its package's escape budget (a count set in `una.toml`) allows, and every use is listed in the build report (what `una report` prints about a build).

Learn more: [When no proof can decide: checked and opaque predicates](/learn/failure/#s7-5).

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

pub kind Phone = Text where valid_phone(it)

pub kind PhoneError = | Malformed

pub predicate opaque valid_phone(t: Text) -> Bool = t.len == 11 and is_digits(t)

pub parse_phone(raw: Text) -> Phone
  fails PhoneError
=
  checked(raw, valid_phone(it)) else fail Malformed
```

## Forms

In the grammar below, each rule gives a name, `=`, what the name stands for and `;`, quoted words are written as shown, NEWLINE, INDENT and DEDENT stand for a line break and for one more or one less level of indentation, `[ ]` marks an optional part, `{ }` a part that may repeat, `|` separates alternatives, `( )` groups them, and `(* *)` holds a comment.

```ebnf
checked_expr    = "checked" "(" line_expr "," line_expr ")" "else" "fail" variant_expr ;
```

- The first argument is the value tested.
- The second is the predicate, with `it` naming the value. It is written in the refinement language (the expressions that a type's condition may use, which the solver can reason about), or is a call of an opaque predicate (one whose body the solver never sees), so that it can run.
- `else fail V` names the variant the function fails with when the test fails. `V` belongs to the function's error kind.

The budget is the key `checked_per_module` of the `[escape_budget]` table of `una.toml`, an integer of at least 0 that defaults to 0.

## Examples

A predicate in the refinement language refines a plain value. Here `Shelf`'s refinement holds for the result because the test established it:

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

Without a budget, `checked` is rejected. The default is 0, and a file checked on its own has the default:

```una
-- fails with U7003
module shop

pub kind Phone = Text where valid_phone(it)

pub kind PhoneError = | Malformed

pub predicate opaque valid_phone(t: Text) -> Bool = t.len == 11 and is_digits(t)

pub parse_phone(raw: Text) -> Phone
  fails PhoneError
=
  checked(raw, valid_phone(it)) else fail Malformed
```

## Edge cases

**The predicate must run.** A call of a function that is not a predicate is outside the refinement language:

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
-- fails with U2014
module shop

pub kind SkuError = | NotDigits

pub digits_only(sku: Text) -> Text
  fails SkuError
=
  checked(sku, is_digits(it)) else fail NotDigits
```

**Opaque predicates are called only in `checked`**, and in the bodies of other opaque predicates. A call anywhere else would establish the fact without counting against the budget:

```una
-- fails with U2016
module shop

pub predicate opaque valid_phone(t: Text) -> Bool = t.len == 11 and is_digits(t)

pub looks_valid(raw: Text) -> Bool = valid_phone(raw)
```

**The variant belongs to the function's error kind**:

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
-- fails with U3009
module shop

pub kind StockError = | Negative

pub kind PaymentError = | Declined

pub stock_level(units: Int) -> Int
  fails StockError
=
  checked(units, it >= 0) else fail Declined
```

**No test that depends on labeled data** (data marked with a label above `public`, such as `internal` or `secret`). A failure of `checked` reveals whether the predicate holds. So `checked` is U4002 on a labeled value, on a predicate that reads a labeled value, or under a raised [Branches and failures: implicit flows](/learn/information-flow/#s17-3) (the label of the conditions that decided whether this line runs):

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
-- fails with U4002
module shop

pub kind CreditError = | Negative

pub balance(cents: secret Int) -> secret Int
  fails CreditError
=
  checked(cents, it >= 0) else fail Negative
```

The same holds when the operand is public but the predicate reads labeled data, here an `internal` bound:

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
-- fails with U4002
module shop

pub kind LimitError = | OverLimit

pub within_limit(cents: Int, limit: internal Int) -> Int
  fails LimitError
=
  checked(cents, it < limit) else fail OverLimit
```

Further rules:

- **Counting.** The first use beyond the budget is U7003, with the later uses as related locations. The budget counts per module, not per package.
- **`it`** is valid only in the predicate of `checked` and in `where` clauses (U2013). In edition `0.3`, only the `where` of a kind counts, not the condition on an operation.
- **Repairs prefer proofs.** When a proof is refuted (the solver finds a case where the fact is false), the repairs list a stronger type, a `requires` clause or a guard before `checked`, which they mark as an escape with a cost.
- **Reported.** Every use of `checked` is listed in the build report, so reviewers see each run-time test that replaces a proof.
- **Narrowing integers.** In edition `0.2`, a narrowing conversion of an integer (to a type with fewer values) carries an `overflow` obligation (a fact `una check` must prove), or narrows through `checked` on the same budget.

## Diagnostics

- [U2013](../../../../crates/una-explain/explain/U2013.md): `it` outside the predicate of `checked` or a `where` clause.
- [U2014](../../../../crates/una-explain/explain/U2014.md): a predicate outside the refinement language.
- [U2016](../../../../crates/una-explain/explain/U2016.md): an opaque predicate called outside `checked`.
- [U3009](../../../../crates/una-explain/explain/U3009.md): the variant of `else fail` outside the function's error kind.
- [U4002](../../../../crates/una-explain/explain/U4002.md): `checked` of a labeled value, with a predicate that reads one, or under a raised program counter.
- [U7003](../../../../crates/una-explain/explain/U7003.md): more uses in a module than its escape budget allows.

## Edition

`checked` and the escape budget are the same in editions `0.1` and `0.2`. Edition `0.2` keeps them and adds fixed-width integers, whose narrowing conversions may go through `checked`.

## Normative source

- `spec/06 §6`: `checked`, its predicate, the budget and its counting per module; `spec/06 §2`: the variant belongs to the error kind.
- `spec/02 §7`: the grammar; `spec/02 §8`: where `it` may appear.
- `spec/03 §3`: the refinement language; `spec/03 §4`: opaque predicates.
- `spec/13 §3`: repairs; `spec/13 §6`: the build report; `spec/13 §7`: `[escape_budget]`.
- `spec/10 §7`: T-Checked and its label rule; `spec/10 §13.2`: the predicate's other reads.
- `spec/07 §3` rule 3: failures under the program counter.
- `spec/18 §4` rule 4: narrowing conversions.
- Decision record: [ADR 0089](../../../adr/0089-checked-predicate-labels.md), for the labels a predicate reads.
