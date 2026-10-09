/-
The restrictions of option B1 (declassification, `docs/plan/formal-track-m4.md` §3) on a
program's policies, and what a pure `via` function's run is: no event and no failure.
-/
import UnaFormal.F1.LTyping

namespace Una.F1

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
  via : ∀ po ∈ p.policies, ∃ fn, p.find po.via = some fn ∧ fn.op = false ∧ fn.get = false ∧
    fn.fails = false ∧ fn.body.Pure

/-- Without policies, the restrictions hold. -/
theorem Program.B1WellFormed.of_nil {p : Program} (hnil : p.policies = []) : p.B1WellFormed where
  src := by rw [hnil]; intro _ h; nomatch h
  dst := by rw [hnil]; intro _ h; nomatch h
  via := by rw [hnil]; intro _ h; nomatch h

/-- A run with no event that does not fail. -/
def Quiet (r : Trace × Res) : Prop := r.1 = [] ∧ ∀ c, r.2 ≠ .failed c

theorem quiet_andThen {first : Trace × Res} {rest : Val → Trace × Res} (h₁ : Quiet first)
    (h₂ : ∀ v, Quiet (rest v)) : Quiet (andThen first rest) := by
  obtain ⟨t, r⟩ := first
  obtain ⟨ht, hr⟩ := h₁
  simp only at ht hr
  subst ht
  cases r with
  | ok v =>
    simp only [andThen]
    have := h₂ v
    rcases hq : rest v with ⟨t', r'⟩
    rw [hq] at this
    exact ⟨by simpa using this.1, this.2⟩
  | failed c => exact absurd rfl (hr c)
  | stuck => exact ⟨rfl, fun _ h => nomatch h⟩
  | outOfFuel => exact ⟨rfl, fun _ h => nomatch h⟩

theorem quiet_elim {o : Option Val} : Quiet ([], o.elim .stuck .ok) := by
  cases o <;> exact ⟨rfl, fun _ h => nomatch h⟩

/-- **A pure body's run** performs no event and does not fail, at any fuel. -/
theorem pure_run (p : Program) (w : World) : ∀ n : Nat,
    (∀ (env : Env) (r : Rhs), r.Pure → Quiet (evalRhs n p w env r)) ∧
    (∀ (env : Env) (e : Expr), e.Pure → Quiet (eval n p w env e))
  | 0 => ⟨fun _ _ _ => ⟨rfl, fun _ h => nomatch h⟩, fun _ _ _ => ⟨rfl, fun _ h => nomatch h⟩⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := pure_run p w n
    have hstuck : Quiet stuck := ⟨rfl, fun _ h => nomatch h⟩
    have hdone : ∀ v, Quiet (done v) := fun _ => ⟨rfl, fun _ h => nomatch h⟩
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
        rcases eval n p w env b with ⟨t, r⟩
        intro h
        cases r with
        | ok v => exact ⟨h.1, fun _ h => nomatch h⟩
        | failed c => exact absurd rfl (h.2 c)
        | stuck => exact ⟨h.1, fun _ h => nomatch h⟩
        | outOfFuel => exact ⟨h.1, fun _ h => nomatch h⟩
      | handle b x hd =>
        have h := ihE env b he.1
        simp only [eval]
        revert h
        rcases eval n p w env b with ⟨t, r⟩
        intro h
        cases r with
        | failed c => exact absurd rfl (h.2 c)
        | ok v => exact h
        | stuck => exact h
        | outOfFuel => exact h

end Una.F1
