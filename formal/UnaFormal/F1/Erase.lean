/-
The labeled typing refines the structural one: erasing labels from a labeled derivation gives a
structural one, and a program the labeled typing accepts is well formed. So T1 holds for the
labeled typing of F1, the judgment of `spec/10`, too.
-/
import UnaFormal.F1.LTyping
import UnaFormal.F1.T1

namespace Una.F1

variable {p : Program} {E : List String}

mutual
  /-- A flow keeps the structure. -/
  theorem Flow.erase {top : Label} : ∀ {t₁ t₂ : LTy}, Flow top t₁ t₂ → t₁.erase = t₂.erase
    | _, _, .int _ | _, _, .bool _ | _, _, .text _ | _, _, .unit | _, _, .record _
    | _, _, .var _ => rfl
    | _, _, .list h _ | _, _, .option h _ | _, _, .result h _ => by
      simp [LTy.erase, Flow.erase h]
    | _, _, .fn hps hr _ _ _ _ => by
      simp only [LTy.erase, LTy.erases_eq, Flows.erase hps, Flow.erase hr]

  theorem Flows.erase {top : Label} :
      ∀ {ts₁ ts₂ : List LTy}, Flows top ts₁ ts₂ → ts₂.map LTy.erase = ts₁.map LTy.erase
    | _, _, .nil => rfl
    | _, _, .cons h hs => by simp [Flow.erase h, Flows.erase hs]
end

theorem lctxLookup_erase {x : String} {t : LTy} :
    ∀ {Γ : LCtx}, lctxLookup Γ x = some t → ctxLookup Γ.erase x = some t.erase
  | [], h => by simp [lctxLookup] at h
  | (y, u) :: Γ, h => by
    simp only [lctxLookup] at h
    simp only [LCtx.erase, List.map_cons, ctxLookup]
    split
    · rw [ite_eq_left (by assumption)] at h
      cases h
      rfl
    · rw [ite_eq_right (by assumption)] at h
      exact lctxLookup_erase h

theorem latom_erase {Γ : LCtx} {a : Atom} {t : LTy} (h : LAtomTy Γ a t) :
    AtomTy Γ.erase a t.erase := by
  cases h with
  | var hx => exact .var (lctxLookup_erase hx)
  | int => exact .int
  | bool => exact .bool
  | text => exact .text
  | unit => exact .unit

theorem latoms_erase {Γ : LCtx} {as : List Atom} {ts : List LTy} (h : LAtomsTy Γ as ts) :
    AtomsTy Γ.erase as (ts.map LTy.erase) := by
  induction h with
  | nil => exact .nil
  | cons ha _ ih => exact .cons (latom_erase ha) ih

theorem latoms_length {Γ : LCtx} {as : List Atom} {ts : List LTy} (h : LAtomsTy Γ as ts) :
    ts.length = as.length := by
  induction h with
  | nil => rfl
  | cons _ _ ih => simp [ih]

theorem ofScalar_erase {pr : Prim} {ts : List Ty} {t : Ty} (l : LExpr) (h : PrimTy pr ts t) :
    (ofScalar t l).erase = t := by
  cases h <;> rfl

theorem map_erase_const {t : LTy} :
    ∀ {ts : List LTy} {items : List Atom}, (∀ u ∈ ts, u.erase = t.erase) →
      ts.length = items.length → ts.map LTy.erase = items.map (fun _ => t.erase)
  | [], [], _, _ => rfl
  | [], _ :: _, _, h => by simp at h
  | _ :: _, [], _, h => by simp at h
  | u :: ts, _ :: items, hu, hl => by
    simp only [List.map_cons, hu u List.mem_cons_self]
    simp only [List.length_cons, Nat.add_right_cancel_iff] at hl
    rw [map_erase_const (fun v hv => hu v (List.mem_cons_of_mem _ hv)) hl]

theorem params_erase (params : List (String × LTy)) (Γ : LCtx) :
    LCtx.erase (params ++ Γ) = bindTys params (params.map (fun q => q.2.erase)) ++ Γ.erase := by
  induction params with
  | nil => rfl
  | cons q params ih =>
    simp only [LCtx.erase, List.cons_append, List.map_cons, bindTys, List.zip_cons_cons] at ih ⊢
    rw [ih]

/-- The parameters of a function instantiated, then erased, are its structural parameter types
instantiated. -/
theorem lsub_params_erase (fn : Fun) (σ : List (String × LTy)) :
    (fn.params.map (fun q => p.lsub σ q.2)).map LTy.erase = fn.paramTys (eraseSubst σ) := by
  simp only [List.map_map, Fun.paramTys]
  apply List.map_congr_left
  intro q _
  exact LTy.erase_lsubst _ σ q.2

mutual
  /-- Erasing labels from a labeled typing of a right-hand side gives a structural typing. -/
  theorem LRhsTy.toStruct :
      ∀ {Γ : LCtx} {pc : LExpr} {r : Rhs} {t : LTy} {ε : Eff},
        LRhsTy p E Γ pc r t ε → RhsTy p Γ.erase r t.erase
    | _, _, _, _, _, .atom h => .atom (latom_erase h)
    | _, _, _, _, _, .prim has hp => by
      rw [ofScalar_erase _ hp]
      exact .prim (latoms_erase has) hp
    | _, _, _, _, _, .call (fn := fn) (targs := targs) hf _ _ _ _ has hflows _ => by
      have h := latoms_erase has
      rw [← Flows.erase hflows, lsub_params_erase] at h
      rw [Program.lsub, LTy.erase_lsubst]
      exact .call hf h
    | _, _, _, _, _, .apply hf has hflows _ => by
      have h := latoms_erase has
      rw [← Flows.erase hflows, List.map_map] at h
      have hf' := latom_erase hf
      simp only [LTy.erase, LTy.erases_eq] at hf'
      rw [LTy.erase_raise]
      simp only [Function.comp_def, LTy.erase_raise] at h
      exact .apply hf' h
    | _, _, _, _, _, .op hl ht _ _ => by
      have h := latom_erase hl
      rw [ht] at h
      exact .op h
    | _, _, _, _, _, .get hu ht _ _ => by
      have h := latom_erase hu
      rw [ht] at h
      exact .get h
    | _, _, _, _, _, .construct hrec has hflows => by
      have h := latoms_erase has
      rw [← Flows.erase hflows] at h
      exact .construct hrec h
    | _, _, _, _, _, .field hv hrec hi => by
      rw [LTy.erase_raise]
      exact .field (latom_erase hv) hrec (by simp [Record.fieldTys, hi])
    | _, _, _, _, _, .list has hflow => by
      have h := latoms_erase has
      rw [map_erase_const (fun u hu => Flow.erase (hflow u hu)) (latoms_length has)] at h
      exact .list h
    | _, _, _, _, _, .none => .none
    | _, _, _, _, _, .some h => .some (latom_erase h)
    | _, _, _, _, _, .lambda (params := params) hb _ => by
      have h := LExprTy.toStruct hb
      rw [params_erase] at h
      simp only [LTy.erase, LTy.erases_eq, List.map_map]
      exact .lambda (by simp) h
    | _, _, _, _, _, .funRef (fn := fn) (targs := targs) hf _ _ _ _ => by
      simp only [LTy.erase, LTy.erases_eq]
      rw [lsub_params_erase, Program.lsub, LTy.erase_lsubst]
      exact .funRef hf
    | _, _, _, _, _, .declassify hpo _ ha hflow _ => by
      have h := latom_erase ha
      rw [Flow.erase hflow] at h
      exact .declassify hpo h
    | _, _, _, _, _, .map hxs hf hflow _ => by
      have h1 := latom_erase hxs
      have h2 := latom_erase hf
      simp only [LTy.erase, LTy.erases, Flow.erase hflow] at h1 h2 ⊢
      rw [LTy.erase_raise]
      exact .map h1 h2
    | _, _, _, _, _, .filter hxs hk hflow _ => by
      have h1 := latom_erase hxs
      have h2 := latom_erase hk
      simp only [LTy.erase, LTy.erases, Flow.erase hflow] at h1 h2 ⊢
      exact .filter h1 h2
    | _, _, _, _, _, .firstWhere hxs hk hflow _ => by
      have h1 := latom_erase hxs
      have h2 := latom_erase hk
      simp only [LTy.erase, LTy.erases, Flow.erase hflow] at h1 h2 ⊢
      exact .firstWhere h1 h2
    | _, _, _, _, _, .fold hxs hi hf he hinit hres _ => by
      have h1 := latom_erase hxs
      have h2 := latom_erase hi
      have h3 := latom_erase hf
      simp only [LTy.erase, LTy.erases, Flow.erase he, Flow.erase hres] at h1 h3 ⊢
      rw [Flow.erase hinit] at h2
      rw [LTy.erase_raise]
      exact .fold h1 h2 h3
    | _, _, _, _, _, .eq ha hb h₁ h₂ _ => by
      have ha' := latom_erase ha
      have hb' := latom_erase hb
      rw [Flow.erase h₁] at ha'
      rw [Flow.erase h₂] at hb'
      exact .eq ha' hb'
    | _, _, _, _, _, .parse hs ht => by
      have h := latom_erase hs
      rw [ht] at h
      exact .parse h

  /-- Erasing labels from a labeled typing of a body gives a structural typing. -/
  theorem LExprTy.toStruct :
      ∀ {Γ : LCtx} {pc : LExpr} {e : Expr} {t : LTy} {ε : Eff},
        LExprTy p E Γ pc e t ε → ExprTy p Γ.erase e t.erase
    | _, _, _, _, _, .atom h => .atom (latom_erase h)
    | _, _, _, _, _, .letR hr hflow hb => by
      have h := LExprTy.toStruct hb
      simp only [LCtx.erase, List.map_cons] at h
      rw [← Flow.erase hflow] at h
      exact .letR (LRhsTy.toStruct hr) h
    | _, _, _, _, _, .letE hv hflow hb => by
      have h := LExprTy.toStruct hb
      simp only [LCtx.erase, List.map_cons] at h
      rw [← Flow.erase hflow] at h
      exact .letE (LExprTy.toStruct hv) h
    | _, _, _, _, _, .ite hc ht he h₁ h₂ => by
      have hc' := latom_erase hc
      have ht' := LExprTy.toStruct ht
      have he' := LExprTy.toStruct he
      rw [Flow.erase h₁] at ht'
      rw [Flow.erase h₂] at he'
      rw [LTy.erase_raise]
      exact .ite hc' ht' he'
    | _, _, _, _, _, .caseOption hs hn hsome h₁ h₂ => by
      have hs' := latom_erase hs
      have hn' := LExprTy.toStruct hn
      have hsome' := LExprTy.toStruct hsome
      simp only [LCtx.erase, List.map_cons, LTy.erase_raise] at hsome'
      rw [Flow.erase h₁] at hn'
      rw [Flow.erase h₂] at hsome'
      rw [LTy.erase_raise]
      exact .caseOption hs' hn' hsome'
    | _, _, _, _, _, .caseResult hs hok herr h₁ h₂ => by
      have hs' := latom_erase hs
      have hok' := LExprTy.toStruct hok
      have herr' := LExprTy.toStruct herr
      simp only [LCtx.erase, List.map_cons, LTy.erase_raise] at hok' herr'
      rw [Flow.erase h₁] at hok'
      rw [Flow.erase h₂] at herr'
      rw [LTy.erase_raise]
      exact .caseResult hs' hok' herr'
    | _, _, _, _, _, .fail _ _ => .fail
    | _, _, _, _, _, .attempt hb => .attempt (LExprTy.toStruct hb)
    | _, _, _, _, _, .handle hb hh h₁ h₂ => by
      have hb' := LExprTy.toStruct hb
      have hh' := LExprTy.toStruct hh
      simp only [LCtx.erase, List.map_cons] at hh'
      rw [Flow.erase h₁] at hb'
      rw [Flow.erase h₂] at hh'
      rw [LTy.erase_raise]
      exact .handle hb' hh'
end

theorem paramTys_nil (fn : Fun) : fn.paramTys [] = fn.params.map (fun q => q.2.erase) := by
  simp [Fun.paramTys, Ty.subst_nil]

/-- A program the labeled typing accepts is well formed. -/
theorem Program.LWellFormed.wellFormed (hwf : p.LWellFormed) : p.WellFormed where
  funs fn hfn := by
    rcases hwf.funs fn hfn with ⟨t, ε, hb, hflow, _⟩ | ⟨_, h⟩
    · have h := LExprTy.toStruct hb
      have hp := params_erase fn.params []
      simp only [List.append_nil] at hp
      rw [hp, Flow.erase hflow] at h
      simpa [paramTys_nil, LCtx.erase] using h
    · exact h
  records := hwf.records
  policies := hwf.policies

/-- **T1 for the labeled typing of F1** (`spec/10 §10`): a body the labeled typing accepts, of a
program it accepts, run in an environment of its context's types with any fuel and any host,
never gets stuck, and a value it ends with has its type's structure. -/
theorem t1_labeled (hwf : p.LWellFormed) (w : World) {env : Env} {Γ : LCtx} {pc : LExpr}
    {e : Expr} {t : LTy} {ε : Eff}
    (henv : EnvTy p env Γ.erase) (he : LExprTy p E Γ pc e t ε) (n : Nat) :
    (eval n p w env e).2 ≠ .stuck ∧ ∀ v, (eval n p w env e).2 = .ok v → ValTy p v t.erase :=
  t1 hwf.wellFormed w henv (LExprTy.toStruct he) n

end Una.F1
