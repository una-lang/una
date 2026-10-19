---
construct: Constraints on generics
editions: ["0.2"]
spec: ["spec/20 §1", "spec/20 §2", "spec/20 §3", "spec/20 §4", "spec/20 §5", "spec/20 §6", "spec/03 §2"]
codes: [U0101, U2001, U2025, U2026, U2027]
---

# Constraints on generics

## What it does

From edition `0.2`, a type parameter of a function, a placeholder such as `T` that stands for any type the caller uses, may carry one built-in constraint: `Eq`, `Ord` or `Integer`. A constraint allows operators on values of the parameter, with their built-in meaning, and restricts the types that may instantiate it. There are no user traits and no implementations (ways, in other languages, to declare your own constraints and say which types meet them): a type satisfies a constraint by what it is.

Learn more: [Generic kinds and constraints](/learn/kinds/#s4-4).

```una
module shop

pub largest[T: Ord](prices: List[T], floor: T) -> T = prices.fold(initial: floor, step: fn(best, price) -> max(best, b: price))

pub total[T: Integer](prices: List[T]) -> Int = prices.map(each: fn(price) -> price.to_int()).sum()

pub same[T: Eq](a: T, b: T) -> Bool = a == b
```

## Forms

In the grammar below, each rule gives a name, `=`, what the name stands for and `;`, quoted words are written as shown, NEWLINE, INDENT and DEDENT stand for a line break and for one more or one less level of indentation, `[ ]` marks an optional part, `{ }` a part that may repeat, `|` separates alternatives, `( )` groups them, and `(* *)` holds a comment.

```ebnf
signature       = lower_ident [ constrained_params ] "(" [ params ] ")" "->" type ;
constrained_params = "[" constrained_param { "," constrained_param } "]" ;
constrained_param  = upper_ident [ ":" upper_ident ] ;
```

The constraints:

| Constraint | Allows on values of `T` | Satisfied by |
| --- | --- | --- |
| `Eq` | `==`, `!=`, and interpolation | every type whose values hold no functions |
| `Ord` | what `Eq` allows, and `<`, `<=`, `>`, `>=` | `Int`, `I8` to `U64`, `Text`, and sum kinds whose variants have no payload |
| `Integer` | what `Ord` allows, and `x.to_int()` | `Int`, `I8` to `U64` |

`Ord` includes `Eq`, and `Integer` includes `Ord`. A type parameter takes at most one constraint, and only a function's type parameters take one.

## Examples

A sum without payloads satisfies `Ord`, and so does `Text`:

```una
module shop

pub kind Size = Small | Large

pub smaller[T: Ord](a: T, b: T) -> T = if a < b then a else b

pub pick(a: Size, b: Size) -> Size = smaller(a, b: b)

pub texts(a: Text, b: Text) -> Text = smaller(a, b: b)
```

A private function has the least constraint its body needs inferred. A public function writes it:

```una
module shop

same_private[T](a: T, b: T) -> Bool = a == b

pub use_it(a: Int, b: Int) -> Bool = same_private(a, b: b)
```

An operator on a parameter needs a constraint that allows it, equality included:

```una
-- fails with U2026
module shop

pub same[T](a: T, b: T) -> Bool = a == b
```

## Edge cases

**Type arguments must satisfy the constraint.** A sum whose variants carry payloads does not satisfy `Ord`:

```una
-- fails with U2025
module shop

pub smaller[T: Ord](a: T, b: T) -> T = if a < b then a else b

pub kind Parcel = Tube(length: Int) | Envelope

pub pick(a: Parcel, b: Parcel) -> Parcel = smaller(a, b: b)
```

**Only three constraints exist**:

```una
-- fails with U2027
module shop

pub same[T: Hashable](a: T, b: T) -> Bool = a == b
```

**Kinds take no constraint.** `Kind[T: Eq]` is a syntax error:

```una
-- fails with U0101
module shop

pub kind Box[T: Eq]
  value: T
```

**No arithmetic on `T`.** `+`, `-`, `*`, `/` and `mod` stay undefined on a type parameter, constrained or not. Convert with `to_int()` and compute on `Int`:

```una
-- fails with U2001
module shop

pub add[T: Integer](a: T, b: T) -> T = a + b
```

Further rules:

- **Labels do not count.** `secret Int` satisfies `Integer`.
- **Operators keep their meaning.** `==` is structural equality and `<` the built-in order, on every type that satisfies the constraint.
- **Information flow.** A comparison on values of `T` carries the labels of its operands, the symbolic label of `T` included, a stand-in for whatever label the caller's type carries. Branching on it raises the program counter, the label of the conditions that decide whether a line runs (see [Branches and failures: implicit flows](/learn/information-flow/#s17-3)).
- **Verification.** In a generic body, the verifier knows nothing about `<` on a `T: Ord` but its name, so no law such as transitivity (`a < b` and `b < c` give `a < c`) is assumed.
- **The prelude.** In edition `0.2`, `min`, `max`, `contains`, `sum` and `sort` are generic over these constraints.

## Diagnostics

- [U0101](../../../../crates/una-explain/explain/U0101.md): a constraint on a kind's type parameter, or one in edition `0.1`.
- [U2001](../../../../crates/una-explain/explain/U2001.md): arithmetic on values of a type parameter.
- [U2025](../../../../crates/una-explain/explain/U2025.md): a type argument that does not satisfy its parameter's constraint.
- [U2026](../../../../crates/una-explain/explain/U2026.md): an operator that the parameter's constraint does not allow.
- [U2027](../../../../crates/una-explain/explain/U2027.md): an unknown constraint name.

## Edition

Constraints exist from edition `0.2` on. In edition `0.1`, type parameters are unconstrained, equality on them keeps an inferred bound, and a written constraint is a syntax error:

```una
-- edition 0.1
-- fails with U0101
module shop

pub same[T: Eq](a: T, b: T) -> Bool = a == b
```

## Normative source

- `spec/20 §1`: the constraints and rules 1 to 6; `spec/20 §2`: syntax; `spec/20 §3`: static semantics; `spec/20 §4`: information flow; `spec/20 §5`: verification; `spec/20 §6`: the prelude.
- `spec/03 §2` rule 4: type parameters in edition `0.1`.
