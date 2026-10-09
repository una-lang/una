/-
Fuel monotonicity: a run that ends, with a value or a failure, at some fuel ends the same way,
with the same events, at any larger fuel. So `t6`, which compares two runs at one fuel, compares
any two runs that end: both end at the larger of their fuels (`t6_ended`).
-/
import UnaFormal.F1.T6

namespace Una.F1

/-- A run of `f'` agrees with one of `f` wherever `f`'s ends. -/
def Agrees {α : Type} (f' f : α → Trace × Res) : Prop := ∀ a, Term (f a).2 → f' a = f a

theorem andThen_mono {f f' : Trace × Res} {k k' : Val → Trace × Res}
    (ht : Term (andThen f k).2) (hf : Term f.2 → f' = f)
    (hk : ∀ v, f.2 = .ok v → Term (k v).2 → k' v = k v) : andThen f' k' = andThen f k := by
  rw [hf (andThen_term ht)]
  obtain ⟨tr, r⟩ := f
  cases r with
  | ok v =>
    simp only [andThen] at ht ⊢
    have hkt : Term (k v).2 := by
      revert ht
      cases k v
      exact id
    rw [hk v rfl hkt]
  | failed c => rfl
  | stuck => rfl
  | outOfFuel => rfl

theorem applyWith_mono {run run' : Env → Expr → Trace × Res} {fv : Val} {vs : List Val}
    (h : ∀ env e, Term (run env e).2 → run' env e = run env e)
    (ht : Term (applyWith run fv vs).2) : applyWith run' fv vs = applyWith run fv vs := by
  cases fv with
  | closure params body cenv =>
    simp only [applyWith] at ht ⊢
    by_cases hl : vs.length = params.length
    · simp only [hl, if_true] at ht ⊢
      exact h _ _ ht
    · simp only [hl, if_false]
  | _ => rfl

theorem walkMap_mono {app app' : Val → Trace × Res} (h : Agrees app' app) :
    ∀ {xs : List Val}, Term (walkMap app xs).2 → walkMap app' xs = walkMap app xs
  | [], _ => rfl
  | v :: vs, ht => by
    simp only [walkMap] at ht ⊢
    refine andThen_mono ht (h v) (fun r _ hk => ?_)
    refine andThen_mono hk (walkMap_mono h) (fun _ _ _ => rfl)

theorem walkFilter_mono {app app' : Val → Trace × Res} (h : Agrees app' app) :
    ∀ {xs : List Val}, Term (walkFilter app xs).2 → walkFilter app' xs = walkFilter app xs
  | [], _ => rfl
  | v :: vs, ht => by
    simp only [walkFilter] at ht ⊢
    refine andThen_mono ht (h v) (fun b _ hk => ?_)
    cases b with
    | bool keep => exact andThen_mono hk (walkFilter_mono h) (fun _ _ _ => rfl)
    | _ => rfl

theorem walkFirst_mono {app app' : Val → Trace × Res} (h : Agrees app' app) :
    ∀ {xs : List Val}, Term (walkFirst app xs).2 → walkFirst app' xs = walkFirst app xs
  | [], _ => rfl
  | v :: vs, ht => by
    simp only [walkFirst] at ht ⊢
    refine andThen_mono ht (h v) (fun b _ hk => ?_)
    cases b with
    | bool keep =>
      cases keep
      · exact walkFirst_mono h hk
      · rfl
    | _ => rfl

theorem walkFold_mono {app app' : Val → Val → Trace × Res}
    (h : ∀ a v, Term (app a v).2 → app' a v = app a v) :
    ∀ {xs : List Val} {acc : Val}, Term (walkFold app acc xs).2 →
      walkFold app' acc xs = walkFold app acc xs
  | [], _, _ => rfl
  | v :: vs, acc, ht => by
    simp only [walkFold] at ht ⊢
    exact andThen_mono ht (h acc v) (fun acc' _ hk => walkFold_mono h hk)

variable {p : Program} {w : World}

theorem eval_mono : ∀ n m : Nat, n ≤ m →
    (∀ env r, Term (evalRhs n p w env r).2 → evalRhs m p w env r = evalRhs n p w env r) ∧
    (∀ env e, Term (eval n p w env e).2 → eval m p w env e = eval n p w env e)
  | 0, _, _ => ⟨fun _ _ h => by simp [evalRhs, Term] at h, fun _ _ h => by simp [eval, Term] at h⟩
  | n + 1, 0, h => absurd h (by omega)
  | n + 1, m + 1, h => by
    obtain ⟨ihR, ihE⟩ := eval_mono n m (by omega)
    have hrun : ∀ env e, Term (eval n p w env e).2 → eval m p w env e = eval n p w env e := ihE
    constructor
    · intro env r ht
      cases r with
      | call f targs args =>
        simp only [evalRhs] at ht ⊢
        cases hf : p.find f <;> cases ha : evalAtoms env args <;> simp only [hf, ha] at ht ⊢
        rename_i fn vs
        by_cases hl : vs.length = fn.params.length
        · simp only [hl, if_true] at ht ⊢
          exact ihE _ _ ht
        · simp only [hl, if_false]
      | apply f args =>
        simp only [evalRhs] at ht ⊢
        split at ht
        · exact applyWith_mono hrun ht
        · rfl
      | declassify pol a =>
        simp only [evalRhs] at ht ⊢
        cases hpo : p.policy pol <;> cases ha : evalAtom env a <;> simp only [hpo, ha] at ht ⊢
        rename_i po v
        cases hfn : p.find po.via <;> simp only [hfn] at ht ⊢
        rename_i fn
        by_cases hl : 1 = fn.params.length
        · simp only [hl, if_true] at ht ⊢
          exact ihE _ _ ht
        · simp only [hl, if_false]
      | map xs f =>
        simp only [evalRhs] at ht ⊢
        split at ht
        · exact walkMap_mono (fun v hv => applyWith_mono hrun hv) ht
        · rfl
      | filter xs keep =>
        simp only [evalRhs] at ht ⊢
        split at ht
        · exact walkFilter_mono (fun v hv => applyWith_mono hrun hv) ht
        · rfl
      | firstWhere xs keep =>
        simp only [evalRhs] at ht ⊢
        split at ht
        · exact walkFirst_mono (fun v hv => applyWith_mono hrun hv) ht
        · rfl
      | fold xs init f =>
        simp only [evalRhs] at ht ⊢
        split at ht
        · exact walkFold_mono (fun a v hv => applyWith_mono hrun hv) ht
        · rfl
      | _ => rfl
    · intro env e ht
      cases e with
      | atom a => rfl
      | letR x ann value body =>
        simp only [eval] at ht ⊢
        exact andThen_mono ht (ihR env value) (fun v _ hk => ihE _ body hk)
      | letE x ann value body =>
        simp only [eval] at ht ⊢
        exact andThen_mono ht (ihE env value) (fun v _ hk => ihE _ body hk)
      | ite cond thenE elseE =>
        simp only [eval] at ht ⊢
        split at ht <;> first | exact ihE _ _ ht | rfl
      | caseOption scrutinee noneE x someE =>
        simp only [eval] at ht ⊢
        split at ht <;> first | exact ihE _ _ ht | rfl
      | caseResult scrutinee x okE y errE =>
        simp only [eval] at ht ⊢
        split at ht <;> first | exact ihE _ _ ht | rfl
      | fail code => rfl
      | attempt body =>
        simp only [eval] at ht ⊢
        have hb : Term (eval n p w env body).2 := by
          revert ht
          cases eval n p w env body with
          | mk tr r => cases r <;> simp [Term]
        rw [ihE env body hb]
      | handle body x handler =>
        simp only [eval] at ht ⊢
        have hb : Term (eval n p w env body).2 := by
          revert ht
          cases eval n p w env body with
          | mk tr r => cases r <;> simp [Term]
        rw [ihE env body hb]
        revert ht
        cases eval n p w env body with
        | mk tr r =>
          cases r with
          | failed c =>
            intro ht
            simp only [] at ht ⊢
            have hh : Term (eval n p w ((x, .int c) :: env) handler).2 := by
              revert ht
              cases eval n p w ((x, .int c) :: env) handler
              exact id
            rw [ihE _ handler hh]
          | _ => intro _; rfl

end Una.F1

namespace Una.F1

variable {p : Program} {L : Label}

/-- **T6 for runs that end, at any fuels.** Two runs of a body the labeled typing accepts, in
environments the observer at `L` cannot tell apart, that both end, whatever fuel each needed,
show that observer the same events and related outcomes. -/
theorem t6_ended (hwf : p.LWellFormed) (hgf : p.GenericFree)
    (hget : p.le (.ofLabel p.acceptsGet) (.ofLabel p.getResult)) (w : World)
    {e₁ e₂ : Env} {Γ : LCtx} {E : List String} {pc : LExpr} {e : Expr} {t : LTy} {ε : Eff}
    (h₁ : LEnvTy p e₁ Γ) (h₂ : LEnvTy p e₂ Γ) (hrel : EnvRel p L Γ e₁ e₂)
    (he : LExprTy p E Γ pc e t ε) {n₁ n₂ : Nat}
    (ht₁ : Term (eval n₁ p w e₁ e).2) (ht₂ : Term (eval n₂ p w e₂ e).2) :
    proj p L (eval n₁ p w e₁ e).1 = proj p L (eval n₂ p w e₂ e).1 ∧
      OutRel (V p L t) (Vis p L ε.failLabel) (eval n₁ p w e₁ e).2 (eval n₂ p w e₂ e).2 := by
  have m₁ := (eval_mono (p := p) (w := w) n₁ (max n₁ n₂) (Nat.le_max_left _ _)).2 e₁ e ht₁
  have m₂ := (eval_mono (p := p) (w := w) n₂ (max n₁ n₂) (Nat.le_max_right _ _)).2 e₂ e ht₂
  have h := t6 hwf hgf hget w h₁ h₂ hrel he (max n₁ n₂) (by rw [m₁]; exact ht₁) (by rw [m₂]; exact ht₂)
  rw [m₁, m₂] at h
  exact h

end Una.F1
