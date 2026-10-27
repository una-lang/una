---
construct: "predicate: reflect and opaque"
editions: ["0.1", "0.2", "0.3"]
spec: ["spec/03 §3", "spec/03 §4", "spec/02 §3", "spec/06 §6", "spec/05 §2", "spec/13 §7"]
codes: [U2014, U2015, U2016, U7003]
---

# `predicate`: `reflect` and `opaque`

## What it does

A predicate is a named, pure (it acts on nothing outside the program), total (it gives a result for every input, without failing) boolean function that refinements and contracts can call. Refinements are the conditions a type adds to its values; contracts are a function's `requires` and `ensures` clauses. It comes in two forms, which differ in what the solver, the program that decides each fact the compiler must prove, knows:

- **`reflect`:** its body is in the refinement language, the small set of expressions the solver can decide, and the solver sees it as a definition. A value obtains its fact by proof.
- **`opaque`:** its body is any pure expression that cannot fail, and the solver sees only a name. A value obtains its fact at run time, through [`checked`](../errors/checked.md), or from a capability operation's `ensures`.

```una
module shop

pub predicate reflect adult(age: Int) -> Bool = age >= 18

pub kind Adult = Int where adult(it)

pub grow(a: Adult) -> Adult = a + 1
```

Learn more: [Refined kinds and predicates](/learn/contracts/refined-kinds-and-predicates/).

## Forms

In the grammar below, each rule gives a name, `=`, what the name stands for and `;`, quoted words are written as shown, NEWLINE, INDENT and DEDENT stand for a line break and for one more or one less level of indentation, `[ ]` marks an optional part, `{ }` a part that may repeat, `|` separates alternatives, `( )` groups them, and `(* *)` holds a comment.

```ebnf
predicate_decl  = [ "pub" ] "predicate" ( "reflect" | "opaque" ) lower_ident
                  "(" [ params ] ")" "->" type "=" inline_or_block ;
```

The kinds of predicate:

| Form | Body | Visible to the solver | How a value obtains the fact |
| --- | --- | --- | --- |
| `predicate reflect` | In the refinement language, not recursive | Yes, as a definition | Proven statically |
| `predicate opaque` | Any pure, non-failing expression | No, only as a name whose meaning it does not know | `checked` at run time, or a capability operation's `ensures` |

## Examples

The solver proves `grow`'s result from `adult`'s definition: `a >= 18` implies `a + 1 >= 18`.

An opaque predicate holds a test the solver cannot reason about, such as `is_digits`. A value gets its fact through [`checked`](../errors/checked.md), which counts against the escape budget, the number of run-time tests each module may use in place of a proof:

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

## A nonlinear relation checked at run time

The refinement language does not let the solver derive a general nonlinear equation such as `area = side * side`. An opaque predicate can express that runtime test; `checked` establishes the refined kind only when the test succeeds. With `checked_per_module = 1`, the package spends one escape for this check.

`una.toml`

```toml
[package]
name = "demo"
version = "0.1.0"
edition = "0.3"

[escape_budget]
checked_per_module = 1
```

`src/demo.una`

```una
module demo

pub predicate opaque square_pair(value: Pair) -> Bool = value.area == value.side * value.side

pub kind Pair
  side: Int
  area: Int

pub kind Square = Pair where square_pair(it)

pub kind ValidationError = | WrongArea

pub validate(value: Pair) -> Square
  fails ValidationError
=
  checked(value, square_pair(it)) else fail WrongArea

example "valid"
  do validate(Pair(side: 3, area: 9))
  expect result.side == 3

example "invalid"
  do validate(Pair(side: 3, area: 8))
  expect fails WrongArea
```

The solver knows only the predicate's name; it does not prove or learn the square equation. Each `checked` counts against the module's budget.

## Edge cases

A `reflect` predicate can also be called in code, as an ordinary function:

```una
module shop

pub predicate reflect adult(age: Int) -> Bool = age >= 18

pub can_vote(age: Int) -> Bool = adult(age)
```

**`reflect` bodies are in the refinement language**:

```una
-- fails with U2015
module shop

pub predicate reflect digits(t: Text) -> Bool = is_digits(t)
```

**`reflect` predicates do not call themselves**, directly or through others:

```una
-- fails with U2015
module shop

pub predicate reflect even(n: Int) -> Bool = even(n - 2)
```

**Predicates cannot fail.** A body is pure and total:

```una
-- fails with U2015
module shop

pub kind StockError = | Unknown

pub predicate opaque restockable(n: Int) -> Bool = if n > 1 then fail Unknown else true
```

**An opaque predicate is called only in `checked`**, and in the bodies of other opaque predicates. A call anywhere else would establish its fact without counting against the escape budget:

```una
-- fails with U2016
module shop

pub predicate opaque valid_phone(raw: Text) -> Bool = raw.len == 11 and is_digits(raw)

pub looks_valid(raw: Text) -> Bool = valid_phone(raw)
```

Further rules:

- **Naming is not establishing.** The `where` clause of a kind, and the `requires` and `ensures` clauses of a capability operation, may name an opaque predicate, since they state the fact without establishing it.
- **Sound by construction.** The solver knows nothing about an opaque predicate beyond facts established at run time, so it cannot introduce an inconsistency.
- **Budgeted.** Each `checked` counts against the module's escape budget, U7003 beyond it.

## Diagnostics

- [U2014](../../../../crates/una-explain/explain/U2014.md): a refinement or contract outside the refinement language.
- [U2015](../../../../crates/una-explain/explain/U2015.md): an invalid predicate body: impure, failing, recursive, or outside the refinement language for `reflect`.
- [U2016](../../../../crates/una-explain/explain/U2016.md): an opaque predicate called outside `checked`.
- [U7003](../../../../crates/una-explain/explain/U7003.md): more `checked` uses than the escape budget allows.

## Edition

Predicates are the same in editions `0.1` and `0.2`, and editions `0.3` and `0.4` keep the rules of edition `0.2` (`spec/13 §7`).

## Normative source

- `spec/03 §4`: predicates; `spec/03 §3`: the refinement language.
- `spec/02 §3`: the grammar.
- `spec/06 §6`: `checked`; `spec/05 §2`: a capability operation's `ensures`.
- Decision record: [ADR 0042](../../../adr/0042-opaque-predicates-only-in-checked.md), for opaque predicates only in `checked`.
