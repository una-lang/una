# 0099. The inferred error kind follows the body's source order

- Status: Accepted
- Date: 2026-10-13
- Deciders: Una project (accepted on 2026-10-13)
- Resolves: issue #609
- Target: every edition. What a program means, and whether it is accepted, do not change: only which site of a conflict is reported, and with which code. The Gate 1 pin does not move.
- Implementation: #609.
- Amends: `spec/04 §5` rule 3, and the texts of U3008 and U3009

## Context

A private function without `fails` has its error kind inferred (`spec/04 §5` rule 3, ADR 0006). A body whose failures have two kinds is rejected: `spec/10 §1` joins two error kinds only when they are equal or one is empty. The specification does not say which kind is inferred, so it does not say where the conflict is reported, nor whether the report is U3009 (at a `fail`) or U3008 (at an `if error pass`).

**Today.** The checker records the failures of each body: each variant raised and each failure passed on. It then reads them to a fixed point. In each round, it reads the facts of every function in declaration order. A function takes the kind of the first fact whose kind is known in that round, and keeps it. A pass of a callee whose kind is inferred too is unknown until the callee is decided, so it is skipped. Within one function this follows source order, which is what the issue's three probes show. Across functions, it follows declaration order:

```una
f(n: Int) -> Int
=
  let x = g(n)
    if error pass
  if x < 0 then fail A1 else x

g(n: Int) -> Int = if n < 0 then fail B1 else n
```

With `g` declared after `f`, `f` gets kind `A` (the pass is skipped in the first round), and the conflict is U3008 at the pass. With `g` declared before `f`, `f` gets kind `B`, and the conflict is U3009 at `A1`. The body of `f` is the same in both.

The result is deterministic, but moving a declaration changes the reported site and code, and so the repair that an agent follows: "use `if error fail` with a variant of `A`" in one order, "`A1` is a variant of `A`, and this code fails with `B`" in the other.

## Decision

1. **The inferred kind is the kind of the body's first failure in source order.** A failure is a variant raised (`fail V`, with `checked ... else`, `if error` and `if none`), or a failure passed on with `if error pass`, whose kind is that of the call it handles. Source order is the order of these sites in the text. A failure of no known kind decides nothing; the next one does. Examples are the pass of a call that cannot fail, and a failure whose kind is already in error, which is reported elsewhere.
2. **A pass of a function whose kind is inferred waits for that kind.** The kinds therefore do not depend on the order of the declarations.
3. **Groups decide together.** The undecided functions may each wait for another. Then consider the groups of functions that pass each other's failures, directly or through one another (strongly connected), and that pass no failure of an undecided function outside the group. In each such group:
   - each function whose later failures, ignoring passes of the group's own functions, include one of a known kind takes the kind of the first of them;
   - the others then go on by rules 1 and 2;
   - if no function of the group has such a failure, no function of the group fails.

   All groups decide at once, so the order of the declarations plays no part here either. A function that only waits for a group is not part of it: it takes the kind the group gives it.
4. **Every failure of another kind is reported where it is:** U3009 at the variant of a `fail`, U3008 at an `if error pass`. As today, every such site is reported, not only the first.

Acceptance is unchanged, in every edition and for every order of the declarations. Two properties carry it:

- **Soundness of the inferred kind.** A function is inferred to have no kind, that is to fail in no way, if and only if no failure of a known kind reaches it: none of its own, and none through any chain of passes. So a function that can fail is never inferred "cannot fail", whatever the order of the declarations and whatever the groups.
- **Every conflict is rejected.** When a kind is inferred, it is the kind of a failure that reaches the function. Rule 4 rejects every other kind at its site, so a body with two kinds is rejected whichever kind rules 1 to 3 pick.

The implementation guards both with tests:

- A randomized test over generated programs checks the first property, both ways. Its programs include failures whose kind is in error, and it checks that an error type is never inferred.
- It also checks that renumbering the functions, as reordering the declarations does, renumbers the result and changes nothing else.

**Editions.** The rule applies to every edition, since it changes only diagnostic placement. The test run of the implementation confirmed the scope: no expected output of edition `0.1` or `0.2` changed (conformance cases, snapshots, CLI tests).

**Complexity.** Without groups, the inference reads each fact once and wakes each waiting function once: linear in the facts. Each round of groups adds one strongly connected components pass over the facts still unread (Tarjan, linear). There are as many rounds as nested groups, at most the number of functions. Today's loop is quadratic in the same worst case.

## Options considered

- **(A) Source order, waiting for callees (chosen).** The rule reads like the code: the first failure an agent sees in the body decides. It is independent of declaration order.
- **(B) Leave the site unspecified.** The kind would be one of the body's kinds and the conflict reported at a site of another. This is honest but gives agents no fixed repair, and the issue's acceptance criteria ask for a rule.
- **(C) Specify today's order.** The rule would be: rounds over declaration order, skipping unknown passes. It is hard to state, and it ties a diagnostic to where an unrelated function is declared.

For the tie rule of groups:

- **Each function's own first failure (chosen).** It is order-independent, and every conflicting pass inside the group is still reported.
- **One kind for the whole group, from the "first" function.** This needs an order between functions, which brings back declaration order.

## Consequences

- `spec/04 §5` rule 3 gains the rule. The explain texts of U3008 and U3009 cite it.
- Conformance cases pin the issue's three probes, the cross-function probe in both declaration orders, and two group shapes.
- Only cross-function cases change. In them, a conflict may move from a pass (U3008) to a `fail` (U3009), or the reverse, and a group whose functions pass each other's failures may report a conflicting pass in each function instead of one variant.
