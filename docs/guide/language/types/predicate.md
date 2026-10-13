---
construct: "predicate: reflect and opaque"
editions: ["0.1", "0.2"]
spec: ["spec/03 §3", "spec/03 §4", "spec/02 §3", "spec/06 §6", "spec/05 §2"]
codes: [U2014, U2015, U2016, U7003]
---

# `predicate`: `reflect` and `opaque`

## What it does

A predicate is a named, pure, total boolean function that refinements and contracts can call (`spec/03 §4`). It comes in two forms, which differ in what the solver knows:

- **`reflect`:** its body is in the refinement language, and the solver sees it as a definition. A value obtains its fact by proof.
- **`opaque`:** its body is any pure expression that cannot fail, and the solver sees only a name. A value obtains its fact at run time, through `checked`, or from a capability operation's `ensures` (`spec/06 §6`, `spec/05 §2`).

```una
module demo

pub predicate reflect adult(age: Int) -> Bool = age >= 18

pub kind Adult = Int where adult(it)

pub grow(a: Adult) -> Adult = a + 1
```

## Forms

```ebnf
predicate_decl  = [ "pub" ] "predicate" ( "reflect" | "opaque" ) lower_ident
                  "(" [ params ] ")" "->" type "=" inline_or_block ;
```

This is `spec/02 §3`. The table is that of `spec/03 §4`:

| Form | Body | Visible to the solver | How a value obtains the fact |
| --- | --- | --- | --- |
| `predicate reflect` | In the refinement language, not recursive | Yes, as a definition | Proven statically |
| `predicate opaque` | Any pure, non-failing expression | No, only as an uninterpreted symbol | `checked` at run time, or a capability operation's `ensures` |

## Examples

The solver proves `grow`'s result from `adult`'s definition: `a >= 18` implies `a + 1 >= 18` (`spec/03 §4`).

An opaque predicate holds a test the solver cannot reason about, such as `is_digits`. A value gets its fact through `checked`, which counts against the escape budget (`spec/03 §4`, `spec/06 §6`):

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

## Edge cases

A `reflect` predicate can also be called in code, as an ordinary function (`spec/03 §4`):

```una
module demo

pub predicate reflect adult(age: Int) -> Bool = age >= 18

pub can_vote(age: Int) -> Bool = adult(age)
```

**`reflect` bodies are in the refinement language** (`spec/03 §4`):

```una
-- fails with U2015
module demo

pub predicate reflect digits(t: Text) -> Bool = is_digits(t)
```

**`reflect` predicates do not call themselves**, directly or through others (`spec/03 §4`):

```una
-- fails with U2015
module demo

pub predicate reflect even(n: Int) -> Bool = even(n - 2)
```

**Predicates cannot fail.** A body is pure and total (`spec/03 §4`):

```una
-- fails with U2015
module demo

pub kind Oops = | Bad

pub predicate opaque risky(n: Int) -> Bool = if n > 1 then fail Bad else true
```

**An opaque predicate is called only in `checked`**, and in the bodies of other opaque predicates. A call anywhere else would establish its fact without counting against the escape budget (`spec/03 §4`, ADR 0042):

```una
-- fails with U2016
module demo

pub predicate opaque valid(t: Text) -> Bool = t.len == 11 and is_digits(t)

pub ok(t: Text) -> Bool = valid(t)
```

Further rules:

- **Code may call a `reflect` predicate.** It is an ordinary pure function to `Bool` that the solver also knows. `spec/03 §4` restricts where opaque predicates are called, and no other predicate, so a function body may call a `reflect` predicate like any other function.
- **Naming is not establishing.** The `where` clause of a kind, and the `requires` and `ensures` clauses of a capability operation, may name an opaque predicate, since they state the fact without establishing it (`spec/03 §4`).
- **Sound by construction.** The solver knows nothing about an opaque predicate beyond facts established at run time, so it cannot introduce an inconsistency (`spec/03 §4`).
- **Budgeted.** Each `checked` counts against the module's escape budget, U7003 beyond it (`spec/06 §6`).

## Diagnostics

- [U2014](../../../../crates/una-explain/explain/U2014.md): a refinement or contract outside the refinement language.
- [U2015](../../../../crates/una-explain/explain/U2015.md): an invalid predicate body: impure, failing, recursive, or outside the refinement language for `reflect`.
- [U2016](../../../../crates/una-explain/explain/U2016.md): an opaque predicate called outside `checked`.
- [U7003](../../../../crates/una-explain/explain/U7003.md): more `checked` uses than the escape budget allows.

## Edition

Predicates are the same in editions `0.1` and `0.2`.

## Normative source

- `spec/03 §4`: predicates; `spec/03 §3`: the refinement language.
- `spec/02 §3`: the grammar.
- `spec/06 §6`: `checked`; `spec/05 §2`: a capability operation's `ensures`.
