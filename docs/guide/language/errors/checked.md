---
construct: checked
editions: ["0.1", "0.2"]
spec: ["spec/06 §6", "spec/02 §7", "spec/02 §8", "spec/03 §3", "spec/03 §4", "spec/13 §3", "spec/13 §6", "spec/13 §7", "spec/07 §3", "spec/18 §4"]
codes: [U2013, U2014, U2016, U3009, U4002, U7003]
---

# `checked` and the escape budget

## What it does

`checked(v, p) else fail V` tests the predicate `p` at run time, with `it` bound to `v` (`spec/06 §6`):

- when `p` holds, the result is `v`, with its type refined by `p`;
- otherwise the function fails with `V`.

This is how a value obtains an opaque fact, one the solver cannot prove (`spec/03 §4`). `checked` replaces a static proof with a run-time test, so it is an escape hatch. Each module may use it only as many times as its package's escape budget allows, and every use is listed in the build report (`spec/06 §6`).

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

## Forms

```ebnf
checked_expr    = "checked" "(" line_expr "," line_expr ")" "else" "fail" variant_expr ;
```

This is `spec/02 §7`.

- The first argument is the value tested.
- The second is the predicate, with `it` naming the value (`spec/02 §8`). It is written in the refinement language, or is a call of an opaque predicate, so that it can run (`spec/06 §6`, `spec/03 §3`).
- `else fail V` names the variant the function fails with when the test fails. `V` belongs to the function's error kind (`spec/06 §2`).

The budget is the key `checked_per_module` of the `[escape_budget]` table of `una.toml`, an integer of at least 0 that defaults to 0 (`spec/13 §7`).

## Examples

A predicate in the refinement language refines a plain value. Here `Small`'s refinement holds for the result because the test established it (`spec/06 §6`):

`una.toml`

```toml
[package]
name = "digits"
version = "0.1.0"
edition = "0.2"

[escape_budget]
checked_per_module = 1
```

`src/digits.una`

```una
module digits

pub kind Small = Int in 0..9

pub kind Bad = | Out

pub small(n: Int) -> Small
  fails Bad
=
  checked(n, it >= 0 and it <= 9) else fail Out
```

Without a budget, `checked` is rejected. The default is 0, and a file checked on its own has the default (`spec/06 §6`, `spec/13 §7`):

```una
-- fails with U7003
module phones

pub kind Phone = Text where valid_phone(it)

pub kind PhoneError = | Malformed

pub predicate opaque valid_phone(t: Text) -> Bool = t.len == 11 and is_digits(t)

pub parse_phone(raw: Text) -> Phone
  fails PhoneError
=
  checked(raw, valid_phone(it)) else fail Malformed
```

## Edge cases

**The predicate must run.** A call of a function that is not a predicate is outside the refinement language (`spec/06 §6`, `spec/03 §3`):

`una.toml`

```toml
[package]
name = "texts"
version = "0.1.0"
edition = "0.2"

[escape_budget]
checked_per_module = 1
```

`src/texts.una`

```una
-- fails with U2014
module texts

pub kind Bad = | Out

pub digits_only(t: Text) -> Text
  fails Bad
=
  checked(t, is_digits(it)) else fail Out
```

**Opaque predicates are called only in `checked`**, and in the bodies of other opaque predicates. A call anywhere else would establish the fact without counting against the budget (`spec/06 §6`, `spec/03 §4`):

```una
-- fails with U2016
module demo

pub predicate opaque valid(t: Text) -> Bool = t.len == 11 and is_digits(t)

pub ok(t: Text) -> Bool = valid(t)
```

**The variant belongs to the function's error kind** (`spec/06 §2`):

`una.toml`

```toml
[package]
name = "counts"
version = "0.1.0"
edition = "0.2"

[escape_budget]
checked_per_module = 1
```

`src/counts.una`

```una
-- fails with U3009
module counts

pub kind Bad = | Out

pub kind Other = | Nope

pub positive(n: Int) -> Int
  fails Bad
=
  checked(n, it >= 0) else fail Nope
```

**No test that depends on labeled data.** `checked` can fail, so it is allowed only under a `public` program counter. A test of a `secret` value would reveal it by failing (`spec/07 §3` rule 3):

`una.toml`

```toml
[package]
name = "secrets"
version = "0.1.0"
edition = "0.2"

[escape_budget]
checked_per_module = 1
```

`src/secrets.una`

```una
-- fails with U4002
module secrets

pub kind Bad = | Out

pub positive(n: secret Int) -> secret Int
  fails Bad
=
  checked(n, it >= 0) else fail Out
```

Further rules:

- **Counting.** The first use beyond the budget is U7003, with the later uses as related locations (`spec/06 §6`). The budget counts per module, not per package (`spec/06 §6`).
- **`it`** is valid only in the predicate of `checked` and in `where` clauses (U2013, `spec/02 §8`).
- **Repairs prefer proofs.** When a proof is refuted, the repairs list a stronger type, a `requires` clause or a guard before `checked`, which they mark as an escape with a cost (`spec/13 §3`).
- **Reported.** Every use of `checked` is listed in the build report, so reviewers see each run-time test that replaces a proof (`spec/06 §6`, `spec/13 §6`).
- **Narrowing integers.** In edition `0.2`, a narrowing conversion of an integer carries an `overflow` obligation, or narrows through `checked` on the same budget (`spec/18 §4` rule 4).

## Diagnostics

- [U2013](../../../../crates/una-explain/explain/U2013.md): `it` outside the predicate of `checked` or a `where` clause.
- [U2014](../../../../crates/una-explain/explain/U2014.md): a predicate outside the refinement language.
- [U2016](../../../../crates/una-explain/explain/U2016.md): an opaque predicate called outside `checked`.
- [U3009](../../../../crates/una-explain/explain/U3009.md): the variant of `else fail` outside the function's error kind.
- [U4002](../../../../crates/una-explain/explain/U4002.md): `checked` under a program counter above `public`.
- [U7003](../../../../crates/una-explain/explain/U7003.md): more uses in a module than its escape budget allows.

## Edition

`checked` and the escape budget are the same in editions `0.1` and `0.2`. Edition `0.2` adds fixed-width integers, whose narrowing conversions may go through `checked` (`spec/18 §4`).

## Normative source

- `spec/06 §6`: `checked` and the budget.
- `spec/02 §7`: the grammar; `spec/02 §8`: where `it` may appear.
- `spec/03 §3`: the refinement language; `spec/03 §4`: opaque predicates.
- `spec/13 §3`: repairs; `spec/13 §6`: the build report; `spec/13 §7`: `[escape_budget]`.
- `spec/07 §3` rule 3: failures under the program counter.
- `spec/18 §4` rule 4: narrowing conversions.
