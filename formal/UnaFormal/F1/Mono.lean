/-
Fuel monotonicity: a run that ends, with a value or a failure, at some fuel ends the same way,
with the same events and the same world, at any larger fuel. So `t6_host`, which compares two
runs at one fuel, compares any two runs that end: both end at the larger of their fuels
(`t6_ended`).
-/
import UnaFormal.F1.T6

namespace Una.F1

variable {H : Host}

/-- A run of `f'` agrees with one of `f` wherever `f`'s ends. -/
def Agrees {α : Type} (f' f : α → Run H) : Prop := ∀ a, Term (f a).2.1 → f' a = f a

theorem andThen_mono {f f' : Run H} {k k' : World H → Val → Run H}
    (ht : Term (andThen f k).2.1) (hf : Term f.2.1 → f' = f)
    (hk : ∀ v, f.2.1 = .ok v → Term (k f.2.2 v).2.1 → k' f.2.2 v = k f.2.2 v) :
    andThen f' k' = andThen f k := by
  rw [hf (andThen_term ht)]
  obtain ⟨tr, r, w⟩ := f
  cases r with
  | ok v =>
    simp only [andThen] at ht ⊢
    have hkt : Term (k w v).2.1 := by
      revert ht
      rcases k w v with ⟨_, _, _⟩
      exact id
    rw [hk v rfl hkt]
  | failed c => rfl
  | stuck => rfl
  | outOfFuel | aborted => rfl

theorem applyWith_mono {run run' : World H → Env → Expr → Run H} {w : World H} {fv : Val}
    {vs : List Val} (h : ∀ w env e, Term (run w env e).2.1 → run' w env e = run w env e)
    (ht : Term (applyWith run w fv vs).2.1) : applyWith run' w fv vs = applyWith run w fv vs := by
  cases fv with
  | closure params body cenv =>
    simp only [applyWith] at ht ⊢
    by_cases hl : vs.length = params.length
    · simp only [hl, ite_true] at ht ⊢
      exact h _ _ _ ht
    · simp only [hl, ite_false]
  | _ => rfl

theorem walkMap_mono {app app' : World H → Val → Run H}
    (h : ∀ w v, Term (app w v).2.1 → app' w v = app w v) :
    ∀ {w : World H} {xs : List Val}, Term (walkMap app w xs).2.1 →
      walkMap app' w xs = walkMap app w xs
  | _, [], _ => rfl
  | w, v :: vs, ht => by
    simp only [walkMap] at ht ⊢
    refine andThen_mono ht (h w v) (fun r _ hk => ?_)
    exact andThen_mono hk (walkMap_mono h) (fun _ _ _ => rfl)

theorem walkFilter_mono {app app' : World H → Val → Run H}
    (h : ∀ w v, Term (app w v).2.1 → app' w v = app w v) :
    ∀ {w : World H} {xs : List Val}, Term (walkFilter app w xs).2.1 →
      walkFilter app' w xs = walkFilter app w xs
  | _, [], _ => rfl
  | w, v :: vs, ht => by
    simp only [walkFilter] at ht ⊢
    refine andThen_mono ht (h w v) (fun b _ hk => ?_)
    cases b with
    | bool keep => exact andThen_mono hk (walkFilter_mono h) (fun _ _ _ => rfl)
    | _ => rfl

theorem walkFirst_mono {app app' : World H → Val → Run H}
    (h : ∀ w v, Term (app w v).2.1 → app' w v = app w v) :
    ∀ {w : World H} {xs : List Val}, Term (walkFirst app w xs).2.1 →
      walkFirst app' w xs = walkFirst app w xs
  | _, [], _ => rfl
  | w, v :: vs, ht => by
    simp only [walkFirst] at ht ⊢
    refine andThen_mono ht (h w v) (fun b _ hk => ?_)
    cases b with
    | bool keep =>
      cases keep
      · exact walkFirst_mono h hk
      · rfl
    | _ => rfl

theorem walkFold_mono {app app' : World H → Val → Val → Run H}
    (h : ∀ w a v, Term (app w a v).2.1 → app' w a v = app w a v) :
    ∀ {w : World H} {xs : List Val} {acc : Val}, Term (walkFold app w acc xs).2.1 →
      walkFold app' w acc xs = walkFold app w acc xs
  | _, [], _, _ => rfl
  | w, v :: vs, acc, ht => by
    simp only [walkFold] at ht ⊢
    exact andThen_mono ht (h w acc v) (fun acc' _ hk => walkFold_mono h hk)

theorem walkLoop_mono {b b' : World H → Val → List Val → Run H}
    (h : ∀ w v st, Term (b w v st).2.1 → b' w v st = b w v st) :
    ∀ {w : World H} {st xs : List Val}, Term (walkLoop b w st xs).2.1 →
      walkLoop b' w st xs = walkLoop b w st xs
  | _, _, [], _ => rfl
  | w, st, v :: vs, ht => by
    simp only [walkLoop] at ht ⊢
    refine andThen_mono ht (h w v st) (fun r w' hk => ?_)
    cases r with
    | tuple st' =>
      simp only at hk ⊢
      split at hk
      · rename_i hc; simp only [hc, ite_true]; exact walkLoop_mono h hk
      · rename_i hc; simp only [hc, ite_false]
    | _ => rfl

variable {p : Program}

theorem eval_mono : ∀ n m : Nat, n ≤ m →
    (∀ w env r, Term (evalRhs H n p w env r).2.1 → evalRhs H m p w env r = evalRhs H n p w env r) ∧
    (∀ w env e, Term (eval H n p w env e).2.1 → eval H m p w env e = eval H n p w env e)
  | 0, _, _ => ⟨fun _ _ _ h => by simp [evalRhs, Term] at h,
      fun _ _ _ h => by simp [eval, Term] at h⟩
  | n + 1, 0, h => absurd h (by omega)
  | n + 1, m + 1, h => by
    obtain ⟨ihR, ihE⟩ := eval_mono n m (by omega)
    constructor
    · intro w env r ht
      cases r with
      | call f targs args =>
        simp only [evalRhs] at ht ⊢
        cases hf : p.find f <;> cases ha : evalAtoms env args <;> simp only [hf, ha] at ht ⊢
        rename_i fn vs
        by_cases hl : vs.length = fn.params.length
        · simp only [hl, ite_true] at ht ⊢
          exact ihE _ _ _ ht
        · simp only [hl, ite_false]
      | apply f args =>
        simp only [evalRhs] at ht ⊢
        split at ht
        · exact applyWith_mono ihE ht
        · rfl
      | declassify pol a =>
        simp only [evalRhs] at ht ⊢
        cases hpo : p.policy pol <;> cases ha : evalAtom env a <;> simp only [hpo, ha] at ht ⊢
        rename_i po v
        cases hfn : p.find po.via <;> simp only [hfn] at ht ⊢
        rename_i fn
        by_cases hl : 1 = fn.params.length
        · simp only [hl, ite_true] at ht ⊢
          have hin : Term (eval H n p w (bindParams fn.params [v] []) fn.body).2.1 := by
            revert ht
            rcases eval H n p w (bindParams fn.params [v] []) fn.body with ⟨t, r, w'⟩
            cases r <;> simp [Term]
          rw [ihE _ _ _ hin]
        · simp only [hl, ite_false]
      | map xs f =>
        simp only [evalRhs] at ht ⊢
        split at ht
        · exact walkMap_mono (fun w v hv => applyWith_mono ihE hv) ht
        · rfl
      | filter xs keep =>
        simp only [evalRhs] at ht ⊢
        split at ht
        · exact walkFilter_mono (fun w v hv => applyWith_mono ihE hv) ht
        · rfl
      | firstWhere xs keep =>
        simp only [evalRhs] at ht ⊢
        split at ht
        · exact walkFirst_mono (fun w v hv => applyWith_mono ihE hv) ht
        · rfl
      | fold xs init f =>
        simp only [evalRhs] at ht ⊢
        split at ht
        · exact walkFold_mono (fun w a v hv => applyWith_mono ihE hv) ht
        · rfl
      | _ => rfl
    · intro w env e ht
      cases e with
      | atom a => rfl
      | letR x ann value body =>
        simp only [eval] at ht ⊢
        exact andThen_mono ht (ihR w env value) (fun v _ hk => ihE _ _ body hk)
      | letE x ann value body =>
        simp only [eval] at ht ⊢
        exact andThen_mono ht (ihE w env value) (fun v _ hk => ihE _ _ body hk)
      | ite cond thenE elseE =>
        simp only [eval] at ht ⊢
        split at ht <;> first | exact ihE _ _ _ ht | rfl
      | caseOption scrutinee noneE x someE =>
        simp only [eval] at ht ⊢
        split at ht <;> first | exact ihE _ _ _ ht | rfl
      | caseResult scrutinee x okE y errE =>
        simp only [eval] at ht ⊢
        split at ht <;> first | exact ihE _ _ _ ht | rfl
      | fail code => rfl
      | attempt body =>
        simp only [eval] at ht ⊢
        have hb : Term (eval H n p w env body).2.1 := by
          revert ht
          rcases eval H n p w env body with ⟨tr, r, w'⟩
          cases r <;> simp [Term]
        rw [ihE w env body hb]
      | handle body x handler =>
        simp only [eval] at ht ⊢
        have hb : Term (eval H n p w env body).2.1 := by
          revert ht
          rcases eval H n p w env body with ⟨tr, r, w'⟩
          cases r <;> simp [Term]
        rw [ihE w env body hb]
        revert ht
        rcases eval H n p w env body with ⟨tr, r, w'⟩
        cases r with
        | failed c =>
          intro ht
          simp only [] at ht ⊢
          have hh : Term (eval H n p w' ((x, .int c) :: env) handler).2.1 := by
            revert ht
            rcases eval H n p w' ((x, .int c) :: env) handler with ⟨_, _, _⟩
            exact id
          rw [ihE _ _ handler hh]
        | _ => intro _; rfl
      | letTuple binders value body =>
        simp only [eval] at ht ⊢
        split at ht
        · split at ht
          · rename_i hc; simp only [hc, ite_true]; exact ihE _ _ _ ht
          · rename_i hc; simp only [hc, ite_false]
        · rfl
      | loop source x xty state init body =>
        simp only [eval] at ht ⊢
        split at ht
        · split at ht
          · rename_i hc; simp only [hc, ite_true]; exact walkLoop_mono (fun w v st hv => ihE _ _ _ hv) ht
          · rename_i hc; simp only [hc, ite_false]
        · rfl

variable {L : Label}

/-- **T6 for runs that end, at any fuels.** Two runs of a body the labeled typing accepts, from
worlds and in environments the observer at `L` cannot tell apart, that both end, whatever fuel
each needed, show that observer the same events and related outcomes, and leave worlds it cannot
tell apart. -/
theorem t6_ended (hwf : p.LWellFormed) (hpol : p.policies = []) (hh : p.HostWF)
    (hlaw : H.Lawful) {w₁ w₂ : World H}
    {e₁ e₂ : Env} {Γ : LCtx} {pc : LExpr} {e : Expr} {t : LTy} {ε : Eff}
    (hΓ : Γ.InPlace) (hpc : pc.PlainFree) (hw : WorldRel p L w₁ w₂) (h₁ : LEnvTy p e₁ Γ)
    (h₂ : LEnvTy p e₂ Γ) (hrel : EnvRel p L Γ e₁ e₂) (he : LExprTy p [] Γ pc e t ε)
    {n₁ n₂ : Nat} (ht₁ : Term (eval H n₁ p w₁ e₁ e).2.1) (ht₂ : Term (eval H n₂ p w₂ e₂ e).2.1) :
    proj p L (eval H n₁ p w₁ e₁ e).1 = proj p L (eval H n₂ p w₂ e₂ e).1 ∧
      OutRel (V p L t) (Vis p L ε.failLabel) (eval H n₁ p w₁ e₁ e).2.1 (eval H n₂ p w₂ e₂ e).2.1 ∧
      WorldRel p L (eval H n₁ p w₁ e₁ e).2.2 (eval H n₂ p w₂ e₂ e).2.2 := by
  have m₁ := (eval_mono (H := H) (p := p) n₁ (max n₁ n₂) (Nat.le_max_left _ _)).2 w₁ e₁ e ht₁
  have m₂ := (eval_mono (H := H) (p := p) n₂ (max n₁ n₂) (Nat.le_max_right _ _)).2 w₂ e₂ e ht₂
  have h := t6_host_ground hwf hpol hh hlaw hΓ hpc hw h₁ h₂ hrel he (max n₁ n₂)
    (by rw [m₁]; exact ht₁) (by rw [m₂]; exact ht₂)
  rw [m₁, m₂] at h
  exact h

end Una.F1
