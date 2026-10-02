# 0060. Recursive kinds

- Status: Accepted
- Date: 2026-10-02
- Deciders: Leonardo (accepted on 2026-10-02), on a proposal by Claude (Una-1)
- Resolves: issue #38 (backlog row "Recursive kinds with a base variant", v0.1 review R-045)
- Amends: `spec/03 §2` rule 5, `spec/10 §12` rule 8, `spec/11 §7`, `spec/12 §4`

## Context

`spec/03 §2` rule 5 allows a kind to contain itself only through `List` or `Option` fields (U2002 otherwise), so every value is finite. A tree or a linked list can be written today, as `Node(value: Int, next: Option[Node])` or `Tree(children: List[Tree])`. An expression type in its natural form cannot:

```una
kind Expr = Lit(value: Int) | Add(left: Expr, right: Expr) | Neg(operand: Expr)
```

The workarounds wrap every recursive field in `Option` or `List`. They lose the shape in the type, add a `None` case to every match, and give the verifier nothing in exchange.

What already holds:

- **Values are finite.** They are immutable and built bottom up. No construct of v0.2 makes a cycle (ADR 0055: no mutable aliasing). A value of a recursive kind is a finite tree.
- **The verifier treats kinds as uninterpreted sorts** with a tag and one function per field (`spec/12 §4`). Recursion needs no change there. The encoding has no datatype theory, and verification stays in QF_UFLIA.
- **Labels and variance** of recursive and mutually recursive kinds take the least solution (ADR 0046 rule 1). The checker computes the deep labels of kinds the same way.
- **Recursion of functions** is allowed, and termination is not checked (`spec/04 §2` rule 3). Predicates may not recurse (U2015, `spec/03 §4`).
- **The native backend** keeps heap values reference counted (ADR 0055).

One new fact: values can be deeper than the call depth. With `for` loops (#81), a program builds a list of a million nodes without a single call. A runtime that releases, compares or renders such a value by recursion overflows its native stack. The interpreter of #81 uses worklists, and `tests/conformance/runtime/for_loop_deep_values` covers it.

Leonardo took R1 to R5 below on 2026-10-02, each as recommended. The coordinator answered the code question: U2002, with no new code.

## Decision

1. **Inhabited kinds (R1).** In edition `0.2`, a record or sum kind may contain itself, or a kind that contains it, through any field, when it has a finite value. Finite values are computed as a least fixpoint over the kinds of a package:
   - The grounded types are:
     - `Int`, `Bool`, `Text`, `Unit`, `Hashed`, the fixed-width integers, function types and type parameters;
     - `List[_]` and `Option[_]`, since `[]` and `None` exist;
     - `Result[T, E]`, when `T` or `E` is grounded.
   - A variant, or a record, is grounded when every field type is grounded. A sum kind is grounded when one of its variants is.
   - An instance `K[A1, ..., An]` of a generic kind is grounded when `K` is, with each parameter replaced by its argument. A type parameter of the kind being checked counts as grounded. So `Wrap(inner: Box[E])` with `kind Box[T]` of one field `value: T` is grounded only when `E` is.
   - A record or sum kind that is not grounded is U2002, once per kind, at its name: "kind `Stream` has no finite value: every variant contains a kind without one; add a variant without it, as `End`". For a record it names the field: "kind `Ring` has no finite value: its field `next` needs a kind without one; make it an `Option` or a `List`".
2. **Regular recursion (R2).** In a set of mutually recursive kinds, an occurrence of a kind of the set, inside the declaration of a kind of the set, has only type parameters of that declaration as its arguments. `kind Nest[T] = Flat | Deep(inner: Nest[List[T]])` is U2002 at the field: "`Nest` occurs in its own recursion with the argument `List[T]`; inside a recursive kind, the kinds of its recursion take only type parameters". Each instance of such a kind then draws its arguments from those of the first instance, so the instances are finite: for the SMT sorts, for the monomorphization of the native backend and for the checker's traversals of types.
3. **Verification by contracts (R3).** Recursive values are verified by modular contracts only. Predicates stay non-recursive (U2015). A property of a whole value is stated as the `ensures` of a recursive function, and the verifier checks it by its contract at each recursive call: induction for partial correctness. The encoding adds no axiom of acyclicity or size, so `e.left != e` is not provable. Structurally recursive reflect predicates are a follow-up, issue #146.
4. **Termination unchanged (R4).** Termination of recursive functions over recursive kinds is not checked, as for all recursion. A non-terminating recursion is stopped by the call depth (U6003).
5. **Unbounded depth of values (R5).** A value has no depth limit. Only calls count toward the call depth. Release, equality, rendering and the trace never recurse on the native stack over the structure of a value, in the interpreter and in `una-rt`.
6. **Edition.** Rules 1 and 2 apply in edition `0.2`. Edition `0.1` keeps `spec/03 §2` rule 5 and its U2002 message, as ADR 0056 does for the memory model.

Rules that follow, unchanged in substance:

- **Exhaustiveness and patterns.** A `when` over a recursive kind has one arm per variant, and the `exhaustive` obligation is the same.
- **Fields and places.** Field access, `is` and constructors are unchanged. A field of a variant payload is still not a place (`spec/17 §2`), so a recursive sum is updated by rebuilding it, and a recursive record by field assignment.
- **Labels and variance** follow ADR 0046.
- **The SMT encoding** (`spec/12 §4`). A recursive kind is one uninterpreted sort, and its recursive fields are functions into the same sort.
- **Native representation.** A value of a record or sum kind is a reference-counted heap cell that holds its fields, a recursive field as a pointer. Release is iterative.

## Consequences

- Expression trees, linked structures and abstract syntax are written in their natural form, which agents generate most often.
- The verifier proves properties of recursive values through function contracts only. A precise property of a whole structure, such as its size or that a tree is sorted, waits for #146.
- `una-rt` takes on a requirement before the native backend compiles records and variants: iterative release, equality and rendering.
- The checker's groundedness fixpoint and its regularity check replace the cycle check of U2002 in edition `0.2`.

## Alternatives considered

- **A syntactic base variant** (R1 B): at least one variant that does not mention the kind. Not chosen: it rejects some inhabited mutual recursion, and it accepts a variant whose fields mention another uninhabited kind.
- **Any recursion** (R1 C). Not chosen: an uninhabited kind is not unsound, but it is almost always a mistake, such as a missing `Nil`, and an agent would get no diagnostic.
- **Polymorphic recursion** (R2 B). Not chosen: every pass over types would need a fixpoint or a bound on the depth of instantiation, and nothing in the roadmap needs it.
- **Structurally recursive reflect predicates now** (R3 B). Deferred to #146: the queries stay ground with a fixed unfolding depth, but the predicates need a termination check (an axiom like `p(x) = not p(x)` is inconsistent), and results at the depth limit are `unknown`.
- **An opt-in structural termination check for functions** (R4 B). Not needed for soundness in partial correctness. It may come with #146.
- **A depth limit on values** (R5 B), with an abort like U6003. Not chosen: it rejects ordinary programs, such as a long list built in a loop.
- **SMT datatypes** (`declare-datatypes`), which bring acyclicity and structural induction facts. Not chosen: they leave QF_UFLIA (`spec/12 §2`), and Z3's datatype reasoning brings performance and `unknown` behaviour that the rest of the encoding avoids.
- **Box-like indirection in the syntax** (`left: Box[Expr]`). Not chosen: with immutable reference-counted values, indirection is the backend's choice, and the source should not spell it.

## Amendment (2026-10-02)

R2 stands. Its justification is the finiteness of the SMT sorts and of the checker's traversals of types. The native backend is uniform and compiles each function once, every value a word (ADR 0055, ADR 0062 rule 11), so "the monomorphization of the native backend" in R2 is not a reason: there is none. Found while drafting ADR 0066, whose type descriptors keep that representation; the coordinator asked for this amendment.
