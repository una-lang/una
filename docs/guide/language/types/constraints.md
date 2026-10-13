---
construct: Constraints on generics
editions: ["0.2"]
spec: ["spec/20 §1", "spec/20 §2", "spec/20 §3", "spec/20 §4", "spec/20 §5", "spec/20 §6", "spec/03 §2"]
codes: [U0101, U2001, U2025, U2026, U2027]
---

# Constraints on generics

## What it does

In edition `0.2`, a type parameter of a function may carry one built-in constraint: `Eq`, `Ord` or `Integer` (`spec/20`). A constraint allows operators on values of the parameter, with their built-in meaning, and restricts the types that may instantiate it. There are no user traits and no implementations: a type satisfies a constraint by what it is (`spec/20`, `spec/20 §1`).

```una
module demo

pub largest[T: Ord](xs: List[T], floor: T) -> T = xs.fold(initial: floor, step: fn(best, x) -> max(best, b: x))

pub total[T: Integer](xs: List[T]) -> Int = xs.map(each: fn(x) -> x.to_int()).sum()

pub same[T: Eq](a: T, b: T) -> Bool = a == b
```

## Forms

```ebnf
signature       = lower_ident [ constrained_params ] "(" [ params ] ")" "->" type ;
constrained_params = "[" constrained_param { "," constrained_param } "]" ;
constrained_param  = upper_ident [ ":" upper_ident ] ;
```

This is `spec/20 §2`. The constraints are those of `spec/20 §1`:

| Constraint | Allows on values of `T` | Satisfied by |
| --- | --- | --- |
| `Eq` | `==`, `!=`, and interpolation | every type whose values hold no functions |
| `Ord` | what `Eq` allows, and `<`, `<=`, `>`, `>=` | `Int`, `I8` to `U64`, `Text`, and sum kinds whose variants have no payload |
| `Integer` | what `Ord` allows, and `x.to_int()` | `Int`, `I8` to `U64` |

`Ord` includes `Eq`, and `Integer` includes `Ord`. A type parameter takes at most one constraint, and only a function's type parameters take one (`spec/20 §1` rule 1, `spec/20 §2` rules 1 and 2).

## Examples

A sum without payloads satisfies `Ord`, and so does `Text` (`spec/20 §1`):

```una
module demo

pub kind Size = Small | Large

pub smaller[T: Ord](a: T, b: T) -> T = if a < b then a else b

pub pick(a: Size, b: Size) -> Size = smaller(a, b: b)

pub texts(a: Text, b: Text) -> Text = smaller(a, b: b)
```

A private function has the least constraint its body needs inferred. A public function writes it (`spec/20 §3` rule 4):

```una
module demo

same_private[T](a: T, b: T) -> Bool = a == b

pub use_it(a: Int, b: Int) -> Bool = same_private(a, b: b)
```

An operator on a parameter needs a constraint that allows it, equality included (`spec/20 §3` rule 1):

```una
-- fails with U2026
module demo

pub same[T](a: T, b: T) -> Bool = a == b
```

## Edge cases

**Type arguments must satisfy the constraint.** A sum whose variants carry payloads does not satisfy `Ord` (`spec/20 §1` rule 3, `spec/20 §3` rule 3):

```una
-- fails with U2025
module demo

pub smaller[T: Ord](a: T, b: T) -> T = if a < b then a else b

pub kind Shape = Circle(radius: Int) | Square

pub pick(a: Shape, b: Shape) -> Shape = smaller(a, b: b)
```

**Only three constraints exist** (`spec/20 §2` rule 3):

```una
-- fails with U2027
module demo

pub same[T: Hashable](a: T, b: T) -> Bool = a == b
```

**Kinds take no constraint.** `Kind[T: Eq]` is a syntax error (`spec/20 §2` rule 1):

```una
-- fails with U0101
module demo

pub kind Box[T: Eq]
  value: T
```

**No arithmetic on `T`.** `+`, `-`, `*`, `/` and `%` stay undefined on a type parameter, constrained or not. Convert with `to_int()` and compute on `Int` (`spec/20 §1` rule 6):

```una
-- fails with U2001
module demo

pub add[T: Integer](a: T, b: T) -> T = a + b
```

Further rules:

- **Labels do not count.** `secret Int` satisfies `Integer` (`spec/20 §1` rule 4).
- **Operators keep their meaning.** `==` is structural equality and `<` the built-in order, on every type that satisfies the constraint (`spec/20 §1` rule 5).
- **Information flow.** A comparison on values of `T` carries the labels of its operands, the symbolic label of `T` included. Branching on it raises the program counter (`spec/20 §4`).
- **Verification.** In a generic body, `<` on a `T: Ord` is an uninterpreted relation with no axioms, so no law such as transitivity is assumed (`spec/20 §5`).
- **The prelude.** In edition `0.2`, `min`, `max`, `contains`, `sum` and `sort` are generic over these constraints (`spec/20 §6`).

## Diagnostics

- [U0101](../../../../crates/una-explain/explain/U0101.md): a constraint on a kind's type parameter, or one in edition `0.1`.
- [U2001](../../../../crates/una-explain/explain/U2001.md): arithmetic on values of a type parameter.
- [U2025](../../../../crates/una-explain/explain/U2025.md): a type argument that does not satisfy its parameter's constraint.
- [U2026](../../../../crates/una-explain/explain/U2026.md): an operator that the parameter's constraint does not allow.
- [U2027](../../../../crates/una-explain/explain/U2027.md): an unknown constraint name.

## Edition

Constraints exist from edition `0.2` on (`spec/20`). In edition `0.1`, type parameters are unconstrained, equality on them keeps an inferred bound, and a written constraint is a syntax error (`spec/03 §2` rule 4):

```una
-- edition 0.1
-- fails with U0101
module demo

pub same[T: Eq](a: T, b: T) -> Bool = a == b
```

## Normative source

- `spec/20 §1`: the constraints and rules 1 to 6; `spec/20 §2`: syntax; `spec/20 §3`: static semantics; `spec/20 §4`: information flow; `spec/20 §5`: verification; `spec/20 §6`: the prelude.
- `spec/03 §2` rule 4: type parameters in edition `0.1`.
