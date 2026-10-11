/-
#472: **the any-host claim for one declaration**, in a document that uses `Files` elsewhere. A
declaration whose reach (`S`: the functions its body calls or references, transitively, and the
`via` functions) holds no host operation runs, from inputs whose closures reach only `S`, as on the
host without state (`eval_hostFreeIn`, `t6_release_erasure_obs_hostFreeIn`). The inputs the
theorem relates are typed: when every parameter type in `S` is closure-free (`LTy.cf`: no function
type, no type variable, and records whose fields are closure-free), a typed input holds no closure
(`LValTy.firstOrder_of_cf`), so it reaches only `S` (`Val.hostFreeIn_of_firstOrder`), and erasure
keeps that (`Val.firstOrder_anon`). A caller elsewhere in the document cannot hand such a
declaration a closure that writes a file.
-/
import UnaFormal.F1.HostFree

namespace Una.F1

/-- A type whose values hold no closure, when every record's fields are such types
(`Program.recordsCF`): no function type, and no type variable, which an instance could make one. -/
def LTy.cf : LTy → Bool
  | .int _ | .bool _ | .text _ | .unit _ | .record _ _ => true
  | .list t _ | .option t _ => t.cf
  | .result t e _ => t.cf && e.cf
  | .fn _ _ _ _ _ _ | .var _ _ | .tuple _ _ => false

/-- Every record's fields are closure-free. -/
def Program.recordsCF (p : Program) : Bool := p.records.all (·.fields.all LTy.cf)

mutual
  /-- **A first-order value reaches only `S`**, whatever `S`: it holds no closure. -/
  theorem Val.hostFreeIn_of_firstOrder {S : String → Prop} :
      ∀ {v : Val}, v.FirstOrder → Val.HostFreeIn S v
    | .list items, h => by
      simp only [Val.FirstOrder] at h; simp only [Val.HostFreeIn]
      exact Val.hostFreeAllIn_of_firstOrder h
    | .record _ fs, h | .tuple fs, h => by
      simp only [Val.FirstOrder] at h; simp only [Val.HostFreeIn]
      exact Val.hostFreeAllIn_of_firstOrder h
    | .some v, h | .ok v, h | .err v, h => by
      simp only [Val.FirstOrder] at h; simp only [Val.HostFreeIn]
      exact Val.hostFreeIn_of_firstOrder h
    | .closure _ _ _, h => by simp [Val.FirstOrder] at h
    | .int _, _ | .bool _, _ | .text _, _ | .unit, _ | .none, _ => by
      simp [Val.HostFreeIn]

  theorem Val.hostFreeAllIn_of_firstOrder {S : String → Prop} :
      ∀ {vs : List Val}, Val.FirstOrderAll vs → Val.HostFreeAllIn S vs
    | [], _ => by simp [Val.HostFreeAllIn]
    | _ :: _, h => by
      simp only [Val.FirstOrderAll] at h; simp only [Val.HostFreeAllIn]
      exact ⟨Val.hostFreeIn_of_firstOrder h.1, Val.hostFreeAllIn_of_firstOrder h.2⟩
end

section Typed

variable {p : Program}

mutual
  /-- **A value of a closure-free type is first-order.** -/
  theorem LValTy.firstOrder_of_cf (hrec : p.recordsCF = true) :
      ∀ {v : Val} {t : LTy}, LValTy p v t → t.cf = true → v.FirstOrder
    | _, _, .int, _ | _, _, .bool, _ | _, _, .text, _ | _, _, .unit, _ | _, _, .none, _ => by
      simp [Val.FirstOrder]
    | _, _, .list h, hc => by
      simp only [LTy.cf] at hc; simp only [Val.FirstOrder]
      exact LValsAll.firstOrder_of_cf hrec h hc
    | _, _, .some h, hc => by
      simp only [LTy.cf] at hc; simp only [Val.FirstOrder]
      exact LValTy.firstOrder_of_cf hrec h hc
    | _, _, .ok h, hc => by
      simp only [LTy.cf, Bool.and_eq_true] at hc; simp only [Val.FirstOrder]
      exact LValTy.firstOrder_of_cf hrec h hc.1
    | _, _, .err h, hc => by
      simp only [LTy.cf, Bool.and_eq_true] at hc; simp only [Val.FirstOrder]
      exact LValTy.firstOrder_of_cf hrec h hc.2
    | _, _, .record (rec := rec) hr hf, _ => by
      simp only [Val.FirstOrder]
      have hmem : rec ∈ p.records := by
        unfold Program.record at hr
        exact List.mem_of_find?_eq_some hr
      exact LValsTy.firstOrder_of_cf hrec hf
        (fun t ht => List.all_eq_true.mp (List.all_eq_true.mp hrec rec hmem) t ht)
    | _, _, .closure .., hc | _, _, .tuple _, hc => by simp [LTy.cf] at hc

  theorem LValsAll.firstOrder_of_cf (hrec : p.recordsCF = true) :
      ∀ {vs : List Val} {t : LTy}, LValsAll p vs t → t.cf = true → Val.FirstOrderAll vs
    | _, _, .nil, _ => by simp [Val.FirstOrderAll]
    | _, _, .cons hv hvs, hc => by
      simp only [Val.FirstOrderAll]
      exact ⟨LValTy.firstOrder_of_cf hrec hv hc, LValsAll.firstOrder_of_cf hrec hvs hc⟩

  theorem LValsTy.firstOrder_of_cf (hrec : p.recordsCF = true) :
      ∀ {vs : List Val} {ts : List LTy}, LValsTy p vs ts → (∀ t ∈ ts, t.cf = true) →
        Val.FirstOrderAll vs
    | _, _, .nil, _ => by simp [Val.FirstOrderAll]
    | _, _, .cons hv hvs, hc => by
      simp only [Val.FirstOrderAll]
      exact ⟨LValTy.firstOrder_of_cf hrec hv (hc _ (by simp)),
        LValsTy.firstOrder_of_cf hrec hvs (fun t ht => hc t (by simp [ht]))⟩
end

/-- Instantiation keeps a closure-free type closure-free: it has no type variable to replace. -/
theorem LTy.cf_lsubst (rd : String → Label) (σ : List (String × LTy)) :
    ∀ {t : LTy}, t.cf = true → (t.lsubst rd σ).cf = true
  | .int _, _ | .bool _, _ | .text _, _ | .unit _, _ | .record _ _, _ => by
    simp [LTy.lsubst, LTy.cf]
  | .list t _, h | .option t _, h => by
    simp only [LTy.cf] at h; simp only [LTy.lsubst, LTy.cf]; exact LTy.cf_lsubst rd σ h
  | .result t e _, h => by
    simp only [LTy.cf, Bool.and_eq_true] at h; simp only [LTy.lsubst, LTy.cf, Bool.and_eq_true]
    exact ⟨LTy.cf_lsubst rd σ h.1, LTy.cf_lsubst rd σ h.2⟩
  | .fn _ _ _ _ _ _, h | .var _ _, h | .tuple _ _, h => by simp [LTy.cf] at h

/-- **Typed inputs of closure-free types are first-order.** -/
theorem LEnvTy.firstOrder_of_cf (hrec : p.recordsCF = true) :
    ∀ {env : Env} {Γ : LCtx}, LEnvTy p env Γ → (∀ q ∈ Γ, q.2.cf = true) →
      ∀ q ∈ env, q.2.FirstOrder
  | _, _, .nil, _ => fun _ h => nomatch h
  | _, _, .cons hv henv, hc => fun q hq => by
    rcases List.mem_cons.mp hq with rfl | hq
    · exact LValTy.firstOrder_of_cf hrec hv (hc _ (List.mem_cons.mpr (.inl rfl)))
    · exact LEnvTy.firstOrder_of_cf hrec henv (fun q' hq' => hc q' (by simp [hq'])) q hq

end Typed

/-- An environment of first-order values reaches only `S`. -/
theorem Val.hostFreeEnvIn_of_firstOrder {S : String → Prop} :
    ∀ {env : Env}, (∀ q ∈ env, q.2.FirstOrder) → Val.HostFreeEnvIn S env
  | [], _ => by simp [Val.HostFreeEnvIn]
  | (_, v) :: env, h => by
    simp only [Val.HostFreeEnvIn]
    exact ⟨Val.hostFreeIn_of_firstOrder (h _ (List.mem_cons.mpr (.inl rfl))),
      Val.hostFreeEnvIn_of_firstOrder fun q hq => h q (by simp [hq])⟩

/-- Erasure keeps an environment first-order: what erases to a first-order environment is one. -/
theorem firstOrder_of_anonEnv {ρ : String → String} :
    ∀ {env env' : Env}, Val.anonEnv ρ env = Val.anonEnv id env' → (∀ q ∈ env, q.2.FirstOrder) →
      ∀ q ∈ env', q.2.FirstOrder
  | [], [], _, _ => fun _ h => nomatch h
  | (x, v) :: env, (y, v') :: env', h, hf => fun q hq => by
    simp only [Val.anonEnv, List.cons.injEq, Prod.mk.injEq] at h
    rcases List.mem_cons.mp hq with rfl | hq
    · rw [← Val.firstOrder_anon (ρ := id), ← h.1.2, Val.firstOrder_anon]
      exact hf _ (List.mem_cons.mpr (.inl rfl))
    · exact firstOrder_of_anonEnv h.2 (fun q hq => hf q (by simp [hq])) q hq
  | [], _ :: _, h, _ | _ :: _, [], h, _ => by simp [Val.anonEnv] at h

section T6

variable {L : Label}

/-- **The document's runs of a declaration that reaches no host operation, on any host** (#472).
`t6_release_erasure_obs_hostFreeIn`, with the premise on the inputs discharged by typing: the
declaration's parameters have closure-free types (`LTy.cf`), so the typed inputs, and the
document's inputs that erase to them, hold no closure. -/
theorem t6_release_erasure_obs_reach {S : String → Prop} {ρ : String → String}
    {P₁ P₂ : Program} (L : Label)
    (hsim : Sim ρ P₁ P₂) (hproj : ∀ t, proj P₁ L t = proj P₂ L t)
    (hwf : P₁.LWellFormed) (hb1 : P₁.B1WellFormed) (hh : P₁.HostWF) (hrec : P₁.recordsCF = true)
    (hp₂ : ∀ f fn, P₂.find f = some fn → S f → fn.body.hostFree = true ∧ ∀ g ∈ fn.body.callees, S g)
    (hv₂ : ∀ pol po, P₂.policy pol = some po → S po.via)
    {H : Host} {w₁ w₂ : World H}
    {σ : List (String × LTy)} {e₁ e₂ : Env} {Γ : LCtx} {E : List String} {pc : LExpr} {e : Expr}
    {t : LTy} {ε : Eff} (hσ : GroundSubst σ) (hE : P₁.EqArgsOK σ E) (hΓ : Γ.InPlace)
    (hΓcf : ∀ q ∈ Γ, q.2.cf = true)
    (hpc : pc.PlainFree) (h₁ : LEnvTy P₁ e₁ (Γ.lsub P₁.recordDeep σ))
    (h₂ : LEnvTy P₁ e₂ (Γ.lsub P₁.recordDeep σ)) (hrel : EnvRel P₁ L (Γ.lsub P₁.recordDeep σ) e₁ e₂)
    (he : LExprTy P₁ E Γ pc e t ε) {e' : Expr} {e₁' e₂' : Env} (hee : e.anon ρ = e'.anon id)
    (he₁ : Val.anonEnv ρ e₁ = Val.anonEnv id e₁') (he₂ : Val.anonEnv ρ e₂ = Val.anonEnv id e₂')
    (hef : e'.hostFree = true) (hec : ∀ g ∈ e'.callees, S g)
    (n : Nat) (hT₁ : Term (eval H n P₂ w₁ e₁' e').2.1) (hT₂ : Term (eval H n P₂ w₂ e₂' e').2.1) :
    proj P₂ L (eval H n P₂ w₁ e₁' e').1 = proj P₂ L (eval H n P₂ w₂ e₂' e').1 ∨
      Diverged P₂ L (eval H n P₂ w₁ e₁' e').1 (eval H n P₂ w₂ e₂' e').1 := by
  have hcf : ∀ q ∈ Γ.lsub P₁.recordDeep σ, q.2.cf = true := by
    intro q hq
    simp only [LCtx.lsub, List.mem_map] at hq
    obtain ⟨q₀, hq₀, rfl⟩ := hq
    exact LTy.cf_lsubst _ _ (hΓcf q₀ hq₀)
  have hf₁ := Val.hostFreeEnvIn_of_firstOrder (S := S)
    (firstOrder_of_anonEnv he₁ (LEnvTy.firstOrder_of_cf hrec h₁ hcf))
  have hf₂ := Val.hostFreeEnvIn_of_firstOrder (S := S)
    (firstOrder_of_anonEnv he₂ (LEnvTy.firstOrder_of_cf hrec h₂ hcf))
  exact t6_release_erasure_obs_hostFreeIn L hsim hproj hwf hb1 hh hp₂ hv₂ hσ hE hΓ hpc h₁ h₂ hrel
    he hee he₁ he₂ hef hec hf₁ hf₂ n hT₁ hT₂

end T6

end Una.F1
