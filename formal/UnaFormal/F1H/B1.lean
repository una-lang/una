/-
The restrictions of option B1 (declassification, `docs/plan/formal-track-m4.md` §3) on a
program's policies, and what a pure `via` function's run is: no event and no failure.
-/
import UnaFormal.F1H.LTyping

namespace Una.F1H

open Una.F1 (Label LExpr)

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

/-- **The restrictions of option B1**, on the policies. Each `from` is a scalar type, each `to`
is a scalar type with a concrete label, and each `via` function is pure: no operation, no
failure, no call. `spec/07 §5` requires none of them (rule 1 gives a policy the `uses` and
`fails` of its `via` function); they are the model's. Of the corpus's 120 policies, none has a
`from` that is not a scalar, 3 have a `to` that is not a scalar (2.5%), and 4 have a `via`
function that is not `Pure` as written here (3.3%). The scalar `from` is a candidate to lift: it
only spares the proof a lemma from the labeled typing of values to their structural typing
(`LValTy` to `ValTy`), which the closures make long. -/
structure Program.B1WellFormed (p : Program) : Prop where
  src : ∀ po ∈ p.policies, po.src.erase.Scalar
  dst : ∀ po ∈ p.policies, po.dst.Ground ∧ po.dst.erase.Scalar
  via : ∀ po ∈ p.policies, ∃ fn, p.find po.via = some fn ∧ fn.op = false ∧ fn.host.isNone = true ∧
    fn.fails = false ∧ fn.body.Pure

/-- Without policies, the restrictions hold. -/
theorem Program.B1WellFormed.of_nil {p : Program} (hnil : p.policies = []) : p.B1WellFormed where
  src := by rw [hnil]; intro _ h; nomatch h
  dst := by rw [hnil]; intro _ h; nomatch h
  via := by rw [hnil]; intro _ h; nomatch h

variable {H : Host}

/-- A run from `w` with no event, that does not fail, and leaves the world as it was. -/
def Quiet (w : World H) (r : Run H) : Prop := r.1 = [] ∧ (∀ c, r.2.1 ≠ .failed c) ∧ r.2.2 = w

theorem quiet_andThen {w : World H} {first : Run H} {rest : World H → Val → Run H}
    (h₁ : Quiet w first) (h₂ : ∀ v, Quiet w (rest w v)) : Quiet w (andThen first rest) := by
  obtain ⟨t, r, w'⟩ := first
  obtain ⟨ht, hr, hw⟩ := h₁
  simp only at ht hr hw
  subst ht hw
  cases r with
  | ok v =>
    simp only [andThen]
    have := h₂ v
    rcases hq : rest w' v with ⟨t', r', w''⟩
    rw [hq] at this
    exact ⟨by simpa using this.1, this.2.1, this.2.2⟩
  | failed c => exact absurd rfl (hr c)
  | stuck => exact ⟨rfl, fun _ h => (nomatch h), rfl⟩
  | outOfFuel => exact ⟨rfl, fun _ h => (nomatch h), rfl⟩

theorem quiet_elim {w : World H} {o : Option Val} : Quiet w ([], o.elim .stuck .ok, w) := by
  cases o <;> exact ⟨rfl, fun _ h => (nomatch h), rfl⟩

/-- **A pure body's run** performs no event, does not fail, and leaves the world as it was, at
any fuel. -/
theorem pure_run (p : Program) (w : World H) : ∀ n : Nat,
    (∀ (env : Env) (r : Rhs), r.Pure → Quiet w (evalRhs H n p w env r)) ∧
    (∀ (env : Env) (e : Expr), e.Pure → Quiet w (eval H n p w env e))
  | 0 => ⟨fun _ _ _ => ⟨rfl, fun _ h => (nomatch h), rfl⟩, fun _ _ _ => ⟨rfl, fun _ h => (nomatch h), rfl⟩⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := pure_run p w n
    have hstuck : Quiet w (stuck w) := ⟨rfl, fun _ h => (nomatch h), rfl⟩
    have hdone : ∀ v, Quiet w (done w v) := fun _ => ⟨rfl, fun _ h => (nomatch h), rfl⟩
    constructor
    · intro env r hr
      cases r <;> simp only [Rhs.Pure] at hr <;> simp only [evalRhs]
      all_goals first
        | exact quiet_elim
        | exact hdone _
        | (split <;> first | exact quiet_elim | exact hstuck | exact hdone _)
    · intro env e he
      cases e with
      | atom a => exact quiet_elim
      | letR x t v b =>
        exact quiet_andThen (ihR env v he.1) (fun _ => ihE _ b he.2)
      | letE x t v b =>
        exact quiet_andThen (ihE env v he.1) (fun _ => ihE _ b he.2)
      | ite c t e =>
        simp only [eval]
        split
        · exact ihE env t he.1
        · exact ihE env e he.2
        · exact hstuck
      | caseOption s nE x sE =>
        simp only [eval]
        split
        · exact ihE env nE he.1
        · exact ihE _ sE he.2
        · exact hstuck
      | caseResult s x oE y eE =>
        simp only [eval]
        split
        · exact ihE _ oE he.1
        · exact ihE _ eE he.2
        · exact hstuck
      | fail c => exact absurd he (by simp [Expr.Pure])
      | attempt b =>
        have h := ihE env b he
        simp only [eval]
        revert h
        rcases eval H n p w env b with ⟨t, r, w'⟩
        intro h
        cases r with
        | ok v => exact ⟨h.1, fun _ h => (nomatch h), h.2.2⟩
        | failed c => exact absurd rfl (h.2.1 c)
        | stuck => exact ⟨h.1, fun _ h => (nomatch h), h.2.2⟩
        | outOfFuel => exact ⟨h.1, fun _ h => (nomatch h), h.2.2⟩
      | handle b x hd =>
        have h := ihE env b he.1
        simp only [eval]
        revert h
        rcases eval H n p w env b with ⟨t, r, w'⟩
        intro h
        cases r with
        | failed c => exact absurd rfl (h.2.1 c)
        | ok v => exact h
        | stuck => exact h
        | outOfFuel => exact h

end Una.F1H
