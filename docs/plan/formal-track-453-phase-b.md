# Formal track: #453 Phase B, `checked` in `via` bodies, and failing releases

- **Issue:** #453 (Phase A merged as #494).
- **Status:** statement draft, for the coordinator before any proof (the B1 discipline).
- **Author:** the builder session.

## 1. Why the statement changes

`via` functions are the trusted part of a policy (`spec/07 §5` rule 3): no label is checked in them. The motivating case is `tests/conformance/verification/via_result_checked_opaque`:

```una
copy(t: internal Text) -> public Slug
  fails SlugError
=
  checked(t, valid_slug(it)) else fail Invalid

pub publish(raw: internal Text) -> Slug
  fails SlugError
=
  let slug = as_slug(raw)
    if error pass
  slug
```

- `copy` fails depending on an `internal` value, and `publish` passes that failure on publicly.
- That is a declassification. The policy `as_slug` releases either a `public Slug` or the failure `Invalid`, and both reveal whether `valid_slug(raw)` holds.
- The compiler accepts this program, as designed: a policy's `uses` and `fails` are its `via` function's (`spec/07 §5` rule 1).

Today's model forbids it. B1 requires `fn.fails = false` of every `via` function, and `Expr.Pure` excludes `fail` and `checked`. In the model, a `via` that failed would end the run with no release event. Its failure would be labeled ⊥ (`Eff.ofFnR`), so a policy whose `to` is above the observer would leak through the failure with nothing in the trace to show it.

## 2. The proposed statement

The shape of `t6_release_host`, `BRelR` and `Diverged` stays. What changes is what a release carries, and when one happens.

1. **A release carries the `via` function's outcome.** `evalRhs`'s `declassify` emits `.release pol r` when the `via` function returns `r`, as today. When it fails with code `c`, it emits `.release pol (.err c)`, and the call fails with `c`. A `via` function performs no event (B1), so the release is its run's only event.
2. **B1, relaxed:**
   - `via : ∀ po ∈ p.policies, ∃ fn, … fn.op = false ∧ fn.host.isNone ∧ fn.body.PureOrFails`
   - `Expr.PureOrFails` is `Expr.Pure` plus `fail c` and `checked`. Still no call, no event, no host operation.
   - `fn.fails` may be `true`.
   - `src` and `dst` keep their scalar restrictions.
3. **T-Declassify labels the failure with what the release reveals.** The call's effects become `{ ofFnR fn.op fn.host false [pol] with fails := if fn.fails then some (pc ⊔ po.dst.top) else none }`.
   - The failure is visible exactly where the release is: `EvVis (.release pol _) ↔ Vis po.dst.top`.
   - The premise `pc ⊑ po.dst.top` stays.
4. **`Allowed` for a release:** `pol ∈ ε.rels ∧ ∃ po, p.policy pol = some po ∧ pc ⊑ po.dst.top ∧ (ValTy p v po.dst.erase ∨ ∃ c, v = .err c)`. `events_firstOrder` still holds, since `.err c` is first-order.
5. **`t6_release_host`'s statement is unchanged in form.**
   - Two related runs agree on what the observer sees, or `Diverged` at a release the observer sees.
   - `Diverged` already allows any two differing released values: `ok v` against `err c`, or `err c₁` against `err c₂`, is now a divergence too.
   - Nothing is claimed after a divergence, as today.
6. **`via` bodies stay structurally typed.** `checked` there has no label premise (`Typing.lean`'s `checked`), as `spec/07 §5` rule 3 says. Only B1's `PureOrFails` and `viaTyped` constrain them.

## 3. What it claims, and does not

- It claims a failure of a policy call is released like its value: the observer at `po.dst.top` sees the release event, and the failure is no more visible than that.
- It does not claim anything about what `via` reveals, as today: a `via` may reveal any function of its argument, including whether a predicate holds.
- `t6_release_erasure_obs` and the any-host variants keep their statements. Their proofs need `events_firstOrder`'s new release case.

## 4. Risks to check in the proof

1. **The binary case of `declassify`:** a failing `via` whose failure is hidden must be confined, which the failure label `pc ⊔ dst.top` gives (`brel_quiet`). A visible one diverges at its release or agrees.
2. **A `handle` or `attempt` around a policy call:** the handler runs under `pc ⊔ dst.top`, as T-Handle (corrected, `spec/10 §13.1`) already says for any failure label.
3. **The re-check:**
   - The importer stops rejecting `via` functions that fail.
   - `checkB1` gains `PureOrFails`.
   - The corpus's `via_result_checked_opaque` (`release.copy`, `app.publish`) should move to accepted, and nothing into rejected.

## 5. Questions for the coordinator

1. Is a release of a failure (`.release pol (.err c)`) the right observation? The alternative is a separate event, `.releaseFail pol c`. One event kind keeps `Diverged` and `EvVis` unchanged, which is why I propose it.
2. Is the failure label `pc ⊔ po.dst.top` right, or should it be `po.dst.top` alone? `pc ⊑ po.dst.top` already holds, so they are equal. I write the join so that the rule reads like T-Op's.
