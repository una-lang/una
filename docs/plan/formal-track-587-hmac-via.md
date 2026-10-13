# Formal track: #587, the failure of an hmac-only `via` function is public (ADR 0098)

Status: definitions and statements approved as the plan. The answers to the open points are folded in below. No proof is written and nothing is built yet.

## What F1 has today

- No `Keys` capability and no `Hashed` type. Host operations are `op` (console), `read`, `write`, `request` and the environment (`Eval.lean`, `Host`).
- B1 asks every `via` body to be `PureOrFails`: no call and no host operation (`B1.lean`). So a `via` function that calls `Keys.hmac` is not re-checked, and the importer reports it as such.
- T-Declassify gives a policy call the effects `Eff.ofPolicy fn.op fn.host fn.fails pc po.dst.top pol`, whose failure is labeled `pc ⊔ dst` (`LTyping.lean`, #453 Phase B, ADR 0090).

## Definitions

**D1. Keys in the host.** `Host` gains two fields with defaults, so that every existing host is unchanged:

```lean
  /-- The keys the run was granted, by name (`keys:` grants): run configuration, fixed for a
  host, so the runs that T6 compares share it. -/
  key : String → Option String := fun _ => none
  /-- HMAC-SHA-256 of a text under a key, as a text: deterministic. -/
  mac : String → String → String := fun _ _ => ""
```

The grants are a field of `H`, not of `World`. Every T6 statement already fixes one `H` for both runs, so ADR 0098 rule 4's premise "the granted keys are shared" is the model's structure, not an extra hypothesis.

**D2. The operation.**
- `Rhs.hmac (key data : Atom)`.
- `eval`: when `key` evaluates to `.text k` and `data` has a `Val.line` rendering `s`:
  - `H.key k = some m` gives `([], .ok (.text (H.mac m s)), w)`;
  - `H.key k = none` gives `([], .failed keyFailure, w)`, where `keyFailure` is `KeyError.UnknownKey`'s code.
  - It records one ghost event, `Event.keyUse k`, and leaves the world unchanged. Otherwise the run is `stuck`.
  - `EvVis (.keyUse _)` is `False`, as for `Event.perf` (#561). No observer sees it, so T6, `BRel` and `WorldRel` keep their statements.
  - In confinement, `Auth` gains `keys : Bool := false`, and `Auth.allows (.keyUse _) := a.keys = true`. The ceiling that admits an hmac call is one whose `keys` is set. A boundary module whose ceiling does not grant `Keys` cannot reach it (ADR 0093). `Rhs.within a (.hmac _ _) := a.keys`.
- `Hashed` is imported as `Text`, labeled as the checker labels it. Equality is the only operation Una gives `Hashed`, and `Text` has it.

**D3. T-Hmac** (`spec/07 §4`, ADR 0039).
- `@k.hmac(key, data)` types when `key : Text` with a top label below `public`; its `key_name` accepts `public`.
- `data : Text` may have any label.
- The result is `Text` labeled `internal ⊔ ℓ*(data)`.
- The failure is labeled `pc`: the key name, which decides it, is public.
- `HostFx` gains `ky : Bool`, the `uses Keys` accounting. `Fun.host` declares it.

**D4. The shape** (ADR 0098 rule 2, in Core's terms):

```lean
/-- `@k.hmac("literal", d)`: a key name that is a text literal. -/
def Rhs.HmacLit : Rhs → Prop
  | .hmac (.text _) _ => True
  | _ => False

/-- `@k.hmac("literal", d)` with its failure passed on, or with `if error fail V` of a code. -/
def Expr.HmacGuarded : Expr → Prop
  | .letR _ _ v (.atom _) => v.HmacLit                              -- `if error pass`
  | .handle (.letR _ _ v (.atom _)) _ (.fail _) => v.HmacLit        -- `if error fail V`, no payload
  | e => e.Pure

/-- A body whose only failure points are top-level hmac calls with a literal key name. Branches
and handlers hold pure code only, so no hmac call is under a branch. -/
def Expr.HmacOnly : Expr → Prop
  | .atom _ => True
  | .letR _ _ v b => (v.Pure ∨ v.HmacLit) ∧ b.HmacOnly
  | .letE _ _ v b => v.HmacGuarded ∧ b.HmacOnly
  | .ite _ t e | .caseOption _ t _ e | .caseResult _ _ t _ e | .caseSum _ _ _ t e =>
    t.Pure ∧ e.Pure
  | .attempt b => b.Pure
  | _ => False
```

`fail`, `raise`, `checked`, calls, `invokeOp`, `withH`, loops and policy calls are outside the shape. A failure with a payload (`raise`) is outside. An hmac call under a branch is outside, because a branch holds only `Pure` code.

**D5. B1 extended.** `Program.B1WellFormed.via` gains a second disjunct. The `via` function may have `fn.host = HostFx.keys` (only `ky`), when `fn.body.HmacOnly`. Every other `via` keeps today's conditions.

**D6. T-Declassify, narrowed.** A second rule, `declassifyHmac`, has the premises of `declassify` and `(p.find po.via).body.HmacOnly`. It gives:

```lean
def Eff.ofPolicyHmac (pc : LExpr) (pol : String) : Eff :=
  ⟨false, HostFx.keys, some pc, [pol], true, [], LExpr.bot, []⟩
```

That is the release, the `Keys` accounting, and a failure labeled `pc` (ADR 0098 rule 3 with $\ell_f = \mathit{public}$).

**D7. The package-level premise** (ADR 0098 rule 2, no handler of `Keys` installed by the program's code). In the model, `Rhs.hmac` is the host's `Keys` by construction. A call that a handler implements is `invokeOp "hmac"`, which is outside `HmacOnly`. So the premise is the importer's:
- it reads `@k.hmac(...)` as `Rhs.hmac` only when the package passes the checker's `plain_vias` premise;
- it reads it as `invokeOp` otherwise.

An import test pins both cases.

## Statements

`Outcome.Ends` (the run neither stuck nor out of fuel) and `Outcome.failCode : Option Nat` (the code of a `.failed`, or `none`) are two small new definitions over `eval`'s outcome.

**S1. Silence up to ghosts** (`B1.lean`, as `silent_run`). The run's only events are `keyUse` ghosts, and it leaves the world as it was.

```lean
theorem hmacOnly_silent {e : Expr} (he : e.HmacOnly) (env : Env) :
    (∀ ev ∈ (eval H n p w env e).1, ∃ k, ev = .keyUse k) ∧ (eval H n p w env e).2.2 = w
```

**S2. The failure is decided by nothing the environment holds.** If both runs end (neither is `stuck` nor out of fuel), they agree on whether they fail, and on the code.

```lean
theorem hmacOnly_failure_env_free {e : Expr} (he : e.HmacOnly) (env₁ env₂ : Env)
    (h₁ : (eval H n p w env₁ e).2.1.Ends) (h₂ : (eval H n p w env₂ e).2.1.Ends) :
    (eval H n p w env₁ e).2.1.failCode = (eval H n p w env₂ e).2.1.failCode
```

The proof is an induction on `HmacOnly`:
- a `Pure` part cannot fail;
- `Rhs.hmac (.text k) _` fails exactly when `H.key k = none`, which depends on neither environment.

The termination channel, a run that is stuck or out of fuel before an hmac call, is excluded by `Ends`, as ADR 0098 rule 4 and ADR 0057 say.

**S3. Typing.** `HmacOnly` bodies are `viaTyped` when T-Hmac is added (`CheckProgram.lean`), and `checkB1` accepts D5 (`checkB1_sound` extended).

**S4. T6, unary** (`UnarySound.lean`). The `declassifyHmac` case: a failure of the call at program counter `pc` raises no event above `pc` and, by S1, leaves the world as it was.

**S5. T6, binary** (`T6.lean`, `BinaryR.lean`). The `declassifyHmac` case:
- when `pc` is visible to the observer, both runs reach the call;
- by S2, both fail with the same code, or both succeed;
- when both succeed, the release event is visible at `dst.top` or hidden, as today;
- when both fail, they emit the same `.release pol (.err c)`, by S2.

`t6_release_host` and the erasure and host-free variants keep their statements. Their hosts already fix `key` and `mac`.

**S6. Non-vacuity** (`HmacViaExample.lean`):
- a program whose policy's `via` is `@keys.hmac(key_name: "pepper", data: x)` with `if error pass` passes `checkB1` and the whole-program check;
- two runs with different `from` values and a host without `"pepper"` both fail with `UnknownKey`, related at a public observer (`#guard` on both runs);
- the same with a host that has it: both succeed, and the releases are hidden from a public observer.

**S7. The importer.**
- It reads `Keys.hmac` with a literal key name, `Hashed` as `Text`, and the shape of D4, under D7's premise.
- It emits `Rhs.hmac` only when the package meets ADR 0098 rule 2's premise. That premise fails when a handler of `Keys` is named outside its declaration and examples, when a handler is written `for` an alias, or when a `with` is in the body. In each of those cases the call imports as `invokeOp`, outside the shape.
- Synthesized documents test both directions, one for each case of the checker's `plain_vias`, so that the formal premise and `via.rs` cannot drift apart.
- `examples/events` is re-checked.
- The corpus and replay deltas are reported. Flips are expected only from not-re-checked to re-checked.

## Cost

D1, D3 and D5 change shared structures:
- `Host` gains two defaulted fields;
- `HostFx` gains one field, which every `HostFx` literal and every lemma over `HostFx.Le`/`or` must cover;
- `Rhs` gains one constructor, which every exhaustive match must cover (eval, typing, erasure, monomorphization, host-free, trace safety, confinement).

That last item is the bulk of the work, as `Rhs.invokeOp` was for #538.

## Decisions

1. `Hashed` is modeled as `Text`.
2. An hmac call records a ghost event, `Event.keyUse`, with `EvVis` false. Confinement admits it under a ceiling with `keys` set.
3. #587 covers `via` functions only. `Rhs.hmac` in ordinary functions is #588.
