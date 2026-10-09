# Formal track: M4, generic functions

- Issue: #46
- Plan: `docs/plan/formal-track.md` §7 (M4: T6 with generics)
- Previous step: `docs/plan/formal-track-m3.md`
- Date: 2026-10-09
- Status: T6 for generic functions is proven, and declassification (option B1, §5)

## 1. What is proven

All of it is in `formal/UnaFormal/F1/`. `scripts/formal.sh` passes with 60 stated theorems, and none of them depends on `sorry`.

| File | Result |
| --- | --- |
| `Subst.lean` | Rule 8's static order and flows are stable under instantiation (`LExpr.le_inst`, `Flow.lsubst`), and the labeled context instantiates (`LCtx.lsub`). |
| `InPlace.lean` | The typing rules keep `ℓ_T` in place (`LExprTy.inPlace`): a well-typed body's types carry a plain symbolic label only at a position of type `T`. |
| `InstVal.lean`, `BinaryInst.lean` | The value relations move between a generic type and its instance: raising, hiding and lowering a value's top label under a substitution, the decisions of T-App, equality at an instance, and primitives at an instance. |
| `UnarySound.lean` | The unary half, now parametric: a body typed in a generic context runs well at every ground instance of it. |
| `T6.lean` | **T6 for F1 with generic functions** (`t6`, parametric in a ground substitution `σ`), and `t6_ground` for a body typed with no type parameters in scope, which is what `main` and every call site is. |
| `Mono.lean` | `t6_ended` now rests on `t6_ground`. |
| `T6Example.lean` | Non-vacuity of the generic case. `pick[secret Int]` is typed and called, and two runs with different secrets are related (`genericCall_noninterferent`). The result of `pick[public Int]` is equal in two runs that differ in a secret, by the relation at the instance type alone (`pickPublic_result_equal`). |

**The theorem.** It is the theorem of M3, with one change. A body may call generic functions at ground type arguments, and closures may come from generic bodies. The relation of two closures at a visible function type carries the substitution under which their body was typed.

**What a generic function's typing gives.** Its body is typed once, with `ℓ_T` and `ℓ*_T` in its labels. `t6` holds at every ground substitution of its type parameters. Its labels are read through `LExpr.inst`, and its types through `LTy.lsubst`. No step of the proof re-types the body at the instance.

## 2. Assumptions, and what M4 found

Each assumption is a hypothesis of `t6` or a field of `Program.LWellFormed`.

1. **One level of generics.** The type arguments of a call are ground (`LTy.GroundAll`). The plan's §3 puts F1 there.
2. **`ℓ_T` only in place** (`LTy.InPlace`). A plain symbolic label labels only a position of type `T`. The reading of `ℓ_T` as "the labels the value of `T` was passed with" needs it. An `ℓ_T` on a `Bool` would be dropped by instantiation, and a branch on it would leak. The rules keep the invariant, and well-formedness asks it of signatures. v0.3.4's Core export breaks it in one place, in 13 lets of the corpus: `to_text` on a value of `T` gives a text labeled `ℓ_T`. The `concat` of the interpolation that follows gives `ℓ*_T`. #440 stars the label, adds a test over the corpus, and adds a `debug_assert!` that holds the checker to the invariant. `spec/07 §7` rule 8 states it.
3. **Signatures are scoped.** A signature mentions only its own type parameters (`sigsScoped`), a function constrains by `Eq` only its own (`eqScoped`), and records declare ground fields (`recordsGround`). The checker's U-codes for unknown type names enforce all three.
4. **No policy** (`p.policies = []`). Declassification comes after M2, as option B1 (§3).

**Logged for later, not a finding.** At a generic call, the checker raises by the decisions an argument carries at its top. The model reads them per position. The checker is the stricter of the two, so the asymmetry costs precision, not soundness. It is in the backlog.

## 3. What remains

- **Declassification, option B1** (the coordinator's decision, 2026-10-09). A body's release becomes an effect at `ℓ(to)`, so a function that declassifies can be called only under a program counter at most `ℓ(to)`. The theorem: two runs agree on every observable event and outcome, or their first observable difference is a pair of releases at the same point with different values. "Runs whose releases agree" is a corollary.
  - **Coverage gap, stated:** Una accepts a call of a declassifying function under a raised program counter, and that is sound, since the release is not observable and the result is raised. The theorem says nothing about those programs.
  - **Option B2**, gradual release in the knowledge style with no restriction, is later research.
- **M2**, first: the Lean checker over `una core --json` and the replay of S-01 to S-12.

## 4. Effort

The four commits of M4 span 09:31 to 10:11 local time, 2026-10-09, after the M3 merge: about an hour of one session. As in M3, the cost was in Lean details, not in the proof's ideas. The one place where the model met the checker, `ℓ_T` in place, became #440.

## 5. Declassification, option B1

**The theorem.** `t6_release` (`formal/UnaFormal/F1/Release.lean`) is `t6` without `p.policies = []`, for the programs within B1's restrictions. Take two runs of a typed body, related for an observer at `L`. Then one of two things holds:

- **They agree as `t6` says:** the observer sees the same events, and the outcomes are related.
- **They diverge at a release:** after a common prefix of what the observer sees, both runs release through the same policy, with different values (`Diverged`). Nothing is claimed after that point.

The corollary `t6_release_agree` covers runs whose visible releases agree: they are related as `t6` says.

A declassification emits a release event with its value. The observer sees the event when it sees the label of the policy's `to`.

**The rules, as `spec/07` states them, plus one effect.**

- **The site.** T-Declassify keeps rule 2 of `spec/07 §5`: the program counter is below the label of `to`.
- **The effect.** A function declares the policies it may release through (`Fun.rels`). A call needs the program counter below the `to` of each of them (`Program.RelBound`).
- **Function values.** A function value may not release: neither a lambda nor a reference to a function that declassifies.

The statements and their proofs:

| Statement | Proof |
| --- | --- |
| `unary_soundR`, the unary half | A declassification runs a pure `via` function (`pure_run`), returns a value of the scalar `to` (T1), and releases it at a program counter below the `to`. |
| `t6_soundR`, the binary half | A divergence passes through sequencing (`andThen_brelR`). Closures release nothing, so their runs stay related as before. At a declassification, both runs release either hidden values, equal values, or the divergence itself. |
| `t6`, `t6_ground`, `t6_ended`, `t6_sound`, `unary_sound` | Unchanged. They are the case without policies, where no release is visible. |

**The restrictions** (`B1WellFormed`) are the model's. `spec/07 §5` requires none of them. Of the corpus's 120 policies:

| Restriction | Policies outside it |
| --- | --- |
| A scalar `from` | 0 |
| A scalar `to` | 3 (2.5%) |
| A pure `via` function: no operation, no failure, no call (`Expr.Pure`) | 4 (3.3%) |

**What the re-check reports.** A declaration outside B1 is reported as not re-checked, with the restriction, and never as rejected. Una accepts such programs, so that is unknown, not a finding (ADR 0008). The restrictions are:

- a non-scalar `from` or `to`;
- an impure `via` function;
- a call of a declassifying function under a raised program counter;
- a declassification in a lambda or as a value.

**Why Una is sound where B1 excludes.** Take a call of a declassifying function under a raised program counter. Una accepts it, and is sound to do so, for three reasons:

- A release is not an observable event in Una.
- `bound(f)` already keeps any public effect inside `f` from happening under a raised program counter (`spec/07 §3` rule 4).
- The call's result is raised.

The exclusion is the model's, and it is needed because B1 makes the release an event. A function body is typed at `⊥`, so a release inside it would be visible in one run and absent in the other. Covering these calls would need either a dynamic program counter in the semantics or releases that cannot be observed.

**What B1 does not claim.**

- Anything after the first visible release whose values differ.
- Any bound on what `via` reveals of the secret it is given. That is the policy's responsibility, not the type system's.
- A delimited-release ("what") guarantee. That is future work.

**Non-vacuity** (`ReleaseExample.lean`). The examples use a program whose policies release whether a secret PIN is 1234:

| Example | What it shows |
| --- | --- |
| `pin_diverges` | PINs 1234 and 1 release different values, and the runs diverge at the release. |
| `guesses_agree` | PINs 1 and 2 release the same value. The corollary gives `t6`'s full conclusion although the secrets differ. |
| `hidden_release_related` | A release to `internal` is hidden from a public observer, and the runs are related although their released values differ. |
| `release_under_secret_untyped` | A call of a declassifying function under a `secret` condition has no typing. |

**Candidates to lift later.**

- **The scalar `from`.** It only spares the proof a lemma from the labeled to the structural typing of values (`LValTy` to `ValTy`), which closures make long.
- **`checked` in a `via` function.** It is a refinement check, which F1 does not have.

**A correction to M2's totals.** On `main` (#446, #447), the re-check reported "accepted" for declarations in documents that have policies. `t6` assumed `p.policies = []`, so those accepts had no theorem behind them. B1 gives them one.

- **What is checked.** Every declaration is checked in a program that holds all of its document's policies, with every `via` body read. `checkB1` checks B1's restrictions over all of those policies, not only the ones the declaration reaches.
- **What the corpus shows.** `main` accepted 25 declarations in the 13 documents with a policy. On this branch, 23 of them are accepted and backed by `t6_release`. The other 2 move to not re-checked:
  - `app.publish` of `verification/via_result_checked_opaque`, whose `via` function uses `checked`;
  - `events.seating.clamp_seats` of the `events` example, whose document has a policy to `Hashed`.
- **What does not change.** No declaration moves into rejected, and the replay of S-01 to S-12 is unchanged.
- **The guard.** `ImportTests.lean` fails the build if a declaration is accepted in a document whose policies are outside B1, even one that declassifies nothing.

**A second correction: no accept without a theorem** (the adversarial review of #452). The re-check checks each declaration against its callees' signatures. T6 needs every function of the program to pass T-Fun. So on `main` a caller was "accepted" even when a callee it reaches was not re-checked or was rejected, with no theorem behind it.

- **The rule.** A declaration is now accepted only when every function it reaches, transitively, is accepted too. It reaches a function through a call, a call inside a lambda, or a use as a value.
- **The exceptions.** A `via` function is covered by `checkB1` instead. The prelude's functions are axioms.
- **Precedence.** A declaration's own rejection is kept.
- **What the corpus shows.** 35 declarations in 22 documents move from accepted to not re-checked:
  - 29 reach a function that is not re-checked. The most common causes are `div` (11), `prelude.append` (4) and generic records (4).
  - 6 reach a function that is rejected for want of #443's export keys.
- **Release sets.** They now count the declarations inside lambdas too, so a caller is never checked against a set that misses one (Una-1's case).
- **Unreadable policies.** A policy F1 cannot read makes its document not re-checked.
- **The proof to come.** The theorem of M4 completion part A2, "every declaration accepted ⇒ the whole program passes `LWellFormed`", will prove this rule.

The corpus totals, `main` → B1: 647 → 610 accepted, 7 → 7 rejected, 1,044 → 1,081 not re-checked. No declaration moves into rejected, and the replay of S-01 to S-12 is unchanged.

`scripts/formal.sh` passes with 70 stated theorems.
