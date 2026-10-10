# Formal track: #453 Phase B, `checked` in `via` bodies, and failing releases

- **Issue:** #453 (Phase A merged as #494).
- **Status:** statement approved by the coordinator (2026-10-10), with the decisions of §5. The language side is S-18 (#496, ADR 0090).
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
   - The corpus's `via_result_checked_opaque` (`app.publish`) should move to accepted, and nothing into rejected. `release.copy` stays not re-checked, as every `via` function is: B1 and its structural typing check it, no label rule does.

## 5. Decisions (the coordinator, 2026-10-10)

1. **One event kind.** A failing release is `.release pol (.err c)`, not a separate `.releaseFail`, so `Diverged` and `EvVis` stay unchanged. A successful release never carries `.err`: `po.dst` is scalar (B1), and `ValTy p v po.dst.erase` holds of the released value, so a lemma, `release_ok_not_err`, shows that `ValTy p v t` for a scalar `t` excludes `.err c`, with a `#guard` on one instance. The two cases of `Allowed` (§2.4) are then disjoint.
2. **The failure label is `pc ⊔ po.dst.top`.** It equals `po.dst.top` under the premise `pc ⊑ po.dst.top`, and is written as a join so the rule reads like T-Op's. It is the label that ADR 0090 gives the language's policy calls, so the model and the checker agree.

## 6. Results (2026-10-10)

- **The model.** `evalRhs`'s `declassify` emits `.release pol (.err c)` when the `via` function fails with `c`, and the call fails with `c` (`Eval.lean`). `Expr.PureOrFails` and `silent_run` (`B1.lean`): a body that may only fail performs no event and leaves the world as it was. B1's `via` asks `PureOrFails`, and `Pure` of a `via` that does not declare `fails`, so its run still cannot fail.
- **T-Declassify** gives the call `Eff.ofPolicy`: the release, and a failure labeled `pc ⊔ po.dst.top` when the `via` declares `fails` (`LTyping.lean`, `Check.lean`).
- **`Allowed`** of a release accepts `ValTy p v po.dst.erase ∨ ∃ c, v = .err c` (`Unary.lean`). `events_firstOrder` gains the `Err(c)` case.
- **Proofs.** The unary case (`UnarySound.lean`) and the binary case (`T6.lean`) of `declassify` cover a failing `via`. In the binary case, two releases are equal when seen, or the runs diverge; a value against a failure always diverges when seen, by `release_ok_not_err` (a value of a scalar type is never `Err(c)`); when the release is hidden, so is the failure. `t6_release_host`, the erasure and host-free variants keep their statements.
- **The re-check.** `checkB1` asks `exprPureOrFails`, and `viaTyped` retries a `via` body with its parameters' top labels at `⊥` when T-Checked's premise stops it: no label is checked in a `via` function (`spec/07 §5` rule 3), and the structural typing is the same (`viaTyped_sound`).
- **Examples.** `ReleaseExample.lean` adds a failing `via` (`gate`), whose run releases `Err(7)` and fails with 7 (`#guard`), and `guard_ok_not_err`. `FailingReleaseExample.lean` (non-vacuity, from the adversarial review): `gate` is typed only by `viaTyped`'s retry (`#guard`s on both disjuncts), the program `progG` passes the whole-program check (`progG_certified`), and `t6_release_host` relates a run that fails and one that returns a value through a release to `internal`, hidden from a public observer (`hidden_failure_related`).
- **A limit of the retry.** It lowers the parameters' top labels only: a `let` inside the `via` body whose annotation keeps a label above the condition it decides still stops the check, and the declaration stays not re-checked, never rejected. The compiler's own temporaries are such `let`s: `if x > 3 then fail TooBig else x` is exported as `let %t0: secret Bool = x > 3` (`written: false`) before the `if`, so the S-18 twins `s18_ok_via_failure_public_to` (both editions) stay not re-checked, now for the `via` function's structural typing instead of B1. Lowering the top labels of the body's `let` annotations in the retry too, which keeps their structure, would accept them; it is a follow-up.
- **Corpus** (an export of main at 56ce83ed by its own compiler, against main's re-check): `app.publish` moves from not re-checked (B1) to accepted, 760 to 761 accepted, none rejected. The only other change is the reason of the four declarations of the S-18 twins above, not re-checked before and after.
- `scripts/formal.sh`: PASS, 212 theorems, no `sorry`.
