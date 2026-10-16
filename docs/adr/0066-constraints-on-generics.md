# 0066. Constraints on generics

- Status: Accepted
- Date: 2026-10-02
- Deciders: Leonardo (accepted on 2026-10-02), on a proposal by Claude (Una-1)
- Resolves: issue #37 (backlog row "Traits or constraints on generics", design D)
- Amends: `00-charter.md §5b`, `spec/01 §5`, `spec/02 §3`, `spec/03 §1`, `§2`, `spec/07 §7`, `spec/10 §12`, `spec/12 §4`, `spec/13 §4`, `§5`, `§7`, `spec/14 §1`, `spec/16 §4`, `spec/18 §6b`, `plan/backlog.md`; a new chapter `spec/20-constraints-on-generics.md`; ADR 0060 gains an amendment

## Context

Type parameters are unconstrained (`spec/03 §2` rule 4): "Operations on a type parameter are limited to equality and those provided by function parameters." What that leaves out:

- **Equality** on `T` is allowed, with an inferred bound. A generic function that compares values of `T`, directly or through its callees, cannot be instantiated with a type that holds functions (`spec/10 §12` rule 9, ADR 0024 rule 11, D-063). The bound appears in no header.
- **Ordering** (`<` and the rest) is defined for `Int`, the fixed-width types, `Text` and sums without payloads (`spec/03 §1`, `spec/18 §2`), and never for `T` (U2001). There is no generic `min`, `max` or sort.
- **Numeric functions.** `sum`, `min`, `max` and `abs` take `Int` only. A `List[U8]` is converted first, and "numeric generic functions wait for constraints on generics" (`spec/18 §6b`).
- **Overflow.** Arithmetic on a fixed-width type has an `overflow` obligation (`spec/18 §5`). On a `T` that may be any integer type, it could not be discharged once for all instantiations.

What the design must respect:

- **No operator overloading and no inheritance** (charter §6). `==` and `<` keep their built-in meaning.
- **One package.** `una.toml` has no dependencies, so the only code a package does not own is `std`.
- **Labels.** A comparison is labeled with the join of its operands' labels (`spec/07 §7` rule 3). A generic body sees symbolic labels `ℓ_T` (rule 8). The prelude functions that read values join every label of their arguments (ADR 0051), and precise labels for user generics are open (#92).
- **Verification.** A generic function is verified once, with one uninterpreted sort per type parameter (`spec/12 §4`), in a quantifier-free fragment. Laws with quantifiers are v0.3 (row #50; `law` and `all` reserved).
- **Native code.** A generic function is compiled once, every value a word (`crates/una-codegen`, ADR 0062 rule 11). `Int` is a tagged word, while a fixed-width value is a machine integer (`spec/16 §4`, `spec/18 §7`). So a word of type `T` does not say how to compare or convert it.
- **Effects.** An effectful function type is second-class (`spec/05 §6`, ADR 0062), so it can be no field of a dictionary value.
- **Reserved words.** `trait` and `impl` are "reserved for v0.2 (constraints on generics)" (`spec/01 §5`).

The proposal asked eight questions, G1 to G8, each with options and a recommendation A. Leonardo answered them on 2026-10-02, each with option A. The other options stay below, marked as not chosen. Everything applies in edition `0.2` only.

## Decision

The decision, in one paragraph: three built-in constraints, `Eq`, `Ord` and `Integer`, written `[T: Ord]` on a function's type parameters. No user traits and no `impl` in v0.2. A type satisfies a constraint by what it is, as the rules of today say. A constraint allows the operators it names on `T`, which keep their built-in meaning. Arithmetic on `T` is not allowed, so no overflow obligation is generic, and `Integer` gives `to_int()` instead. Native code passes a hidden type descriptor for each constrained parameter, and stays compiled once. Verification adds no laws. The prelude gains `min`, `max`, `sort` and `sum` over constrained types.

### G1. User traits or a closed set

**A (chosen): a closed set of built-in constraints, no user traits in v0.2.**

| Constraint | Allows on `T` | Satisfied by |
| --- | --- | --- |
| `Eq` | `==`, `!=` | every type whose values hold no functions (today's rule) |
| `Ord` | what `Eq` allows, and `<`, `<=`, `>`, `>=` | `Int`, `I8` to `U64`, `Text`, sums whose variants have no payload |
| `Integer` | what `Ord` allows, and `x.to_int()` | `Int`, `I8` to `U64` |

1. `Ord` includes `Eq`, and `Integer` includes `Ord`. This is a fixed order among three built-ins, not inheritance a program can declare.
2. A type parameter written with a constraint satisfies that constraint and the ones it includes.
3. `trait` and `impl` stay reserved, with the message "reserved for a later version (user traits)".

**B:** user traits with implementations, as `trait Shape` with `area(self) -> Int` and `impl Shape for Circle`. Methods are named functions, never operators (charter §6). Coherence rule: an `impl` lives in the module of its trait or of its type, and one per trait and type. Not chosen for v0.2: it needs dispatch for user methods, rules for effects and contracts of methods, and labels of their results. Nothing in the roadmap needs it before v0.3, and the syntax of A extends to it.

**C:** no constraints. Generic code keeps taking function parameters, and the prelude adds functions per integer type. Not chosen: twelve copies of `sum`, `min` and `max`, and still no generic sort.

### G2. The surface

**A (chosen): `[T: C]` on the type parameters of functions.**

```una
pub sort[T: Ord](xs: List[T]) -> List[T]

pub largest[T: Ord](xs: List[T], floor: T) -> T
=
  xs.fold(initial: floor, step: fn(best, x) -> max(best, b: x))

pub total[T: Integer](xs: List[T]) -> Int = xs.map(each: fn(x) -> x.to_int()).sum()
```

1. `type_params = "[" type_param { "," type_param } "]"` and `type_param = upper_ident [ ":" upper_ident ]`. One constraint per parameter is enough, since the three are ordered.
2. Constraints are written on functions only. A kind's type parameters stay unconstrained: a kind that holds values of `T` does not need to compare them.
3. `Eq`, `Ord` and `Integer` are names of the prelude in the upper namespace. A user kind with one of these names is U1002, as for `I8` (`spec/18 §1`).

**B:** a `where` clause after the header, as `where T is Ord`. Not chosen: a second place for type information, and `where` already starts a refinement.

### G3. Equality on unconstrained type parameters

**A (chosen): `==` on `T` needs `T: Eq` in edition 0.2.**

1. In edition `0.2`, `==` or `!=` on values of an unconstrained type parameter is a diagnostic at the operator, with a repair that adds `: Eq`.
2. A private function gets its constraints inferred, as for `uses` (ADR 0007) and `uses of` (ADR 0062): `Eq` when its body compares values of `T` or calls a function that needs `T: Eq`, and likewise for `Ord` and `Integer`. Public headers write them, so the bound is visible where reviewers read.
3. The inferred bound of D-063 becomes this written constraint. Edition `0.1` keeps D-063 unchanged.

**B:** keep equality on every `T`, with D-063's inferred bound, and `Eq` only as documentation. Not chosen: the one constraint that exists stays invisible in public headers.

### G4. Dispatch and native lowering

**A (chosen): a hidden type descriptor per constrained parameter.**

1. A function with a constrained parameter `T` takes one extra hidden argument per such parameter: a word that describes the type the call instantiates it with. The descriptor is a constant at each call with a known type, and is passed on when the caller's own `T` instantiates it.
2. The descriptor says how to compare and convert words of `T`:
   - tagged `Int`;
   - a fixed-width type and its width and signedness;
   - `Text`;
   - a sum without payloads, compared by tag;
   - for `Eq`, structural equality, as `una_rt_equal` does today.
3. The function is still compiled once, which keeps ADR 0062 rule 11 and the uniform representation of ADR 0055. Unconstrained parameters take no descriptor.
4. The interpreter needs nothing: its values carry their own form.

**B:** monomorphize constrained functions per instantiation. Each copy knows its types. Not chosen: code grows with the instantiations, and it breaks the "compiled once" rule that higher-order functions rely on.

**C:** dictionaries of functions, as a record of closures per constraint. Not chosen: a record of function values is what the second-class rule restricts, and a descriptor is smaller for a closed set.

### G5. Labels

**A (chosen): the rules of today, unchanged.**

1. A comparison or `to_int()` on `T` is labeled with the join of its operands' labels, the symbolic `ℓ_T` included (`spec/07 §7` rules 3 and 8). Under a raised program counter the usual U4001 to U4003 rules apply.
2. The new prelude functions read values, so their results join every label of their arguments, element types included, as ADR 0051 keeps for `min`, `max`, `sum` and `contains`. For `sort`, every position of the result carries the join of the labels of all elements and of the list, since the order reveals comparisons.
3. Precise labels for user generics stay with #92.

**B:** position-precise labels for `sort` (each element keeps its own label). Not chosen: where an element ends up depends on every other element, which would leak.

### G6. Verification

**A (chosen): no laws in v0.2.**

1. In a generic body, `<` and the others on a `T: Ord` are uninterpreted relations of `T`'s sort, with no axioms. `to_int()` on a `T: Integer` is an uninterpreted function into `Int`.
2. At a call, the callee's contract is instantiated with the type arguments as today. When `T` is `Int` or a fixed-width type, its comparisons are the integer ones, and `to_int()` is the identity, with the range of the type.
3. The prelude's contracts are hypotheses, as for all of `std`:
   - `min` and `max` keep their `ensures` over `T`;
   - `sort` ensures `result.len == xs.len`.
   Sortedness and permutation need quantifiers, which are v0.3.
4. Laws such as transitivity, or `ensures` laws on constraints, stay with row #50 in v0.3.

**B:** ground instances of order axioms (reflexivity, antisymmetry and transitivity over the terms of each query). More proofs of generic code, but a new kind of axiom in the encoding, with its own cost and soundness review. Deferred.

### G7. Effects and failures

**A (chosen): constraint operations are pure and never fail.** `==`, `<` and `to_int()` on `T` have no effects and cannot fail, as on concrete types, so ADR 0062 is unchanged. Effectful operations on `T` keep coming from function parameters, with `uses of` and `fails of`.

**B:** none needed while there are no user traits (G1 B would have to decide it).

### G8. The prelude and what it unblocks

**A (chosen): the prelude of edition `0.2` generalizes its functions.**

| Function | Edition `0.2` signature | Contract |
| --- | --- | --- |
| `min`, `max` | `min[T: Ord](a: T, b: T) -> T` | as today, over `T` |
| `contains` | `contains[T: Eq](xs: List[T], item: T) -> Bool` | none, as today |
| `sum` | `sum[T: Integer](xs: List[T]) -> Int` | none, as today; the total is an `Int`, so it never overflows |
| `sort` (new) | `sort[T: Ord](xs: List[T]) -> List[T]` | `result.len == xs.len`; ascending and stable |
| `abs` | unchanged, `abs(x: Int) -> Int` | as today; `abs` of a fixed-width value can overflow |

1. Calls with `Int` arguments keep their meaning and contracts.
2. A `List[U8]` sums without conversion. `max` of two `I32` values is an `I32`, and records with no function fields work with `contains`, as today, with the bound now written.
3. Arithmetic on `T` stays out (G1 A), so a user's `sum[T]` converts with `to_int()`, and the overflow rules of `spec/18` stay on concrete types.

Rules that follow:

9. **Edition.** In edition `0.2` the constraint syntax parses and the new prelude signatures apply. Edition `0.1` is unchanged, `trait` and `impl` stay reserved in both, and their edition `0.2` message changes (G1 rule 3).
10. **Instantiation.** A type argument that does not satisfy its parameter's constraint is a diagnostic at the call. The checker reports it with the type and the constraint. Refined and second-class types stay excluded as type arguments (U2002, U3016).
11. **Diagnostics** (assigned by the coordinator):
    - U2025: a type argument that does not satisfy a constraint (rule 10);
    - U2026: an operation on a type parameter that its constraint does not allow, `==` without `Eq` among them (G3), with a repair that adds the constraint;
    - U2027: an unknown constraint name after `:`.
12. **The report.** A function's constraints appear in its contract: `una.contract/1` gains a `constraints` list, an additive key that changes the meaning of no other key, so the schema keeps its version. The authority and information sections of the report are unchanged.
13. **Scope.** This ADR opens constraints on generics for v0.2: charter §5b gains them, and row #37 moves to "Opened for v0.2", as Leonardo decided on 2026-10-02.
14. **The specification** is `spec/20`.

## Consequences

- Generic code can compare, order and sum values of any integer type, and the prelude gets `sort` and generic `min`, `max` and `sum`.
- The equality bound on type parameters becomes visible in public headers.
- No new kind of declaration, no dispatch on user code, no change to the information-flow rules or to verification's fragment.
- Native code passes descriptors to constrained functions only. Programs without constraints compile as today.
- Deferred: user traits and implementations, arithmetic on `T`, laws of constraints, constraints on kinds.

## Alternatives considered

Besides the B and C options under each decision:

- **Operators defined by traits.** Excluded by charter §6.
- **Type classes with default methods and superclasses.** Not chosen: inheritance is a non-goal (charter §6), and nothing in v0.2 needs defaults.
- **Arithmetic on `T: Integer`** with an overflow obligation per instantiation, checked at each call. Not chosen for v0.2: the obligation would depend on values inside the generic body, which no caller sees, so it could not be discharged at the call.

Note (2026-10-16): the type descriptor gains a kind for `F64` (ADR 0114 rule 9.4): equality by bits, order by the totalOrder key.
