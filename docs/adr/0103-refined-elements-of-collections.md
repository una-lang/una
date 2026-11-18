# 0103. Refined elements of collections

- Status: Accepted
- Date: 2026-10-14
- Deciders: Una project (accepted on 2026-10-14)
- Resolves: issue #39 (v0.1 review R-012, deferred by ADR 0040)
- Target: edition `0.3`. Editions `0.1` and `0.2` keep ADR 0040: a refined type argument stays U2002 there. The Gate 1 pin (v0.1.9, edition `0.1`) does not move.
- Loosens: edition `0.3` accepts programs that edition `0.2` rejects (rule 9). This needs an amendment of the preview rule of `spec/13 §7` (ADR 0056, amendments of 2026-10-13), which this record proposes with it.
- Amends, in edition `0.3`: ADR 0040 rules 1 and 2; `spec/03 §3`, `§6`, `§7`; `spec/12 §3`, `§4`; `spec/13 §7`; `spec/14 §1`; `spec/17 §2` rule 4; `spec/21 §2`, `§4`

## Context

ADR 0040 rejects a refined type as a type argument: `List[Pos]`, `Option[Seats]`, `Map[Text, Pos]` and `Box[Pos]` are U2002, and a type argument inferred from a refined value is its base. That closed R-012, where such signatures were shown by `una contract` but never checked. It also took away a common way to state a fact once: "every price in this list is positive" has to be checked again wherever an element is used, with a guard or `checked`.

ADR 0040 recorded the alternative (E5 B): obligations where a collection is built, and facts where an element is read. It was set aside because it seemed to need quantifiers, "for every element", which `spec/12 §2` keeps out of every query. This record shows that it does not:

- **The refinement of a kind is closed.** A kind's `where` clause speaks only of `it`, of literals and of predicates (`spec/03 §3`); a range's bounds are literals. So "every element of `xs` satisfies `φ`" never depends on another variable of the program. Two element types compare by one quantifier-free query over a fresh binder: `φ_A(e) ∧ ¬φ_B(e)` is unsatisfiable exactly when `∀e. φ_A(e) ⇒ φ_B(e)` holds. ADR 0037 already compares the refinements of function types this way.
- **Elements reach code only through binders and projections.** Una has no indexing operator and no element projection in the encoding. An element is read through a prelude function's result (`first`, `at`, `first_where`, `or_else`, a map's `get`), a pattern variable (`Some(x)`, a payload field), a loop variable (`for x in xs`) or a lambda's parameter (`map`, `filter`, `fold`). Each of these is a term the query already names, so the element's refinement is a ground fact about that term, as the refinement of a record field is today (`spec/12 §3`).
- **Values are immutable.** A list never changes after it is built. `set`, `append` and a map's `set` give a new value (`spec/17`, `spec/21 §5`). A fact proven when a value is built holds for as long as the value exists.

## Decision

1. **Refined type arguments are well-formed in edition `0.3`.** A refined kind, a type with range sugar, or a type that contains one MAY be a type argument: `List[Pos]`, `Option[Int in 0..5]`, `Result[Seats, E]`, `Map[Text, Pos]`, `Map[Pos, Text]`, `Box[Pos]`, and `List[List[Pos]]`, in a signature, a kind's fields, a function type, an annotation and a written type argument. ADR 0040 rule 1 stays in force for editions `0.1` and `0.2`. A refined type satisfies a constraint of `spec/20 §1` when its base does: `Pos` satisfies `Integer`, so `Map[Pos, V]` is well-formed.

2. **A refined position is a fact about every value at it.** Each position of a type (its top, a list's element, each type argument of `Option`, `Result`, `Map` or a kind, and the parameters and result of a function type: the positions of `spec/13 §5`, ADR 0067) may carry a refinement. The invariant that every rule below keeps is: **every value at a refined position satisfies its refinement**. Rules 3 to 5 keep it where values are built. Rule 6 uses it where they are read.

3. **Obligations at construction.** A value flows into a refined position only at a flow site of `spec/12 §3` (an argument, a constructor field, an annotated `let`, an assignment, an `edit` call, a join, a return or a lambda's result) or at a list literal's element. Each such flow is a `refinement` obligation:
   - **A list literal** with element type `T`: one obligation per element, `φ_T` of that element, under the path context of the literal. `[]` gives none.
   - **A constructor of a generic kind,** `Some(x)` or `Ok(x)` included: the field whose type is the parameter, instantiated with the refined type, is a constructor field, so its obligation is the one `spec/12 §3` already generates for a refined field.
   - **A prelude function** whose parameter has the refined type after instantiation: `append`'s `item`, `set`'s `item`, `or_else`'s `default`, and a map's `with` and `set`, at `key` and `value`. Each is an argument flow.
   - **A function's result** that becomes an element: the result of `map`'s `each` flows into the element type of the result list, and `fold`'s `step` into the accumulator type. These are lambda results, so a lambda's body gets one obligation over every normal return path (ADR 0037 rule 7), with the refinement of its parameters as hypotheses.
   - **The body of a `together for`** (`spec/19 §2` rule 2), which makes one element per task. When the element type is refined, given by an annotated `let`, an assignment's place or another expected type, the body's value flows into it. That is one obligation over the body's normal return paths, as for a lambda's result, with the loop variable's refinement as a hypothesis. Without an expected type, the element type follows rule 5, the body's value being the one value that determines it.
   - **A whole collection** flowing into a type with a refined position (rule 4).

   No obligation arises where the elements come from a value already at that type. `filter`, `sort`, `first_where`, a map's `without`, `remove`, `keys`, `values` and `entries`, `first` and `at` only return elements they received.

4. **Subtyping of collections, by variance.** A flow of a value of type `C[σ̄]` into `C[τ̄]` compares each refined position as ADR 0046 compares labels:
   - **Covariant positions,** that is the element of `List`, both arguments of `Map`, the arguments of `Option` and `Result`, the result of a function type, and a covariant parameter of a user kind: one obligation `φ_σ(e) ⇒ φ_τ(e)` over a fresh binder `e`, when `τ` carries a refinement that `σ` does not carry as the same refined kind or the same range.
   - **Contravariant positions,** the parameters of a function type: the implication the other way (ADR 0037 rule 1).
   - **Invariant positions,** a parameter of a user kind that appears in a function type (ADR 0046): both implications.

   The obligation is closed. It holds no hypothesis of the path context, since a kind's refinement speaks of nothing else (Context). So the same pair of refinements gives the same query wherever it is compared, and the proof cache answers it after the first time.
   - `List[Pos]` flows into `List[Int]` with no obligation. `List[Int]` flows into `List[Pos]` with the obligation `true ⇒ e > 0`, which is refuted: a list of integers is not known to hold only positive ones. A list is converted by building it at the refined type (Consequences).
   - **`edit` arguments are invariant in effect.** An `edit` argument flows in at the call and its final value flows back into the caller's place (`spec/17 §5` rule 2). So passing a `List[Pos]` place to an `edit xs: List[Int]` parameter gives the refuted obligation of the flow back. A callee that may write any integer into the list cannot be given a list of positive ones. No rule is added: the flow back is already a flow site.

5. **Instantiation.** A type argument that is written, or fixed by the expected type, is that type, refinement included. One fixed by `edit` arguments' places is their type when every such place agrees on the refinement, as the same refined kind or range. Otherwise it is the base, and each refined place then gets the refuted flow back of rule 4, which names it. The expected type fixes it where it determines it, as `List[Pos]` does for `append`'s result. A type argument inferred from values keeps a refinement only when both of these hold:
   - every value that determines it has that refinement at that position, as the same refined kind or the same range. A lambda's result counts as such a value, typed with the type arguments at their base;
   - the type parameter occurs only at covariant positions of the callee's result type. In `make[T](x: T) -> fn(T) -> Bool`, `T` is at a contravariant position, so `make(p)` gives `fn(Int) -> Bool`.

   Otherwise the type argument is the base, as in ADR 0040 rule 2. The base of a type is the type with every refinement removed, at every position.
   - So `Some(p)` and `[p, q]` with `p, q: Pos` are `Option[Pos]` and `List[Pos]`; `[p, 1]` is `List[Int]`. `contains(prices, item: n)` with `n: Int` instantiates `T` with `Int`, and asks nothing of `n`.
   - In a generic kind whose parameter is invariant (ADR 0046), an inferred type argument is the base. Only a written or expected type gives it a refinement, so an inferred refinement never meets an invariant flow.
   - An editable binding without an annotation has the type of its initial value with every refinement removed, at every position (`spec/17 §2` rule 4, extended from the top). An annotation, as `let edit out: List[Pos] = []`, makes each refinement an invariant of the binding, and so of a loop that updates it (`spec/17 §5` rule 3).
   - **Map lookups take the key's base.** `has`, `get`, `remove` and `without` take `key` at the base of `K`. A lookup stores nothing, so a key outside the refinement only finds no entry. `with` and `set` take `key` at `K` (rule 3).

6. **Facts on access.** The hypotheses of `spec/12 §3` gain the refinements of nested positions:
   - **Binders.** A binder whose type is refined at its top gets that refinement, as today. So a pattern variable (`when o is Some(x)`), the value of `if none fail` and `if error fail`, a loop variable (`for x in xs`), a lambda's parameter and the result of a call each get the refinement of the position they come from, once rule 5 has given them that type. A join (`if`, `when`, `if none use e`, `if error use e`) keeps a refinement at a nested position when every branch has it there, as the same refined kind or range, or when the expected type gives it, and then every branch flows into it. At the top, a join's type is as today. So `o if none use 0` with `o: Option[Pos]` is an `Int`.
   - **Projections.** The ground facts of `spec/12 §3` already apply the refinement of a record field to the term of that field. They extend to the instantiated type: the refinement of a position applied to each term in the query that projects it, under the tag of its variant. These positions are the `value` of an `Option` or `Result`, the field of a generic kind's value at a refined argument, and the `value` of a map's `get` under its tag `Some`.
   - **Prelude results.** `first(xs)` and `at(xs, i)` on a `List[Pos]` are `Option[Pos]`, `first_where` too, `or_else` is `Pos` when both its arguments are, and a map's `get` is `Option[V]` with `V` refined. With rule 5, these follow from the generic signatures of `spec/14 §1` and `spec/21 §2` without a special case.
   - In a generic body, nothing changes: the type parameter carries no refinement, and its sort has no facts.

7. **Why the facts are sound.** A fact on access is assumed, never proven, so it relies on the invariant of rule 2. A value at a refined position comes from one of these:
   - a flow site, where rules 3 and 4 put an obligation, which tier 1 proves;
   - a prelude function that returns values it received at the same position, as `filter`, `sort` and `first` do (the trusted signatures of `spec/14 §1` and `spec/21 §2` say which, Decision 3);
   - a user generic function, instantiated with a refined type argument. Its body is verified once for an abstract `T` (`spec/20 §5`). It has no way to make a value of `T` but from values of `T` it received: Una has no literal, conversion, default or decoder of a type parameter, no arithmetic on one (`spec/20 §1` rule 6), `to_int()` only leaves `T`, and the prelude functions it calls are parametric too. So every `T` it returns came from a `T` its caller proved, by rule 3;
   - a handler operation's result, which flows into the operation's declared type (`spec/12 §3`), or a host operation, whose declared types hold no refined argument (`spec/14 §3`).

   Runtime values are unchanged: refinements are erased in the interpreter and the native backend, and no check is added at run time.

8. **The verifier's encoding** (`spec/12 §4`).
   - **Sorts erase refinements.** `List[Pos]` and `List[Int]` are the same sort, `i!List_Int`, as `Pos` and `Int` are both `Int` today. A value keeps its term when it flows from one to the other, so a fact proven of it stays attached through every flow, and no conversion function enters the query.
   - **The fresh binder** of a collection obligation (rule 4) is declared like ADR 0037's binders: a `u!v!` constant named by the position's path of `spec/13 §5` (`element`, `arg 2`), never bound by a quantifier. The query stays in `QF_UFLIA`.
   - **Goal and message.** The goal prints as `<position> of <value>: <refinement>`, as ADR 0037 prints `parameter x of f: x > 0`. For example, `element of prices: element > 0`, `arg 2 of stock: arg_2 >= 0`. The message is "refinement `<goal>` is refuted", and the counterexample binds the binder by its name (`spec/12 §7`).
   - **Repairs** (`spec/13 §3`). At a list element or an argument, the repairs of a `refinement` obligation are as today. At a collection flow, the one repair is the refined type at the source, where the collection is built.

9. **Editions, and what this loosens.** Every rule above applies only in edition `0.3`.
   - **Edition `0.3` accepts programs that edition `0.2` rejects.** It accepts refined type arguments, which `0.2` rejects with U2002, and that is a construct edition `0.3` adds. It also accepts some programs that `0.2` can express: `when Some(p) is Some(x)` followed by a call that needs `x > 0` is refuted in `0.2`, where the option is `Option[Int]`, and proven in `0.3`. Erasing the refinements does not give a program `0.2` accepts, so this is not one of the narrowing constructs that the preview rule of `spec/13 §7` admits. This record therefore amends that rule: "A later `0.3.x` release MAY also accept refined type arguments, and the facts on access they give (ADR 0103), in a package of edition `0.3`."
   - **What it does not loosen.** No obligation of edition `0.2` is dropped or weakened. Every program that edition `0.3` newly accepts is accepted on facts that rule 7 shows sound.
   - **A program of edition `0.2`, checked as `0.3`,** may see a type with more refinements than before (rules 5 and 6). It gains no obligation from that:
     - every such refinement is inferred, at a nested position, and covariant (rule 5);
     - it flows only into the base, which a program of edition `0.2` writes, or into a position that rule 5 inferred from the same refinement. Neither gives an obligation (rule 4);
     - an editable binding without an annotation drops it (rule 5).

     Only the added facts change its queries. A query may then take another number of solver steps, so under the fixed `step_budget` an obligation proven in `0.2` can become unknown (U5002) in `0.3`. Such a program can gain an error under the step budget, and can lose one where the added facts prove an obligation that edition `0.2` refuted (the loosening above). A check over the conformance cases and examples of edition `0.2`, run as `0.3`, pins that their outcomes stay the same, or lose only the U5001 and U5002 of obligations that the added facts prove.

10. **Diagnostic codes.** None is added.
    - U2002 keeps its meaning for a refined type argument in editions `0.1` and `0.2`. Its explanation (`una explain U2002`) gains a line saying that edition `0.3` accepts them (ADR 0103).
    - The obligations of rules 3 and 4 are of the kind `refinement`, so they are U5001 when refuted and U5002 when unknown, as today.
    - A `where` clause of an element type outside the refinement language is U2014 where the kind is declared, as today.
    - U2025 still requires a map's key type to satisfy `Ord`, by its base (rule 1).

## Labels

Labels and refinements are independent at every position (`spec/03 §5`). `List[secret[PII] Pos]` has a `secret[PII]` element whose base is refined. Every rule of `spec/07` reads only labels and stays unchanged:
- flows compare the labels of each position by ADR 0046, and the refinements by rule 4, separately;
- the symbolic labels of a generic function (`spec/07 §7` rule 8) are replaced at a call by the labels of the type argument, whatever its refinement;
- the verifier erases labels (`spec/12 §4`). So an obligation, or a fact on access, is a judgment at compile time over every run, and it makes no value depend on another at run time. A fact about a `secret` element that discharges an obligation on a `public` value creates no flow: neither value is read to prove it, and nothing of it remains in the executable.

The re-check of the formal track (F1) models information flow and erases refinements (`docs/plan/formal-track-status.md`). So this record changes no theorem of F1, below.

## Generics

- **Constraints** (`spec/20`, ADR 0066). A refined type satisfies a constraint by its base (rule 1). Inside a generic body nothing changes: `T` has no refinement, and its order and `to_int()` stay uninterpreted.
- **Instantiation** is rule 5. A generic function's contract is instantiated with the refined type argument as with any other, so its `ensures` about `T` values hold together with the refinement facts of rule 6.
- **Parametricity** carries the soundness of rule 6 through user generic functions (rule 7). It is a property of the language as it stands. A future construct that can make a value of a type parameter other than by passing one on MUST either be unavailable at a refined `T` or check the refinement where it makes the value. Its ADR states which. Such constructs include a decoder `decode[T]`, a default value of `T`, a literal of an `Integer` `T`, and a constraint that allows `+` or `-` on `T`: with the last, `x - x` at `T = Pos` would be a `Pos` that is `0`.

## The formal track (F1)

- **No theorem changes.** F1 proves information flow, authority and trace safety over the Core IR that `una core --json` exports. Refinements are outside that model by design: a predicate is counted "outside the flow model", and labels are what its theorems speak of.
- **The importer.** The Core IR already gives every position a refinement (`spec/09 §2`, `τ ::= {ν: b | φ}^ℓ`), so the export keeps its schema, `una.core/1`. A refinement now appears at nested positions where it never did. The importer of the re-check MUST read it and ignore it, as it ignores top-level refinements. Until it does, a declaration that holds a refined type argument is "not re-checked", never accepted. The corpus delta of the implementation PR shows which.
- **Refinement soundness** is not a theorem of F1 today, at the top level or nested. Rule 7 is the argument for the nested case. A theorem of refinement typing, "every value at a refined position satisfies its refinement", is an open obligation for a later milestone of the formal track. This record adds it to the formal-track status page as "accepted, not proved", as ADR 0092 did for trace safety.

## Complexity

Let `n` be the size of the program, `d` the depth of a type, and `t` the number of terms of a query.
- **Type checking.** A flow compares two types position by position: O(d). Instantiation (rule 5) is one pass over the values that determine each type argument, so it is linear in the call's arguments.
- **Obligations.** A list literal of `k` elements gives `k` obligations, as `k` arguments of a call do. A collection flow gives at most one obligation per refined position of the type, two at an invariant one: O(d), and none when the refinements are the same kind. Each is a closed query of size O(|φ_A| + |φ_B|) over one binder. The same pair of refinements gives the same query text, so the proof cache solves it once per package.
- **Facts.** Each projection term of the query at a refined position adds one ground instance of the refinement: O(t · |φ|) per query, as the record-field facts of `spec/12 §3` already cost.
- **Overall**, the number of obligations and the size of each query grow linearly in `n` for types of bounded depth, and no query holds a quantifier. Scaling tests pin it: a list literal of 10, 100 and 1,000 elements; a chain of 1,000 `append`s into a `List[Pos]` binding in a loop; and types nested 1 to 8 deep. The per-element cost of each stays within a fixed ratio.

## Tests

Conformance cases, each rejected and accepted forms, in edition `0.3`, and the same programs in editions `0.1` and `0.2`, where U2002 stays:
- a list literal with one element outside the refinement: U5001 at that element;
- `append`, `set`, `or_else` and a map's `with` and `set` with an argument outside the refinement;
- `map` whose lambda returns a value outside the element refinement, and one whose lambda's parameter fact proves it;
- a `together for` under an annotated `List[Pos]`, whose body is sometimes outside the refinement: U5001;
- two `edit` arguments whose places fix one type parameter with different refinements: the refined place's flow back is refuted;
- the conformance cases and examples of edition `0.2`, checked as `0.3`, keep their outcomes, or lose only the U5001 and U5002 of obligations that the added facts prove;
- `List[Int]` into `List[Pos]` (refuted, with the binder `element` in the counterexample), `List[Pos]` into `List[Int]` (no obligation), and the same through `Option`, `Result`, `Map` and a covariant user kind;
- an invariant user kind with two different refinements (refuted both ways);
- an `edit` argument of `List[Pos]` to an `edit xs: List[Int]` parameter: refuted by the flow back;
- facts on access through `first`, `at`, `first_where`, `or_else`, `when ... is Some(x)`, `for x in xs`, a lambda's parameter, `fold`'s accumulator and a map's `get`, each proving an obligation that edition `0.2` refutes;
- a loop that builds a `let edit out: List[Pos]` by `append` under a guard, and keeps the refinement as its invariant;
- a map lookup with a key outside a refined key type: no obligation;
- a generic user function instantiated with `Pos`: an argument outside it is refuted, and its result gives the fact;
- labels: a `List[secret Pos]` keeps every label check of `spec/07` unchanged (one rejected flow, one accepted);
- inference: `[p, 1]` is `List[Int]`; an un-annotated `let edit xs = [p]` accepts a later `append` of a negative value;
- the scaling tests of Complexity.

## Consequences

- `una contract` and the report show refined type arguments, and each is checked where values are built, which closes what R-012 found missing without the rejection of ADR 0040.
- **How a list becomes refined.** A list of a base type is not converted by a flow (rule 4). It is built at the refined type: a literal, a `map` whose function returns the refined type, or a loop that appends to an annotated binding under a guard. A `filter` whose predicate implies the refinement does not narrow the type (Alternatives).
- Agents and readers state an element fact once, in the type, and the verifier carries it to every access. The guard or `checked` at each use is no longer needed.
- Runtime behavior, the interpreter and the native backend are unchanged.
- Edition `0.3` gains a looser rule than its preview rule allowed (rule 9). That is the reason this record needs acceptance by the project before it is implemented.

## Alternatives considered

- **Quantified facts in tier 2,** as ADR 0040 expected: `∀i. 0 ≤ i < len(xs) ⇒ φ(at(xs, i))`. Not chosen: quantifiers leave the decidable fragment of `spec/12 §2`, make `unknown` common, and are not needed, since elements are only reached through named terms (Context).
- **An element function in the encoding** (`i!at!List`), with the refinement of every element term. Not chosen: Una has no indexing operator, so there is no term to apply it to, and it would add a second way to name an element beside the binders that already exist.
- **Narrowing `filter`,** a `filter` whose result has the refinement its predicate implies, `xs.filter(fn(x) -> x > 0)` as a `List[Pos]`. Deferred: it needs a signature whose result type depends on its argument, which the prelude does not have. A loop with a guard (Consequences) expresses it today.
- **Keep the refinement on inferred type arguments always.** Not chosen: `fold(xs, initial: p, step: fn(acc, x) -> acc - x)` would instantiate the accumulator with `Pos` from `initial` and refute the lambda, an obligation that a program accepted by edition `0.2` never had. Rule 5 keeps a refinement only when every value that determines it agrees.
- **Lookups at `K`,** with an obligation on the key of `get`. Not chosen: a lookup stores nothing, and the obligation would reject a correct program that looks up a key the map cannot hold.
- **Allow it in edition `0.2`.** Not chosen: edition `0.2` keeps its promise that a later release does not change what it accepts (`spec/13 §7`).
