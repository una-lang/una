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

What B1 does not claim:
- anything after the first visible release whose values differ;
- any bound on what `via` reveals of the secret it is given: that is the policy's
  responsibility, not the type system's. A delimited-release ("what") guarantee is future work.
-/
import UnaFormal.F1.T6

namespace Una.F1

/-- A release event. -/
def Event.isRelease : Event → Bool
  | .release _ _ => true
  | _ => false

mutual
  /-- A right-hand side that performs no event and cannot fail or call: the `via` functions of
  option B1. -/
  def Rhs.Pure : Rhs → Prop
    | .atom _ | .prim _ _ | .construct _ _ | .field _ _ | .list _ | .none | .some _ | .eq _ _
    | .lambda _ _ | .funRef _ _ => True
    | _ => False

  /-- A body that performs no event, cannot fail, and calls nothing. -/
  def Expr.Pure : Expr → Prop
    | .atom _ => True
    | .letR _ _ v b => Rhs.Pure v ∧ Expr.Pure b
    | .letE _ _ v b => Expr.Pure v ∧ Expr.Pure b
    | .ite _ t e | .caseOption _ t _ e | .caseResult _ _ t _ e | .handle t _ e =>
      Expr.Pure t ∧ Expr.Pure e
    | .attempt b => Expr.Pure b
    | .fail _ => False
end

/-- **The restrictions of option B1**, on the policies. Each `to` is a scalar type with a
concrete label, and each `via` function is pure: no operation, no failure, no call. `spec/07 §5`
requires neither (rule 1 gives a policy the `uses` and `fails` of its `via` function); they are
the model's, and cost 2.5% of the corpus's policies each. -/
structure Program.B1WellFormed (p : Program) : Prop where
  dst : ∀ po ∈ p.policies, po.dst.Ground ∧ po.dst.erase.Scalar
  via : ∀ po ∈ p.policies, ∃ fn, p.find po.via = some fn ∧ fn.op = false ∧ fn.get = false ∧
    fn.fails = false ∧ fn.body.Pure

variable (p : Program) (L : Label)

/-- **The runs diverge at a release**: after a common prefix of what the observer sees, both
release through the same policy, with different values. -/
def Diverged (t₁ t₂ : Trace) : Prop :=
  ∃ pre pol v₁ v₂ rest₁ rest₂, v₁ ≠ v₂ ∧
    proj p L t₁ = pre ++ Event.release pol v₁ :: rest₁ ∧
    proj p L t₂ = pre ++ Event.release pol v₂ :: rest₂

/-- Two runs related up to their first different release: when both end, the observer sees the
same events and related outcomes (`BRel`), or the runs diverge at a release. -/
def BRelR (R : Val → Val → Prop) (vf : Prop) (r₁ r₂ : Trace × Res) : Prop :=
  Term r₁.2 → Term r₂.2 → (proj p L r₁.1 = proj p L r₂.1 ∧ OutRel R vf r₁.2 r₂.2) ∨
    Diverged p L r₁.1 r₂.1

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

end Una.F1
