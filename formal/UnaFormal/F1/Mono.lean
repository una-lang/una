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
def Agrees {α : Type} (f' f : α → Run H) : Prop := ∀ a, TermX (f a).2.1 → f' a = f a

theorem andThen_mono {f f' : Run H} {k k' : World H → Val → Run H}
    (ht : TermX (andThen f k).2.1) (hf : TermX f.2.1 → f' = f)
    (hk : ∀ v, f.2.1 = .ok v → TermX (k f.2.2 v).2.1 → k' f.2.2 v = k f.2.2 v) :
    andThen f' k' = andThen f k := by
  rw [hf (andThen_termX ht)]
  obtain ⟨tr, r, w⟩ := f
  cases r with
  | ok v =>
    simp only [andThen] at ht ⊢
    have hkt : TermX (k w v).2.1 := by
      revert ht
      rcases k w v with ⟨_, _, _⟩
      exact id
    rw [hk v rfl hkt]
  | _ => rfl

theorem applyWith_mono {run run' : World H → Env → Expr → Run H} {w : World H} {fv : Val}
    {vs : List Val} (h : ∀ w env e, TermX (run w env e).2.1 → run' w env e = run w env e)
    (ht : TermX (applyWith run w fv vs).2.1) : applyWith run' w fv vs = applyWith run w fv vs := by
  cases fv with
  | closure params body cenv =>
    simp only [applyWith] at ht ⊢
    by_cases hl : vs.length = params.length
    · simp only [hl, ite_true] at ht ⊢
      exact h _ _ _ ht
    · simp only [hl, ite_false]
  | _ => rfl

theorem walkMap_mono {app app' : World H → Val → Run H}
    (h : ∀ w v, TermX (app w v).2.1 → app' w v = app w v) :
    ∀ {w : World H} {xs : List Val}, TermX (walkMap app w xs).2.1 →
      walkMap app' w xs = walkMap app w xs
  | _, [], _ => rfl
  | w, v :: vs, ht => by
    simp only [walkMap] at ht ⊢
    refine andThen_mono ht (h w v) (fun r _ hk => ?_)
    exact andThen_mono hk (walkMap_mono h) (fun _ _ _ => rfl)

theorem walkFilter_mono {app app' : World H → Val → Run H}
    (h : ∀ w v, TermX (app w v).2.1 → app' w v = app w v) :
    ∀ {w : World H} {xs : List Val}, TermX (walkFilter app w xs).2.1 →
      walkFilter app' w xs = walkFilter app w xs
  | _, [], _ => rfl
  | w, v :: vs, ht => by
    simp only [walkFilter] at ht ⊢
    refine andThen_mono ht (h w v) (fun b _ hk => ?_)
    cases b with
    | bool keep => exact andThen_mono hk (walkFilter_mono h) (fun _ _ _ => rfl)
    | _ => rfl

theorem walkFirst_mono {app app' : World H → Val → Run H}
    (h : ∀ w v, TermX (app w v).2.1 → app' w v = app w v) :
    ∀ {w : World H} {xs : List Val}, TermX (walkFirst app w xs).2.1 →
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
    (h : ∀ w a v, TermX (app w a v).2.1 → app' w a v = app w a v) :
    ∀ {w : World H} {xs : List Val} {acc : Val}, TermX (walkFold app w acc xs).2.1 →
      walkFold app' w acc xs = walkFold app w acc xs
  | _, [], _, _ => rfl
  | w, v :: vs, acc, ht => by
    simp only [walkFold] at ht ⊢
    exact andThen_mono ht (h w acc v) (fun acc' _ hk => walkFold_mono h hk)

theorem iter_termX {r : Run H} (h : TermX (Run.iter r).2.1) : TermX r.2.1 := by
  obtain ⟨t, o, w⟩ := r
  cases o with
  | exit b _ => trivial
  | _ => exact h

theorem endLoop_termX {n : Nat} {r : Run H} (h : TermX (Run.endLoop n r).2.1) : TermX r.2.1 := by
  obtain ⟨t, o, w⟩ := r
  cases o with
  | exit b _ => trivial
  | _ => exact h

/-- An iteration's run, as `iter` reads it, agrees where the iteration ends. -/
theorem iter_mono {r r' : Run H} (h : TermX r.2.1 → r' = r) (ht : TermX (Run.iter r).2.1) :
    Run.iter r' = Run.iter r := by
  rw [h (iter_termX ht)]

theorem walkLoop_mono {b b' : World H → Val → List Val → Run H}
    (h : ∀ w v st, TermX (b w v st).2.1 → b' w v st = b w v st) :
    ∀ {w : World H} {st xs : List Val}, TermX (walkLoop b w st xs).2.1 →
      walkLoop b' w st xs = walkLoop b w st xs
  | _, _, [], _ => rfl
  | w, st, v :: vs, ht => by
    simp only [walkLoop] at ht ⊢
    refine andThen_mono ht (fun hf => iter_mono (h w v st) hf) (fun r w' hk => ?_)
    cases r with
    | tuple st' =>
      simp only at hk ⊢
      split at hk
      · rename_i hc; simp only [hc, ite_true]; exact walkLoop_mono h hk
      · rename_i hc; simp only [hc, ite_false]
    | _ => rfl

/-- `for until` with more fuel: the same run wherever it ends (ADR 0116 rule 7). -/
theorem walkUntil_mono {c c' b b' : World H → List Val → Run H}
    (hc : ∀ w st, TermX (c w st).2.1 → c' w st = c w st)
    (hb : ∀ w st, TermX (b w st).2.1 → b' w st = b w st) :
    ∀ {n m : Nat} {w : World H} {st : List Val}, n ≤ m → TermX (walkUntil c b n w st).2.1 →
      walkUntil c' b' m w st = walkUntil c b n w st
  | 0, _, _, _, _, ht => by simp [walkUntil, TermX] at ht
  | _ + 1, 0, _, _, h, _ => absurd h (by omega)
  | n + 1, m + 1, w, st, h, ht => by
    simp only [walkUntil] at ht ⊢
    refine andThen_mono ht (hc w st) (fun v _ hk => ?_)
    cases v with
    | bool cb =>
      cases cb
      · simp only at hk ⊢
        refine andThen_mono hk (fun hf => iter_mono (hb _ st) hf) (fun r _ hk' => ?_)
        cases r with
        | tuple st' =>
          simp only at hk' ⊢
          split at hk'
          · rename_i hl; simp only [hl, ite_true]; exact walkUntil_mono hc hb (by omega) hk'
          · rename_i hl; simp only [hl, ite_false]
        | _ => rfl
      · rfl
    | _ => rfl

/-- `for ever` with more fuel: the same run wherever it ends (ADR 0116 rule 10). -/
theorem walkEver_mono {stops : Nat → Bool} {b b' : World H → List Val → Run H}
    (hb : ∀ w st, TermX (b w st).2.1 → b' w st = b w st) :
    ∀ {n m i : Nat} {w : World H} {st : List Val}, n ≤ m → TermX (walkEver stops b n i w st).2.1 →
      walkEver stops b' m i w st = walkEver stops b n i w st
  | 0, _, _, _, _, _, ht => by simp [walkEver, TermX] at ht
  | _ + 1, 0, _, _, _, h, _ => absurd h (by omega)
  | n + 1, m + 1, i, w, st, h, ht => by
    simp only [walkEver] at ht ⊢
    split
    · rfl
    · rename_i hs
      simp only [hs] at ht
      refine andThen_mono ht (fun hf => iter_mono (hb w st) hf) (fun r _ hk => ?_)
      cases r with
      | tuple st' =>
        simp only at hk ⊢
        split at hk
        · rename_i hl; simp only [hl, ite_true]; exact walkEver_mono hb (by omega) hk
        · rename_i hl; simp only [hl, ite_false]
      | _ => rfl

/-- `on stop` agrees where it ends. -/
theorem finishEver_mono {k k' : World H → List Val → Run H} {st₀ : List Val} {r r' : Run H}
    (hk : ∀ w st, TermX (k w st).2.1 → k' w st = k w st) (hr : TermX r.2.1 → r' = r)
    (ht : TermX (finishEver k st₀ r).2.1) : finishEver k' st₀ r' = finishEver k st₀ r := by
  have hT : TermX r.2.1 := by
    obtain ⟨t, o, w⟩ := r
    cases o <;> simp_all [finishEver, TermX]
  rw [hr hT]
  obtain ⟨t, o, w⟩ := r
  cases o with
  | ok v =>
    cases v with
    | tuple st =>
      simp only [finishEver] at ht ⊢
      have : TermX (k w st).2.1 := by
        revert ht; rcases k w st with ⟨_, _, _⟩; exact id
      rw [hk w st this]
    | _ => rfl
  | failed c =>
    simp only [finishEver] at ht ⊢
    have : TermX (k w st₀).2.1 := by
      revert ht; rcases k w st₀ with ⟨_, r', _⟩; cases r' <;> simp [TermX]
    rw [hk w st₀ this]
  | raised kd c ps =>
    simp only [finishEver] at ht ⊢
    have : TermX (k w st₀).2.1 := by
      revert ht; rcases k w st₀ with ⟨_, r', _⟩; cases r' <;> simp [TermX]
    rw [hk w st₀ this]
  | _ => rfl

variable {p : Program}

theorem eval_mono : ∀ n m : Nat, n ≤ m →
    (∀ w env r, TermX (evalRhs H n p w env r).2.1 → evalRhs H m p w env r = evalRhs H n p w env r) ∧
    (∀ w env e, TermX (eval H n p w env e).2.1 → eval H m p w env e = eval H n p w env e)
  | 0, _, _ => ⟨fun _ _ _ h => by simp [evalRhs, TermX] at h,
      fun _ _ _ h => by simp [eval, TermX] at h⟩
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
      | invoke f args =>
        simp only [evalRhs] at ht ⊢
        cases hf : p.find f <;> cases ha : evalAtoms env args <;> simp only [hf, ha] at ht ⊢
        rename_i fn vs
        by_cases hl : vs.length = fn.params.length
        · simp only [hl, ite_true] at ht ⊢
          simp only [Run.ghost] at ht ⊢
          rw [ihE _ _ _ ht]
        · simp only [hl, ite_false]
      | invokeOp o args =>
        simp only [evalRhs] at ht ⊢
        cases hf : p.find (p.handlerFor o (lookup env (markerName o))) <;>
          cases ha : evalAtoms env args <;> simp only [hf, ha] at ht ⊢
        rename_i fn vs
        by_cases hl : vs.length = fn.params.length
        · simp only [hl, ite_true] at ht ⊢
          simp only [Run.ghost] at ht ⊢
          rw [ihE _ _ _ ht]
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
          have hin : TermX (eval H n p w (bindParams fn.params [v] []) fn.body).2.1 := by
            revert ht
            rcases eval H n p w (bindParams fn.params [v] []) fn.body with ⟨t, r, w'⟩
            cases r <;> simp [TermX]
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
      | raise _ _ _ => rfl
      | unreachable => rfl
      | withH o f b =>
        simp only [eval] at ht ⊢
        exact ihE _ _ _ ht
      | caseSum sc i xs t e =>
        simp only [eval] at ht ⊢
        cases hsc : evalAtom env sc with
        | none => simp [hsc]
        | some v =>
          cases v with
          | variant k j fs =>
            simp only [hsc] at ht ⊢
            by_cases hi : j = i
            · by_cases hl : fs.length = xs.length
              · simp only [if_pos hi, if_pos hl] at ht ⊢
                exact ihE _ _ _ ht
              · simp [if_pos hi, if_neg hl]
            · simp only [if_neg hi] at ht ⊢
              exact ihE _ _ _ ht
          | _ => simp [hsc]
      | attemptK kind body =>
        simp only [eval] at ht ⊢
        have hb : TermX (eval H n p w env body).2.1 := by
          revert ht
          rcases eval H n p w env body with ⟨tr, r, w'⟩
          cases r <;> dsimp only <;> (try split) <;> simp [TermX]
        rw [ihE w env body hb]
      | handleK kind body x handler =>
        simp only [eval] at ht ⊢
        have hb : TermX (eval H n p w env body).2.1 := by
          revert ht
          rcases eval H n p w env body with ⟨tr, r, w'⟩
          cases r <;> dsimp only <;> (try split) <;> simp [TermX]
        rw [ihE w env body hb]
        revert ht
        rcases eval H n p w env body with ⟨tr, r, w'⟩
        cases r with
        | raised k c ps =>
          intro ht
          dsimp only at ht ⊢
          by_cases hk : k = kind
          · simp only [if_pos hk] at ht ⊢
            have hh : TermX (eval H n p w' ((x, .variant k c ps) :: env) handler).2.1 := by
              revert ht
              rcases eval H n p w' ((x, .variant k c ps) :: env) handler with ⟨_, _, _⟩
              exact id
            rw [ihE _ _ handler hh]
          · simp only [if_neg hk]
        | _ => intro _; rfl
      | attempt body =>
        simp only [eval] at ht ⊢
        have hb : TermX (eval H n p w env body).2.1 := by
          revert ht
          rcases eval H n p w env body with ⟨tr, r, w'⟩
          cases r <;> simp [TermX]
        rw [ihE w env body hb]
      | handle body x handler =>
        simp only [eval] at ht ⊢
        have hb : TermX (eval H n p w env body).2.1 := by
          revert ht
          rcases eval H n p w env body with ⟨tr, r, w'⟩
          cases r <;> simp [TermX]
        rw [ihE w env body hb]
        revert ht
        rcases eval H n p w env body with ⟨tr, r, w'⟩
        cases r with
        | failed c =>
          intro ht
          simp only [] at ht ⊢
          have hh : TermX (eval H n p w' ((x, .int c) :: env) handler).2.1 := by
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
          · rename_i hc; simp only [hc, ite_true]
            rw [walkLoop_mono (fun w v st hv => ihE _ _ _ hv) (endLoop_termX ht)]
          · rename_i hc; simp only [hc, ite_false]
        · rfl
      | exit stop state => rfl
      | loopUntil cond state init body =>
        simp only [eval] at ht ⊢
        split at ht
        · split at ht
          · rename_i hc; simp only [hc, ite_true]
            rw [walkUntil_mono (fun w st hv => ihE _ _ _ hv) (fun w st hv => ihE _ _ _ hv) (by omega)
              (endLoop_termX ht)]
          · rename_i hc; simp only [hc, ite_false]
        · rfl
      | loopEver site state init body os o =>
        simp only [eval] at ht ⊢
        split at ht
        · split at ht
          · rename_i hc; simp only [hc, ite_true]
            refine finishEver_mono (fun w st hk => ?_) (fun hr => ?_) ht
            · split at hk
              · rename_i hl; simp only [hl, ite_true]; exact ihE _ _ _ hk
              · rename_i hl; simp only [hl, ite_false]
            · rw [walkEver_mono (fun w st hv => ihE _ _ _ hv) (by omega) (endLoop_termX hr)]
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
      OutRel p L (V p L t) (Vis p L ε.failLabel) (eval H n₁ p w₁ e₁ e).2.1 (eval H n₂ p w₂ e₂ e).2.1 ∧
      WorldRel p L (eval H n₁ p w₁ e₁ e).2.2 (eval H n₂ p w₂ e₂ e).2.2 := by
  have m₁ := (eval_mono (H := H) (p := p) n₁ (max n₁ n₂) (Nat.le_max_left _ _)).2 w₁ e₁ e
    (termX_of_term ht₁)
  have m₂ := (eval_mono (H := H) (p := p) n₂ (max n₁ n₂) (Nat.le_max_right _ _)).2 w₂ e₂ e
    (termX_of_term ht₂)
  have h := t6_host_ground hwf hpol hh hlaw hΓ hpc hw h₁ h₂ hrel he (max n₁ n₂)
    (by rw [m₁]; exact ht₁) (by rw [m₂]; exact ht₂)
  rw [m₁, m₂] at h
  exact h

end Una.F1
