/-
**T6 with declassification**, option B1 (`docs/plan/formal-track-m4.md` §3): noninterference up
to the first visible release whose values differ. The statements are stated here, before their
proofs.

A call of a policy evaluates its `via` function and emits a release event with the value it
releases (`Event.release`); an observer at `L` sees it when it sees the label of the policy's
`to`. The theorem: two runs of a typed body, related for the observer, either agree as `t6`
says, or, after a common visible prefix, release through the same policy two different values,
after which nothing is claimed. Its corollary: runs whose visible releases agree are related as
`t6` says.

B1's restrictions (`B1WellFormed`, `B1.lean`) are the model's, not the specification's: a scalar
`from` and `to`, a pure `via` function, and no release in a function value or under a raised
program counter (`Program.RelBound`). A program outside them is not re-checked.

What B1 does not claim:
- anything after the first visible release whose values differ;
- any bound on what `via` reveals of the secret it is given: that is the policy's
  responsibility, not the type system's. A delimited-release ("what") guarantee is future work.
-/
import UnaFormal.F1.T6

namespace Una.F1

variable (p : Program) (L : Label)

/-- **The statement of T6 with declassification (B1)**: `t6` without `p.policies = []`, for the
programs of option B1, up to the first visible release whose values differ. -/
def T6Release : Prop :=
  p.LWellFormed → p.B1WellFormed → p.le (.ofLabel p.acceptsGet) (.ofLabel p.getResult) →
  ∀ (w : World) {σ : List (String × LTy)} {e₁ e₂ : Env} {Γ : LCtx} {E : List String}
    {pc : LExpr} {e : Expr} {t : LTy} {ε : Eff},
    GroundSubst σ → p.EqArgsOK σ E → Γ.InPlace → pc.PlainFree →
    LEnvTy p e₁ (Γ.lsub p.recordDeep σ) → LEnvTy p e₂ (Γ.lsub p.recordDeep σ) →
    EnvRel p L (Γ.lsub p.recordDeep σ) e₁ e₂ → LExprTy p E Γ pc e t ε → ∀ n : Nat,
    BRelR p L (V p L (t.lsubst p.recordDeep σ)) (Vis p L (ε.inst (p.sd σ)).failLabel)
      (eval n p w e₁ e) (eval n p w e₂ e)

/-- **Its corollary, runs whose releases agree**: when the releases the observer sees are the
same in both runs, the runs are related as `t6` says. -/
def T6ReleaseAgree : Prop :=
  p.LWellFormed → p.B1WellFormed → p.le (.ofLabel p.acceptsGet) (.ofLabel p.getResult) →
  ∀ (w : World) {σ : List (String × LTy)} {e₁ e₂ : Env} {Γ : LCtx} {E : List String}
    {pc : LExpr} {e : Expr} {t : LTy} {ε : Eff},
    GroundSubst σ → p.EqArgsOK σ E → Γ.InPlace → pc.PlainFree →
    LEnvTy p e₁ (Γ.lsub p.recordDeep σ) → LEnvTy p e₂ (Γ.lsub p.recordDeep σ) →
    EnvRel p L (Γ.lsub p.recordDeep σ) e₁ e₂ → LExprTy p E Γ pc e t ε → ∀ n : Nat,
    Term (eval n p w e₁ e).2 → Term (eval n p w e₂ e).2 →
    (proj p L (eval n p w e₁ e).1).filter Event.isRelease =
      (proj p L (eval n p w e₂ e).1).filter Event.isRelease →
    proj p L (eval n p w e₁ e).1 = proj p L (eval n p w e₂ e).1 ∧
      OutRel (V p L (t.lsubst p.recordDeep σ)) (Vis p L (ε.inst (p.sd σ)).failLabel)
        (eval n p w e₁ e).2 (eval n p w e₂ e).2

/-- The corollary follows from the theorem: at a divergence, the releases the observer sees
differ at the first release after the common prefix. -/
theorem t6ReleaseAgree_of (h : T6Release p L) : T6ReleaseAgree p L := by
  intro hwf hb1 hget w σ e₁ e₂ Γ E pc e t ε hσ hE hΓ hpc h₁ h₂ hrel he n ht₁ ht₂ hagree
  rcases h hwf hb1 hget w hσ hE hΓ hpc h₁ h₂ hrel he n ht₁ ht₂ with hok | ⟨pre, pol, v₁, v₂, r₁, r₂, hne, hp₁, hp₂⟩
  · exact hok
  · rw [hp₁, hp₂] at hagree
    simp only [List.filter_append, List.filter_cons, Event.isRelease, ite_true] at hagree
    have := List.append_cancel_left hagree
    simp only [List.cons.injEq, Event.release.injEq] at this
    exact absurd this.1.2 hne

/-- **T6 with declassification (option B1)**: the runs of a typed body, related for the
observer, agree as `t6` says, or diverge at a visible release with different values. -/
theorem t6_release : T6Release p L :=
  fun hwf hb1 hget w _ _ _ _ _ _ _ _ _ hσ hE hΓ hpc h₁ h₂ hrel he n =>
    (t6_soundR hwf hb1 hget w n).2 hσ hE hΓ hpc h₁ h₂ hrel he

/-- **Runs whose visible releases agree** are related as `t6` says. -/
theorem t6_release_agree : T6ReleaseAgree p L := t6ReleaseAgree_of p L (t6_release p L)

end Una.F1
