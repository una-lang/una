/-
Non-vacuity of #453 Phase B: a policy whose `via` function fails, on a program the whole-program
check certifies.

- The `via` function `gate` fails under a condition on its secret parameter, so the labeled checker
  refuses its body as written, and `viaTyped` types it by its retry, with the parameter's top label
  at `⊥` (two `#guard`s).
- `progG` passes the whole-program check, so it is accepted by the labeled typing, within B1, and
  its host labels are as `t6_release_host` asks (`progG_certified`).
- Its policy releases to `internal`. On 1234 the call fails with 7, and on 5 it returns `false`:
  a public observer sees neither release, and `t6_release_host` relates the two runs, a failure
  against a value (`hidden_failure_related`).
-/
import UnaFormal.F1.CheckProgramSound
import UnaFormal.F1.ReleaseExample

namespace Una.F1.FailingReleaseExample

open ReleaseExample Check

/-- `policy peekg from secret Int to internal Bool via gate`. -/
def peekg : Policy := ⟨"peekg", .int sec, .bool intl, "gate"⟩

/-- `gate` as the `via` function of `peekg`. -/
def progG : Program := { prog with policies := [peekg], funs := [gate] }

-- The labeled checker refuses `gate`'s body under its `secret` parameter: it fails under a
-- `secret` condition. Its retry, with the parameter's top label at `⊥`, types it.
#guard viaTypedAt progG gate gate.params = false
#guard viaTypedAt progG gate (gate.params.map fun q => (q.1, q.2.lowerTop)) = true
#guard viaTyped progG gate = true

-- `flowB` is defined by well-founded recursion: it is unsealed for the kernel to evaluate it.
unseal flowB flowsB in
/-- The whole-program check accepts `progG`. -/
theorem progG_certified : progG.LWellFormed ∧ progG.B1WellFormed ∧ progG.HostWF :=
  progOk_sound_nil (by rfl)

/-- `let b: internal Bool = peekg(s); b`: a release to `internal` that may fail. -/
def gBody : Expr := .letR "b" (.bool intl) (.declassify "peekg" (.var "s")) (.atom (.var "b"))

/-- What `gBody` does: it releases, and may fail at `⊥ ⊔ internal`. -/
def gEff : Eff := (Eff.ofPolicy false HostFx.none true .bot intl "peekg").union .none

theorem bot_leG (e : LExpr) : LExpr.le progG.top LExpr.bot e :=
  .inr ⟨⟨Level.public_le _, fun _ h => by simp [LExpr.bot, LExpr.ofLabel, Label.bot] at h⟩,
    fun _ h => by simp [LExpr.bot, LExpr.ofLabel] at h,
    fun _ h => by simp [LExpr.bot, LExpr.ofLabel] at h⟩

theorem gBody_typed : LExprTy progG [] Γ .bot gBody (.bool intl) gEff :=
  .letR (.declassify (po := peekg) (fn := gate) rfl rfl (.var rfl) (Flow.refl _) (bot_leG _) ⟨rfl, rfl⟩)
    (Flow.refl _) (.atom (.var rfl)) (by simp [LTy.InPlace, intl, LExpr.ofLabel])

theorem sec_hiddenG : ¬ Vis progG Label.bot (LExpr.bot.join sec) := by
  rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;>
    simp [progG, prog, Program.top, sec, LExpr.ofLabel, LExpr.join, LExpr.bot, Label.bot, Label.join,
      Level.join] at h <;> revert h <;> decide

theorem intl_hiddenG : ¬ Vis progG Label.bot intl := by
  rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;>
    simp [progG, prog, Program.top, intl, LExpr.ofLabel, Label.bot] at h <;> revert h <;> decide

theorem envs_typedG (n : Int) : LEnvTy progG (env n) (Γ.lsub progG.recordDeep []) := .cons .int .nil

theorem envs_relG (a b : Int) : EnvRel progG Label.bot (Γ.lsub progG.recordDeep []) (env a) (env b) :=
  .cons (.hidden sec_hiddenG) .nil

/-- **A hidden failing release.** On 1234 `gate` fails, so the call releases `Err(7)` and fails
with 7; on 5 it releases `false`. A public observer sees neither release, and the runs, a failure
against a value, are related as `t6_host` says: their failure label `⊥ ⊔ internal` is hidden too. -/
theorem hidden_failure_related :
    eval Host.unit 6 progG w0 (env 1234) gBody = ([.release "peekg" (.err (.int 7))], .failed 7, w0) ∧
    eval Host.unit 6 progG w0 (env 5) gBody = ([.release "peekg" (.bool false)], .ok (.bool false), w0) ∧
    proj progG Label.bot (eval Host.unit 6 progG w0 (env 1234) gBody).1 =
        proj progG Label.bot (eval Host.unit 6 progG w0 (env 5) gBody).1 ∧
      OutRel progG Label.bot (V progG Label.bot ((LTy.bool intl).lsubst progG.recordDeep []))
        (Vis progG Label.bot (gEff.inst (progG.sd [])).failLabel)
        (eval Host.unit 6 progG w0 (env 1234) gBody).2.1
        (eval Host.unit 6 progG w0 (env 5) gBody).2.1 := by
  have r₁ : eval Host.unit 6 progG w0 (env 1234) gBody = ([.release "peekg" (.err (.int 7))], .failed 7, w0) := rfl
  have r₂ : eval Host.unit 6 progG w0 (env 5) gBody =
      ([.release "peekg" (.bool false)], .ok (.bool false), w0) := rfl
  have hpk : ∀ v, ¬ EvVis progG Label.bot (Event.release "peekg" v) := fun _ ⟨po, hpo, hv⟩ => by
    cases hpo
    exact intl_hiddenG hv
  obtain ⟨hwf, hb1, hh⟩ := progG_certified
  refine ⟨r₁, r₂, ?_⟩
  rcases t6_release_host progG Label.bot Host.unit hwf hb1 hh Host.unit_lawful
      (σ := []) (fun _ h => nomatch h) (fun _ h => by simp at h) Γ_inPlace rfl (WorldRel.refl w0)
      (envs_typedG 1234) (envs_typedG 5) (envs_relG 1234 5) gBody_typed 6 (by rw [r₁]; trivial)
      (by rw [r₂]; trivial) with h | h
  · exact ⟨h.1, h.2.1⟩
  · exfalso
    obtain ⟨pre, _, _, _, _, _, _, h₁, _⟩ := h
    rw [r₁, proj_single_hidden (hpk _)] at h₁
    cases pre <;> simp at h₁

end Una.F1.FailingReleaseExample
