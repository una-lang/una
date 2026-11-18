# 0071. A length frame for loops

- Status: Accepted
- Date: 2026-10-07
- Deciders: the coordinator, under the owner's delegation (decisions N-07, 2026-10-06, and N-13, 2026-10-07, which narrowed D3), on a proposal by Owner (Una-1)
- Resolves: issue #260 (Beta), pre-tag finding S-1
- Amends: `spec/17 §5` rule 3; `spec/12 §6`; the guide §11 with the implementation

## Context

A loop is checked once, for any iteration (`spec/17 §5` rule 3, ADR 0056 amendment of 2026-10-02):
- The bindings it updates are fresh in the body, at their declared types.
- After the loop, they are fresh again.
- No fact about their values survives, beyond their types' refinements.

That is sound, but it forgets the one fact that the main in-place update needs:

```una
pub bump(edit xs: List[Int]) -> Unit
=
  for i in 0..<xs.len
    set(edit xs, index: i, item: i + 1)
```

`set` keeps the length (`ensures xs.len == old(xs).len`, `spec/14 §1`), and the range is evaluated once, before the loop, at the entry length. In the body, though, `xs` is fresh, so `i < xs.len` is unknown. The obligation is reported refuted, with a counterexample (`i = 0`, `xs.len = 0`) that no run can reach: with `xs.len = 0` the range is empty.

v0.2.0 documents the limit and its workaround, a redundant `if i < xs.len` guard. Agents meet it in the first loop they write over a list's indices. They then rebuild the list instead, which is O(n) per update where `set` is O(1) in place (ADR 0056, #30), against the performance rule.

What a fix must keep:
- **Soundness.** Every fact the verifier assumes about a fresh binding must hold on every iteration.
- **QF_UFLIA, with no quantifiers** (`spec/12`, ADR 0054). The proof cache key does not change.
- **A cost linear in the body,** with no new query per loop.
- **No invariant syntax.** There is no `invariant` clause in v0.2, and this ADR adds none.

## Decision

### D1. Length-preserving updates

A list binding that a loop updates is **length-preserving** when every path of the body that completes an iteration gives it a new value whose length is its value's at the start of that iteration. The checker decides this syntactically, over Core, with a class of names known to share the iteration's starting length. The class starts with the state binder. A name joins it when it is:

1. **An alias:** a `let` of a name in the class.
2. **A preserving `edit` call:** the component of an `edit` argument, in the tuple a call returns (`spec/17 §4`), when the argument is in the class and the callee has an `ensures` clause that is exactly `p.len == old(p).len`, or `old(p).len == p.len`, for that parameter `p`. The prelude's `set` has that clause, and so may any function of the program.
3. **A join:** the component of an `if` or `case` whose every branch that completes gives a name in the class at that position.
4. **A nested loop:** the component of a nested loop whose initial value is in the class, and which preserves it by this rule.

Anything else breaks the class:
- `append` or any other call;
- a `take` argument;
- a call whose callee lacks that exact clause, even when its body happens to keep the length;
- a fresh list, a literal, or a rebinding to another list.

A path that fails or stops does not complete an iteration, so it does not count: a failure leaves the caller's values unchanged (`spec/17 §7` rule 1).

### D2. The frame

For each length-preserving binding `x` of a loop, with initial value `x₀`:
- **In the body:** the verifier assumes `len(x) = len(x₀)`.
- **After the loop:** it assumes `len(x′) = len(x₀)` of the loop's final value `x′`.

Both are ground equalities over the uninterpreted `len` of `spec/12 §4`, so the query stays in QF_UFLIA, with no quantifier.

**Soundness,** by induction on the iterations:
- The equality holds at the first iteration, where `x = x₀`.
- Each completed iteration ends with a value in the class. Its length equals that iteration's starting length, by the callee contracts named in D1 (which are proven, or trusted where any contract is), or by being the same value.
- So the equality holds at every iteration and after the last.
- A loop with no iteration leaves `x′ = x₀`.

The frame is only about the length. No fact about the elements survives, as before.

### D3. A failure that only the loop's abstraction allows

The verifier names each value that a loop makes fresh: the state binders in its body, and the components of its final value. Take an obligation that the solver refutes, and whose counterexample gives a value to a term over one of those fresh values. That counterexample may be no run at all: the loop forgot the fact that excludes it.

Such an obligation is checked once more (decision N-13, 2026-10-07), when the values it depends on are state binders of the loops around it, or are computed from them:
- **The second query** is the first, with every enclosing loop put at its first iteration: each state binder equal to its value on entry, and the variable of a range equal to the lower bound. The frame of D2 and the refinements of the state's types stay.
- **Still refuted:** the obligation is **refuted** (U5001) with the second query's counterexample. That counterexample is a run, the first iteration's. So a loop that breaks a refinement invariant on its first iteration, as `n = n - 1` on `n: Pos` from `n = 1`, keeps its counterexample.
- **Not refuted:** the obligation is **unknown** (U5002), with no counterexample. Its reason names the bindings: "the loop forgets every fact about `xs` but the refinements of its type, and its length when the loop only `set`s a list, as `spec/17 §5` rule 3 says, so the counterexample sets `xs` freely; guard the use with an `if`, or check the value with `checked`".
- **A value from a loop's final value,** after the loop or after a nested loop, is not fixed by any first iteration. So no second query is made, and the obligation is unknown with that reason.

ADR 0053 already turns a refutation that is no execution into unknown in the same way. The program is still rejected, so this changes only the report.

### D4. Cost

- **The class:** one pass over the loop body, in time linear in its size. A nested loop's result is computed once and reused, so a nest stays linear in total.
- **The frame:** one equality per preserving binding.
- **D3:**
  - Each binder computed from a loop-fresh value is marked when it is bound. A `let` of an `if` or `case` reads the variables of its branches, so marking is linear in the body times the nesting depth of its expressions, and it only runs after a loop.
  - A refuted answer then needs a check of the terms its counterexample shows.
  - One more query, in QF_UFLIA, for each refuted obligation in a loop body whose counterexample sets loop-fresh values, and none for any other obligation.

No query is added for an obligation that is proven, or refuted outside a loop.

## Consequences

- The repro, and its `let edit` and `let n = xs.len` forms, verify without a guard. So do loops that call an `edit` helper whose contract keeps the length.
- A loop that appends, takes, or calls a helper without the clause is checked as before.
  - A failure there that hinges on the forgotten state is reported unknown, with a reason that says why, rather than refuted with an impossible counterexample.
  - A failure that the first iteration reaches stays refuted, with that iteration as its counterexample.
- **Engines:** the interpreter and the native backend are unchanged.
- **Spec:** `spec/17 §5` rule 3 gains the frame, and `spec/12 §6` gains the unknown reason.
- **The guide:** §11 drops the workaround and states the rule.
- **Tests first:** conformance cases for:
  - the repro and its variants;
  - an `edit` helper with the clause;
  - adversarial cases that must stay rejected: an append, a helper without the clause, a `take`, an alias of another list, a nested loop that appends, a branch that rebinds to a literal, and a frame after the loop that is checked against a changed length;
  - D3's reason.

  Also verifier unit tests for the class.

## Alternatives considered

- **An inductive length invariant, checked by the solver.** The verifier would assume `len(x) = len(x₀)` in the body and prove it on every completed path. That covers any body that keeps the length, but it adds an obligation per loop. A failed proof would also have to fall back silently, which is a second query and a new diagnostic path. The syntactic class covers the agents' loops at no query cost, and the solver-checked form can come later.
- **An `invariant` clause.** It is general, but it is new syntax that agents must write, and v0.2 has none. That is for a later edition.
- **Keeping every fact about the elements, as a frame for unchanged indices.** It needs quantifiers or SMT arrays, which `spec/12` and ADR 0069 M8 B rule out.
- **Only the D3 reason, with no frame.** It is honest, but it still forces a guard on the most common loop.

## Amendment (2026-10-07)

Two gaps of D3, from the soundness check of #276, filed as #278 and decided by the coordinator. Both were refutations with a counterexample that is no run:
- **A state that starts from an earlier loop's final value** is fixed by no first iteration. So, like the final value itself, it gets no second query, and a refutation that sets it is unknown.
- **The body of a loop over a list runs only when the list has an element.** So the body assumes `xs.len >= 1` of the visited list, in every query, not only the second. This is sound for every iteration.

## Amendment (2026-10-08, #404)

The reason quoted under "Not refuted" is reworded; the decision is unchanged. The dogfood pass of v0.3.3 (D-3) found that an agent could not act on it: it cited `spec/17 §5`, which agents working from the guide do not read; it spoke of a list's length for a map; and its advice, a guard or `checked`, cannot prove a postcondition. The reason now names each binding as a list or a map when it is one, names a length only for a list, cites nothing, and for a postcondition points at the pattern that proves it, the contract on a helper with an `edit` parameter that one iteration calls. `spec/12` quotes both forms.
