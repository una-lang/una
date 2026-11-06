# Draft: ADR 0116, Amendment (2026-11-06): values computed by the code a site reaches (IF5 item 7, issue #1685)

- Status: Draft, for the project's decision. The accepted text of ADR 0116 does not change until the project accepts an amendment. `spec/26` is unchanged meanwhile.
- Implementation: PR #1687, held. Its code refuses more than IF5's six items until this amendment is accepted, or until the project chooses the alternative (C) and the PR is reworked to it.
- Origin: finding T2 of the security hunt of #973 (Low, confirmed).

## Why

IF5 defines `τ(e)` as the join of six items: the program counter, `ℓ*` of the free variables of `e`, the latency label of each operation in its effect set (handlers included), `⊤` for an unknown function value, the deadline's label and `τ_conc`. It says `τ(e)` "is computed from headers and the call graph". A value that a function reached from `e` labels in its own body, from literals, is in none of the six items:

```una
key() -> secret Int = 3000000

work() -> Unit
=
  let k: secret Int = 3000000
  let r = k + key()
  unit
```

`attempt work() within 5 * millisecond` has a `public` `τ`, and printing whether it timed out is accepted. The same `k` bound in the caller and passed to `work(k: secret Int)` is a free variable of the site and makes `τ` `secret` (U4003). The same gap holds for `first of`, `for ever within`, the statement a `limiter` scopes over and a driven step. A kind with a `secret` field built in a callee, and a handler's operation that builds one, behave the same way.

Under T6 a literal is the same in every run, so this is no break of noninterference. Two runs that differ only in `secret` inputs still agree. But a value the program declares `secret` loses its label one call away, while the same code written at the site keeps it. A program that keeps a hard-coded key as `secret Text` and works on it in a helper gets no timing label for it.

## A. The new item

IF5 gains an item 7, and `spec/26 §7` rule 5 and §2 rule 5 gain the matching items:

7. `ℓ*` of every value that `e` computes, and that the code it reaches computes: every function in its call graph, and the operation bodies of every handler item 3 counts, their lambdas included. The labels of type parameters are left out, since the values of a type parameter come from the callers, whose own values count.

The sentence "computed from headers and the call graph" becomes "computed from headers, the bodies' values and the call graph". The other items are unchanged. Item 7 joins after them, so a message still names a read, an operation or a function value first where one of them raises `τ`.

**Spec text proposed** (moved here from PR #1687; `spec/26` is unchanged until acceptance):

- `spec/26 §7` rule 5, after item 6:

  > 7. the deep label (`spec/07 §7` rule 3) of every value that the body computes, and that the code it reaches computes: every function it can reach and the operation bodies of every handler item 3 counts, their lambdas included. The labels of type parameters are left out, since the values of a type parameter come from the callers, whose own values count. So a value labeled from literals, such as `let k: secret Int = 3000000` in a function the body calls, a labeled result or a kind with a labeled field built there, counts as the same value read from outside does under item 2 (issue #1685). The item is stricter than items 2 and 3 alone: a function reached that computes on a parameter labeled above the argument it gets counts at the parameter's label. It is computed once per function while bodies are checked, in time linear in their size.

- `spec/26 §2` rule 5, after item 6:

  > 7. the deep label of every value that `step` and the code it reaches compute, as §7 rule 5 item 7 states (issue #1685).

## B. What it newly refuses

Item 7 counts values, not only labels written in annotations. So a reached function that computes on a parameter labeled above the argument it gets now counts at the parameter's label:

```una
spin(n: secret Int) -> secret Int
=
  let edit acc: secret Int = 0
  for i in 0..<n
    acc = acc + 1
  acc

pub decision() -> public Bool
=
  let r = attempt spin(10) within 5 * millisecond
  when r
    is Ok(v) -> true
    is Err(e) -> false
```

This is accepted today and U4001 under item 7. A callee that never reads its `secret` parameter, such as `unread(n: secret Int) -> Bool = true`, called as `unread(10)`, stays accepted. The review of PR #1687 checked every case of the conformance corpus, `examples/` and the una-by-example programs, and found no other newly refused program.

**Why values, not annotations.** Every label annotation is an upper bound: `let edit acc: secret Int = 0` in `spin` reads as a declared constant just as `let k: secret Int = 3000000` does. A rule that counted only annotations (labeled `let`s, labeled results, kinds built with labeled fields) has two weaknesses:
- It counted the raise at a call argument: then `work(1)` to a function that never reads its `secret` parameter is refused. That is `first_of_timing_floor`, accepted today.
- It did not count the raise at a call argument: then it misses a literal passed to a `secret` parameter that the callee loops on.

Counting the values each body computes needs no flow analysis and has neither problem.

## C. The alternative: values built from literals are out of model

`spec/26 §7` rule 5 would state that a value built from literals, in code that a site reaches, is outside `τ`, as a constant, since T6 compares runs that agree on the program's text. Nothing is refused that is accepted today, and the preview's direction is kept: "MAY only become more precise" (amendment of 2026-10-19), "None may become stricter, except to fix soundness" (amendment of 2026-10-25, F). The cost is the inconsistency of T2: a `secret` declared in a helper has no timing label, while the same declaration at the site has one.

## D. Costs

- **Time.** One `ℓ*` of the type of each expression checked, O(size of the type), the order of the type record beside it. Each site then makes O(F + H) joins for the F functions and H handlers it already visits, and the call graph O(F + H) joins once per package. Linear in the size of the bodies. The scaling tests of the timing label pass unchanged.
- **Precision.** Section B. It is stricter than items 2 and 3 alone. It does not count a parameter that is never read.
- **Interplay.** The precise `τ_conc` (amendment of 2026-10-25, #1087) computes `κ` from the same graph, so `κ` counts item 7 with no change of its own. The `bound` label of a `start`'s reach (#1674) records part of the same information for another rule. Both can stay.

## Recommendation

Accept A. It keeps a value's label wherever the value is computed. It closes T2 for every construct with one rule. It refuses no program of the corpus but the stated kind, and it is linear. If the project prefers to keep every program accepted today, choose C, and PR #1687 is reworked to state the exclusion and drop the code. Until a decision, PR #1687 stays held, and no release ships item 7 without it.
