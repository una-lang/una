# 0056. Memory model: mutable value semantics

- Status: Accepted
- Date: 2026-10-01
- Deciders: Leonardo (accepted on 2026-10-01), on a proposal by Claude (coordinator)
- Resolves: issue #109 (epic #18; implementation #29, #81, #82, #30)
- Amends: `spec/01 §5`, `spec/02`, `spec/04`, `spec/07 §7`, `spec/09`, `spec/10`, `spec/11`, `spec/12`, `spec/13 §4` and `§7`, `spec/14 §1`, the guide's keyword section

## Context

In v0.1 values are immutable and bindings are never reassigned. Programs have no loops, and contracts cannot read entry values (`spec/04`, `spec/09`). The words `edit`, `take`, `region`, `shared` and `iso` are reserved for v0.2 (`spec/01 §5`).

The main goal of v0.2 is native performance (ADR 0055). Efficient programs need mutation, of accumulators, buffers and records built field by field. But the guarantees of v0.1 rest on immutable values:
- the UAM is the reference semantics;
- verification stays in QF_UFLIA, with no quantifiers;
- information flow is checked by typing.

Leonardo decided the memory model in four rounds on 2026-10-01 (V2-1 to V2-16). This ADR states those decisions and the rules that follow from them.

## Decision

1. **Mutable value semantics (V2-2).**
   - There are no references and no lifetimes in the surface language. Values stay immutable; only bindings declared mutable are rebound.
   - `edit` is an in-out parameter convention. It desugars to returning the new value and rebinding it.
   - `take` consumes its argument.
   - In-place mutation of a unique value is an optimization of the backend, with no syntax (V2-15).
2. **No mutable aliasing in v0.2 (V2-3).** `shared`, `iso` and `weak` are reserved for v0.3 or later. Graphs and cycles use arenas and indices in the standard library.
3. **Transactional failure (V2-5).**
   - A call that fails after changing an `edit` argument leaves that argument unchanged for the caller, exactly as the desugar says.
   - The rollback covers values only. Operations already performed on capabilities, which the trace records, are not undone, as in v0.1 when a function fails after an operation.
4. **Contracts over edited parameters (V2-6).** In `ensures`, an `edit` parameter `x` is its final value, and `old(x)` its value on entry. `old` is legal only in `ensures`. `requires` reads entry values.
5. **Affine `take` (V2-7).** Any use of a variable after it was passed by `take` is a compile error. The semantics stay pure, and the backend may reuse the memory.
6. **`let edit` bindings (V2-8).** `let edit x = e` declares a mutable binding. Each update is a rebind in Core IR, a new SSA `let`.
7. **Field edits (V2-9).**
   - A field of an editable binding can be updated: `r.f = v`, or `edit r.f` as an argument, desugars to rebuilding `r` with that field changed.
   - List elements change only through library functions with an index `requires`.
8. **Lambdas capture by value (V2-10).** A lambda captures a `let edit` binding's value when it is created. Later updates are not seen inside it, and the lambda cannot change the binding.
9. **Refinements hold at every rebind (V2-11).** Each update carries the refinement obligation of a new `let`, so there are no intermediate invalid states.
10. **Exclusivity.** `f(edit a, edit a)`, or `edit a` together with another use of `a` in the same call, is a compile error.
11. **Loops (V2-12).**
    - `for x in xs` has a full body: operations with `@`, `fail`, `if error`, and several statements.
    - It desugars to a compiler-generated recursive function whose parameters and results are the `let edit` bindings the body updates.
    - There is no `while` and no `invariant` clause in v0.2. The refinements of the updated bindings are the loop invariant (V2-11).
    - A failing body fails the whole loop, and the bindings keep their values from before the loop (V2-5).
12. **Call-site markers (V2-13).** `edit` and `take` are written at the call site, including on a method receiver: `add(edit inv, item: x)`, `edit total.record(e)`, `send(take buffer)`. A line without `@`, `edit`, `take` or an assignment changes nothing.
13. **`if` without `else` (V2-14).** It is allowed when the `if` is a statement of type `Unit`, and `else` stays mandatory when the `if` is a value.
14. **No `m.with(k, v)` (V2-15).** `with` rebinds capabilities. A functional update is written `let edit r2 = r`, then `r2.f = v`.
15. **Labels of editable bindings (V2-16).**
    - The declared type of a `let edit` binding, label included, is fixed. A rebind under a raised program counter that does not fit is rejected, never silently raised.
    - A loop over labeled structure runs its body under that label: `fail` there is U4002, and sinks must accept the program counter.

16. **Editions.** The memory model exists only in packages of edition `0.2` (`edition = "0.2"` in `una.toml`).
    - Edition `0.1` keeps the keyword tables of v0.1: `old` is a name, `weak` is not reserved, and `edit`, `take` and the rest of the v0.2 syntax give their v0.1 U0103 messages.
    - A single file has edition `0.1`.
    - The Gate 1 benchmark stays on edition `0.1`.
17. **Function values.** A function with an `edit` or `take` parameter cannot be used as a value (U2021), so function types are unchanged. Operations and handlers cannot declare conventions (U2020).
18. **Rules that follow** (`spec/17`, with the coordinator's defaults as answered on 2026-10-01):
    - **Markers and places.** A call-site marker is required exactly where a parameter declares a convention (U2020). The argument of `edit` is a place rooted at an editable binding; the argument of `take` is a binding.
    - **Use after `take`** is U2017 until the binding is assigned again.
    - **Exclusivity.** An `edit` argument together with any other use of its root binding in the same call is U2018.
    - **Assignment** `p = e` applies only to places rooted at an editable binding (U2019). Field paths through records are places; there are no compound operators and no element assignment.
    - **`old`** is a keyword, legal only in `ensures` and only on an `edit` parameter (U2012).
    - **Lists.** The prelude gains `set`, which updates one list element under an index `requires`.
    - **Core IR** gains internal tuples for the desugar. `take` leaves no trace there.
    - **Statements.** A call with a marker is a whole statement or `let` (U3013). An `if` without `else` is a block-form `Unit` statement.
    - **`for`** iterates over a `List`, or an `Int` range `a..<b` or `a..b`, with no `break` and no `continue`. A range loop's variable carries its bounds as a hypothesis.
    - **Reserved words.** `shared`, `iso` and `weak` are reserved for v0.3. `region` is no keyword, since the backend chooses where values live (assumption A1). `var` suggests `let edit`, and `while` and `loop` suggest `for` or recursion.
    - **Codes.** U2017 to U2021 are assigned by the coordinator, listed in `spec/17 §10`, and join `spec/13 §4` with the implementation.

## Consequences

- Programs can update accumulators, records and lists without copying when values are unique, and the reference semantics, the verifier and the information-flow rules apply unchanged to the desugared form.
- The UAM, the verifier and the label rules gain no new judgment beyond the desugar: an `edit` call is a call that returns a pair, and a loop is a recursive function.
- Effects are not rolled back by a failure. The spec and the agent guide say so with an example.

## Alternatives considered

- **References with borrowing (Rust style).** Not chosen: lifetimes in the surface conflict with machine-generated code and with the verifier's fragment.
- **Mutable aliasing (`shared`).** Deferred to v0.3: it needs ownership or alias analysis that v0.2 does not have.
- **Loops as `fold`.** Not chosen: `fold` takes a pure, non-failing, one-expression lambda, so its body could not perform operations, fail or hold statements.

## Amendment (2026-10-02)

A `for` loop lowers to a Core `Loop` node, equivalent to the generated recursive function of `spec/17 §4`: the source is evaluated once, each iteration binds the loop variable and the current values of the bindings the body updates, the body returns their new values, and a failure in the body fails the whole loop. The coordinator approved this form for issue #81 (L-Q1).

- **Call depth.** The iterations of a loop are not calls, so they do not count toward the call depth of `spec/11` (U6003). A loop over more elements than the maximum depth runs to its end.
- **Verification.** The body is checked once, for any iteration. The bindings the loop updates are fresh there, at their declared types, whose refinements are the invariant. Every other fact of the enclosing context is kept, since the loop does not change the bindings it does not update. After the loop, the updated bindings are fresh again at their declared types, so no fact about their values survives it.
- **Ranges.** An inclusive range ends at its upper bound without computing its successor, so a range up to the greatest value of a fixed-width type does not overflow.

The consequence above, "a loop is a recursive function", holds of the meaning: the node is that function, with iterations that are not calls.
