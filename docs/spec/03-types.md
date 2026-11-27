# 03. Types

Status: Normative, v0.1.

(v0.5, ADR 0137.) **Binding words.** In edition `0.5`, a binding of the surface language that this chapter writes `let` is written `const`, and one it writes `let edit` is written `var` (`spec/02 §6`, `spec/17 §2`); the rules are unchanged. A Core `let` (`spec/09`) stays. Its Una blocks of edition `0.5` start with the line `-- edition 0.5` and use these words; its other blocks keep the words of their editions.

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

(v0.2, ADR 0057.) Edition `0.2` adds the fixed-width integer types `I8`, `I16`, `I32`, `I64`, `U8`, `U16`, `U32` and `U64`, with overflow obligations (`spec/18`). `Int` stays unbounded.

(v0.2, ADR 0079.) In edition `0.2`, a kind, a variant or a capability cannot take the name of a type of this table, `Int`, `Nat`, `Bool`, `Text`, `Unit`, `Hashed`, `List`, `Option` or `Result` (U1002), as it cannot take a fixed-width name (`spec/18 §1`) or `Map` (`spec/21 §1`). In edition `0.1` such a declaration shadows the built-in type in its module.

(v0.2, ADR 0069.) Edition `0.2` adds `Map[K, V]`, a keyed collection whose keys satisfy `Ord`, ordered by its keys (`spec/21`).

Equality `==` and `!=` are structural and defined for every type except function types (U2001). Ordering `<`, `<=`, `>`, `>=` is defined for `Int`, `Text` (lexicographic by scalar value) and sum kinds whose variants have no payload (declaration order). (v0.2, ADR 0066.) In edition `0.2`, the fixed-width types are ordered too (`spec/18 §2`), and on a type parameter equality and ordering need a constraint (`spec/20`).

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
4. Generic parameters are unconstrained in v0.1 (no traits). Operations on a type parameter are limited to equality and those provided by function parameters. (v0.2, ADR 0066.) In edition `0.2`, a function's type parameter may carry the constraint `Eq`, `Ord` or `Integer`, which allows equality, ordering and `to_int()`, and equality needs `Eq` (`spec/20`).
5. Recursive kinds are allowed only through `List` or `Option` fields (U2002 otherwise), so every value is finite.
   - (v0.2, ADR 0060.) In edition `0.2`, a kind may contain itself through any field when it has a finite value, as `kind Expr = Lit(value: Int) | Add(left: Expr, right: Expr)`. A kind without one, whose every variant (or, for a record, some field) needs a value of a kind without one, is U2002. In a recursion, the kinds of the recursion take only type parameters as arguments (U2002 otherwise). `spec/10 §12` rule 8 states both rules. Edition `0.1` keeps the rule above.

## 3. Refinements

A refinement predicate is written in the **refinement language** RL, a subset of expressions that the verifier can reason about (`spec/12 §2`):

- integer, boolean and text literals (text literals are distinct constants, `spec/12 §4`);
- `it`, parameters of the enclosing function, and field access on them (`it.remaining`, `a.b.c`);
- `.len` on `Text` and `List` values; (v0.5, ADR 0119 V5) `.len` on every collection of `spec/28`, `.size` on a `BitSet`, `s.has(item: x)` on a `Set`, a `HashSet` and a `BitSet`, and `m.has(key: k)` on a `HashMap` (`spec/28 §11`);
- `+`, `-`, unary `-`; `*` where at least one operand is a literal; `/` and `mod` by a nonzero literal;
- `==`, `!=`, `<`, `<=`, `>`, `>=`, `in range`, `is Variant`;
- `and`, `or`, `not`, `implies`;
- calls to predicates (§4).

An expression outside RL in a `where`, `requires` or `ensures` clause is an error (U2014), reported at the outermost offending sub-expression. Outside RL, in particular, are the ordering of `Text`, `is` with a literal, comparing with a variant by `==`, `unit` and interpolated text; the diagnostic carries an edit from `x == V` to `x is V` and from `x is 1` to `x == 1` (ADR 0024).

Range sugar: `T in a..b` means `T where a <= it and it <= b`; `T in a..<b` means `T where a <= it and it < b`.

**Refined type arguments** (ADR 0040). A refined kind, or a type with range sugar, written as a type argument is ill-formed (U2002), including inside a kind definition or a function type: `Option[Pos]`, `List[Int in 0..5]`, `Result[Pos, E]` and `Box[Pos]` are rejected, and a type that contains one, as `Option[List[Pos]]`, is reported at the innermost refined type argument. A type argument inferred from a value of a refined kind is the base of that kind, so the result does not carry the refinement. The parameters and result of a function type are not type arguments: `fn(Pos) -> Int` is well-formed (ADR 0037).

(v0.3, ADR 0103.) **Refined elements.** In edition `0.3` a refined kind, a type with range sugar, or a type that contains one MAY be a type argument: `List[Pos]`, `Option[Int in 0..5]`, `Map[Text, Pos]`, `Box[Pos]`, in signatures, fields, function types, annotations and written type arguments. Every value at such a position satisfies its refinement: each flow of a value into it is an obligation (`spec/12 §3`), and each value read from it carries the refinement as a fact. A refined type satisfies a constraint of `spec/20 §1` when its base does. Editions `0.1` and `0.2` keep the rule above.

**Refined function kinds** (ADR 0045). A `where` clause on a kind whose base is a function type, as `kind F = fn(Int) -> Int where good(it)`, is ill-formed (U2002 at the clause), with a message that names the kind: Core IR has no refinement over a function base (`spec/09 §2`). The parameters and the result of the function type may be refined instead. Range sugar is not such a clause: in `kind F = fn(Int) -> Int in 0..5` the range binds to the result type. An alias of such a kind, `kind G = F`, is reported once, at the clause of `F`.

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

**Where an opaque predicate is called** (ADR 0042). An opaque predicate is called only as the predicate of `checked` (`spec/06 §6`) and in the bodies of other opaque predicates. The `where` clause of a kind and the `requires` and `ensures` clauses of a capability operation MAY name it, since they state the fact without establishing it. Any other call, in code, in a clause of a function or handler, in the body of a `reflect` predicate or in an example, is U2016 at the call; the help proposes `checked(value, p(it)) else fail <Variant>`. Every opaque fact of code is thus counted against the escape budget and listed in the build report.

## 5. Labeled types

Any type may carry a label prefix: `secret[PII] Phone`, `internal Hashed`. An unlabeled type is `public`. Labels, the lattice and flow rules are defined in `spec/07`.

## 6. Subtyping

Subtyping is used at every point where a value flows into a position with a declared type (argument, return, field, `let` with annotation).

| Relation | Condition |
| --- | --- |
| `{v: B \| p} <: {v: B \| q}` | Obligation `p implies q` under the path context (`spec/12 §3`) |
| `l1 T <: l2 T` | `l1 ⊑ l2` (`spec/07 §2`) |
| (v0.3, ADR 0103) `C[σ] <: C[τ]` at a refined nested position | the refinement of each position of `τ` that `σ` does not carry, as an obligation over a fresh binder: covariant through `List`, `Option`, `Result`, `Map` and a covariant kind parameter, contravariant through a function's parameters, both ways through an invariant kind parameter (ADR 0046) |
| function types | Contravariant parameters, covariant result, declared uses and fails of the subtype included in the supertype's |

Capabilities are not values and have no subtyping; passing them is governed by sufficiency (`spec/10 §2`).

There are no implicit conversions between distinct nominal kinds or between `Int` and `Text`.

## 7. Type inference

- Function, predicate, handler and capability signatures are fully annotated. Private functions MAY omit `uses` and `fails` (`spec/04 §5`); types are never omitted.
- `let` bindings infer their type from the right-hand side. A binding whose right-hand side is in RL gets the selfified type `{v: B | v == e}`, so facts flow into later obligations.
- Lambda parameter types are taken from the expected function type; without one, annotations are required (U2010).
- Generic arguments are inferred at call sites by unification. Refinements are never inferred beyond selfification and path conditions, which keeps verification predictable.
- (v0.3, ADR 0103 rule 5.) A type argument inferred from values keeps a refinement only when every value that determines it has that refinement at that position, as the same refined kind or range, and the parameter is at no contravariant or invariant position of the callee's result. Otherwise it is the base, as in editions `0.1` and `0.2`. A join (`if`, `when`, `if none use`, `if error use`) keeps a nested refinement only when every branch has it. Map lookups (`has`, `get`, `remove`, `without`) take the key at the base of the key type (`spec/21 §2`).
