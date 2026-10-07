# 20. Constraints on generics

Status: Normative, v0.2 (ADR 0066).

A type parameter of a function may carry one of three built-in **constraints**: `Eq`, `Ord` or `Integer`. A constraint allows operators on values of the parameter, which keep their built-in meaning, and restricts the types that can instantiate it. There are no user traits and no implementations: a type satisfies a constraint by what it is.

**Editions.** The constructs of this chapter exist only in packages of edition `0.2` (`spec/13 §7`).
- In edition `0.1`, type parameters stay unconstrained, equality on them keeps the inferred bound of `spec/10 §12` rule 9, and the prelude keeps its v0.1 signatures (`spec/14 §1`).
- A single file has the default settings, and so edition `0.1`.
- The Gate 1 benchmark stays on edition `0.1`.

## 1. The constraints

| Constraint | Allows on values of `T` | Satisfied by |
| --- | --- | --- |
| `Eq` | `==`, `!=`, and interpolation (ADR 0068) | every type whose values hold no functions |
| `Ord` | what `Eq` allows, and `<`, `<=`, `>`, `>=` | `Int`, `I8` to `U64`, `Text`, and sum kinds whose variants have no payload |
| `Integer` | what `Ord` allows, and `x.to_int()` | `Int`, `I8` to `U64` |

1. **Order.** `Ord` includes `Eq`, and `Integer` includes `Ord`: a type that satisfies a constraint satisfies the ones it includes. The order is fixed. A program declares no constraint of its own.
2. **Type parameters.** A type parameter written with a constraint satisfies that constraint and the ones it includes. An unconstrained type parameter satisfies none.
3. **Kinds.** A record or a sum satisfies `Eq` when its values hold no functions, whatever its type arguments are. It satisfies `Ord` only when it is a sum whose variants have no payload. Lists, options and results satisfy `Eq` when their element types do, and never `Ord`. (v0.2, ADR 0069; not implemented in v0.2.0; planned (#236).) A `Map[K, V]` satisfies `Eq` when `V` does, and never `Ord` (`spec/21 §1`).
4. **Labels** do not take part in satisfaction: `secret Int` satisfies `Integer`.
5. **Operators keep their meaning.** `==` is structural equality and `<` the built-in order of `spec/03 §1`, on every type that satisfies the constraint. Nothing redefines them (charter §6).
6. **No arithmetic on `T`.** `+`, `-`, `*`, `/` and `%` stay undefined on a type parameter, constrained or not, so the `overflow` obligations of `spec/18 §5` stay on concrete types. `x.to_int()` on a `T: Integer` gives an `Int`, on which arithmetic is defined.

## 2. Syntax

These productions change `spec/02 §2` and `§3`:

```ebnf
signature       = lower_ident [ constrained_params ] "(" [ params ] ")" "->" type ;
constrained_params = "[" constrained_param { "," constrained_param } "]" ;
constrained_param  = upper_ident [ ":" upper_ident ] ;
```

```una
pub sort[T: Ord](xs: List[T]) -> List[T]

pub largest[T: Ord](xs: List[T], floor: T) -> T = xs.fold(initial: floor, step: fn(best, x) -> max(best, b: x))

pub total[T: Integer](xs: List[T]) -> Int = xs.map(each: fn(x) -> x.to_int()).sum()
```

1. A constraint is written on the type parameters of functions only. The type parameters of kinds keep `type_params` of `spec/02 §2`, and `Kind[T: Eq]` is a syntax error (U0101).
2. Each type parameter takes at most one constraint.
3. `Eq`, `Ord` and `Integer` are names of the prelude in the upper namespace. A constraint name that is none of them is U2027. A user kind, variant or capability named `Eq`, `Ord` or `Integer` is U1002, as for `I8` (`spec/18 §1`).
4. `trait` and `impl` stay reserved, with the message of §9.

## 3. Static semantics

1. **Operators on `T`.** An operator of §1 on values of a type parameter is allowed when the parameter's constraint allows it, directly or through the constraints it includes. Otherwise it is U2026 at the operator, with a repair that writes the constraint that allows it on the parameter. This holds for `==` and `!=` too: equality on an unconstrained type parameter is U2026.
2. **`to_int()`.** On a value of a type parameter `T: Integer`, `x.to_int()` is a built-in method with type `Int`. On any other type parameter it is U2026.
3. **Instantiation.** At a call, and wherever a function is used as a value, each type argument MUST satisfy the constraint of its parameter (U2025 at the call, naming the type and the constraint). Refined and second-class types stay excluded as type arguments (U2002, U3016). A type argument that is a type parameter of the caller satisfies the constraints of its own declaration (§1 rule 2).
4. **Inference for private functions.** A private function whose type parameter has no written constraint gets the least constraint that its body needs: the one that allows every operator of §1 its body applies to values of the parameter, joined with those of the callees its type parameter instantiates. Public functions write their constraints (U2026 at the first operator, or U2025 at the call that needs more). This follows ADR 0007 for `uses` and ADR 0062 for `uses of`.
5. **Equality and functions.** The bound of `spec/10 §12` rule 9 is, in edition `0.2`, the constraint `Eq`: a type whose values hold functions does not satisfy `Eq` (§1), so instantiating a parameter `T: Eq` with it is U2025.

## 4. Information flow

1. **Operators.** A comparison, an equality or `to_int()` on values of `T` is labeled with the join of every label of its operands, the symbolic label `ℓ_T` included (`spec/07 §7` rules 3 and 8). Branching on it raises the program counter, symbolically when the label contains `ℓ_T`.
2. **The prelude.** The functions of §6 read values, so their results are labeled with the join of every label of their arguments, element types included (ADR 0051). For `sort`, every position of the result, and the result list, carries the join of the labels of the list and of all its elements, since where an element ends up depends on the others.

## 5. Verification

1. **Generic bodies.** In the body of a generic function, `<`, `<=`, `>` and `>=` on a `T: Ord` are uninterpreted relations over the sort of `T`, `u!Param_<name>` (`spec/12 §4`), with no axioms. `x.to_int()` on a `T: Integer` is an uninterpreted function from that sort to `Int`. Equality stays SMT equality.
2. **Calls.** A callee's contract is instantiated with the type arguments, as today (`spec/12 §3`). When the type argument is `Int` or a fixed-width type, its comparisons are those of the integers, and `to_int()` is the identity, with the bounds of the type. For `Text` they are the uninterpreted order of `Text`, and for a sum without payloads the order of its tags.
3. **No laws.** No axiom states that an order is total, reflexive or transitive. Laws of constraints wait for laws with quantifiers (`plan/backlog.md`, v0.3).

## 6. The prelude of edition `0.2`

These signatures replace those of `spec/14 §1` in edition `0.2`:

| Signature | Contract |
| --- | --- |
| `min[T: Ord](a: T, b: T) -> T` | `ensures result <= a and result <= b and (result == a or result == b)` |
| `max[T: Ord](a: T, b: T) -> T` | `ensures result >= a and result >= b and (result == a or result == b)` |
| `contains[T: Eq](xs: List[T], item: T) -> Bool` | none |
| `sum[T: Integer](xs: List[T]) -> Int` | none |
| `sort[T: Ord](xs: List[T]) -> List[T]` | `ensures result.len == xs.len` |

1. `sum` adds the values of the elements as `Int`, so it never overflows.
2. `sort` returns the elements in ascending order, and keeps the order of the list for equal elements (stable).
3. `abs` stays `abs(x: Int) -> Int`, since `abs` of a fixed-width value can overflow.
4. Calls with `Int` arguments keep their meaning and contracts.

## 7. Dynamic semantics

1. **The interpreter** evaluates `==`, `<` and `to_int()` on values of `T` by the form of the value, as on concrete types (`spec/11`).
2. **The native backend** (`spec/16 §4`) compiles each function once. A function with constrained type parameters takes, for each of them, a hidden **type descriptor** after its parameters: a word that says how to compare and convert values of the type that instantiates it (a tagged `Int`, a fixed-width type with its width and signedness, `Text`, a sum without payloads compared by tag, or structural equality). A call with a known type passes a constant descriptor. A call that instantiates a parameter with a type parameter of the caller passes the caller's descriptor on. Unconstrained parameters take no descriptor.

## 8. The report and the contract

1. The header of `una contract` shows the constraints as written, and inferred ones as written by the canonical form, with the comment `-- inferred` (`spec/13 §5`).
2. `una.contract/1` gains the key `constraints`, after `header`: one entry per constrained type parameter, `{"param": "T", "constraint": "Ord", "inferred": false}`, in parameter order, and `[]` when there are none.

## 9. Reserved words

In edition `0.2`, the table of `spec/01 §5`, as changed by `spec/17 §9` and `spec/19 §9`, changes as follows:

| Keywords | Message |
| --- | --- |
| `trait`, `impl` | reserved for a later version (user traits) |

## 10. Diagnostic codes

These codes join the registry of `spec/13 §4` with their implementation:

| Code | Domain | Meaning |
| --- | --- | --- |
| U2025 | types | A type argument does not satisfy the constraint of its parameter |
| U2026 | types | An operation on a type parameter that its constraint does not allow |
| U2027 | types | An unknown constraint name |
