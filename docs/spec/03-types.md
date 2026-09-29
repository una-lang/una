# 03. Types

Status: Normative, v0.1.

## 1. Built-in types

| Type | Values | Notes |
| --- | --- | --- |
| `Int` | Mathematical integers | Unbounded (ADR 0004). No overflow. |
| `Nat` | `Int where it >= 0` | Prelude alias |
| `Bool` | `true`, `false` | |
| `Text` | Unicode strings | `t.len` counts Unicode scalar values |
| `Unit` | `unit` | |
| `List[T]` | `[a, b, c]` | Immutable. `xs.len` |
| `Option[T]` | `Some(value: T)`, `None` | |
| `Result[T, E]` | `Ok(value: T)`, `Err(failure: E)` | Produced by `attempt` (`spec/06 §5`) |
| `Hashed` | Opaque digests | Only produced by host capabilities (`spec/14`) |
| `fn(A, B) -> C` | Lambdas and named functions | Not comparable, not storable in kinds if effectful (`spec/05 §6`) |

Equality `==` and `!=` are structural and defined for every type except function types (U2001). Ordering `<`, `<=`, `>`, `>=` is defined for `Int`, `Text` (lexicographic by scalar value) and sum kinds whose variants have no payload (declaration order).

## 2. Kinds

A `kind` declaration introduces one of four forms (`spec/02 §2`):

```una
kind Seats = Int in 0..5000                                  -- refinement with range sugar
kind Phone = Text where valid_phone(it)                      -- refinement
kind Availability = SoldOut | Few | Open                     -- sum
kind Lookup = Found(registration: Registration) | Missing    -- sum with payload
kind Event                                                   -- record
  name: Text
  remaining: Seats
kind NonEmpty[T] = List[T] where it.len > 0                  -- generic refinement
```

Rules:

1. Records and sums are **nominal**: two declarations with the same shape are different types.
2. Aliases and refinements are **structural over their base**: `Seats` is `Int` refined by `0 <= it and it <= 5000`. A `Seats` value may be used where `Int` is expected; an `Int` used where `Seats` is expected produces a proof obligation (`spec/12`).
3. A refinement of a refined kind conjoins both predicates.
4. Generic parameters are unconstrained in v0.1 (no traits). Operations on a type parameter are limited to equality and those provided by function parameters.
5. Recursive kinds are allowed only through `List` or `Option` fields (U2002 otherwise), so every value is finite.

## 3. Refinements

A refinement predicate is written in the **refinement language** RL, a subset of expressions that the verifier can reason about (`spec/12 §2`):

- integer and boolean literals;
- `it`, parameters of the enclosing function, and field access on them (`it.remaining`, `a.b.c`);
- `.len` on `Text` and `List` values;
- `+`, `-`, unary `-`; `*` where at least one operand is a literal; `/` and `mod` by a nonzero literal;
- `==`, `!=`, `<`, `<=`, `>`, `>=`, `in range`, `is Variant`;
- `and`, `or`, `not`, `implies`;
- calls to predicates (§4).

An expression outside RL in a `where`, `requires` or `ensures` clause is an error (U2014), reported with the offending sub-expression.

Range sugar: `T in a..b` means `T where a <= it and it <= b`; `T in a..<b` means `T where a <= it and it < b`.

## 4. Predicates

Predicates are named, pure, total boolean functions usable in RL.

| Form | Body | Visible to the solver | How a value obtains the fact |
| --- | --- | --- | --- |
| `predicate reflect` | MUST be in RL and MUST NOT call itself directly or indirectly (U2015) | Yes, as a definition | Proven statically |
| `predicate opaque` | Any pure, non-failing expression (U2015 otherwise) | No, only as an uninterpreted symbol | `checked` at runtime (`spec/06 §6`) or a capability operation's `ensures` (`spec/05 §2`) |

```una
predicate reflect adult(age: Int) -> Bool = age >= 18

predicate opaque valid_phone(t: Text) -> Bool = t.len == 11 and is_digits(t)
```

Opaque predicates are sound by construction: the solver knows nothing about them beyond facts established at runtime, so they cannot introduce inconsistency. Their bodies run in the interpreter when `checked` evaluates them.

## 5. Labeled types

Any type may carry a label prefix: `secret[PII] Phone`, `internal Hashed`. An unlabeled type is `public`. Labels, the lattice and flow rules are defined in `spec/07`.

## 6. Subtyping

Subtyping is used at every point where a value flows into a position with a declared type (argument, return, field, `let` with annotation).

| Relation | Condition |
| --- | --- |
| `{v: B \| p} <: {v: B \| q}` | Obligation `p implies q` under the path context (`spec/12 §3`) |
| `l1 T <: l2 T` | `l1 ⊑ l2` (`spec/07 §2`) |
| function types | Contravariant parameters, covariant result, declared uses and fails of the subtype included in the supertype's |

Capabilities are not values and have no subtyping; passing them is governed by sufficiency (`spec/10 §2`).

There are no implicit conversions between distinct nominal kinds or between `Int` and `Text`.

## 7. Type inference

- Function, predicate, handler and capability signatures are fully annotated. Private functions MAY omit `uses` and `fails` (`spec/04 §5`); types are never omitted.
- `let` bindings infer their type from the right-hand side. A binding whose right-hand side is in RL gets the selfified type `{v: B | v == e}`, so facts flow into later obligations.
- Lambda parameter types are taken from the expected function type; without one, annotations are required (U2010).
- Generic arguments are inferred at call sites by unification. Refinements are never inferred beyond selfification and path conditions, which keeps verification predictable.
