# Formal track: M4, generic functions

- Issue: #46
- Plan: `docs/plan/formal-track.md` §7 (M4: T6 with generics)
- Previous step: `docs/plan/formal-track-m3.md`
- Date: 2026-10-09
- Status: T6 for generic functions is proven; declassification follows M2 (the coordinator's order)

## 1. What is proven

All of it is in `formal/UnaFormal/F1/`. `scripts/formal.sh` passes with 59 stated theorems, and none of them depends on `sorry`.

| File | Result |
| --- | --- |
| `Subst.lean` | Rule 8's static order and flows are stable under instantiation (`LExpr.le_inst`, `Flow.lsubst`), and the labeled context instantiates (`LCtx.lsub`). |
| `InPlace.lean` | The typing rules keep `ℓ_T` in place (`LExprTy.inPlace`): a well-typed body's types carry a plain symbolic label only at a position of type `T`. |
| `InstVal.lean`, `BinaryInst.lean` | The value relations move between a generic type and its instance: raising, hiding and lowering a value's top label under a substitution, the decisions of T-App, equality at an instance, and primitives at an instance. |
| `UnarySound.lean` | The unary half, now parametric: a body typed in a generic context runs well at every ground instance of it. |
| `T6.lean` | **T6 for F1 with generic functions** (`t6`, parametric in a ground substitution `σ`), and `t6_ground` for a body typed with no type parameters in scope, which is what `main` and every call site is. |
| `Mono.lean` | `t6_ended` now rests on `t6_ground`. |
| `T6Example.lean` | Non-vacuity of the generic case: `pick[secret Int]` is typed and called, and two runs with different secrets are related (`genericCall_noninterferent`). |

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
