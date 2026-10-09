/-
The unary half's theorem for the generic-free part of F1: a run keeps to its labeled typing
(`unary_sound`).
-/
import UnaFormal.F1.InstVal

namespace Una.F1

variable {p : Program}

theorem walkMap_uok {app : Val → Trace × Res} {A : Event → Prop} {F : Prop} {a b : LTy} (l : LExpr)
    (happ : ∀ v, LValTy p v a → UOk A (fun r => LValTy p r b) F (app v)) :
    ∀ {items : List Val}, LValsAll p items a →
      UOk A (fun r => LValTy p r (.list b l)) F (walkMap app items)
  | [], _ => uok_done (.list .nil)
  | v :: vs, .cons hv hvs => by
    simp only [walkMap]
    refine andThen_uok (happ v hv) (fun r hr => ?_)
    refine andThen_uok (walkMap_uok l happ hvs) (fun rest hrest => ?_)
    cases hrest with
    | list hrs => exact uok_done (.list (.cons hr hrs))

theorem walkFilter_uok {app : Val → Trace × Res} {A : Event → Prop} {F : Prop} {a : LTy}
    {lb : LExpr} (l : LExpr)
    (happ : ∀ v, LValTy p v a → UOk A (fun r => LValTy p r (.bool lb)) F (app v)) :
    ∀ {items : List Val}, LValsAll p items a →
      UOk A (fun r => LValTy p r (.list a l)) F (walkFilter app items)
  | [], _ => uok_done (.list .nil)
  | v :: vs, .cons hv hvs => by
    simp only [walkFilter]
    refine andThen_uok (happ v hv) (fun b hb => ?_)
    cases hb with
    | @bool keep =>
      refine andThen_uok (walkFilter_uok l happ hvs) (fun rest hrest => ?_)
      cases hrest with
      | list hrs =>
        cases keep
        · exact uok_done (.list hrs)
        · exact uok_done (.list (.cons hv hrs))

theorem walkFirst_uok {app : Val → Trace × Res} {A : Event → Prop} {F : Prop} {a : LTy}
    {lb : LExpr} (l : LExpr)
    (happ : ∀ v, LValTy p v a → UOk A (fun r => LValTy p r (.bool lb)) F (app v)) :
    ∀ {items : List Val}, LValsAll p items a →
      UOk A (fun r => LValTy p r (.option a l)) F (walkFirst app items)
  | [], _ => uok_done .none
  | v :: vs, .cons hv hvs => by
    simp only [walkFirst]
    refine andThen_uok (happ v hv) (fun b hb => ?_)
    cases hb with
    | @bool keep =>
      cases keep
      · exact walkFirst_uok l happ hvs
      · exact uok_done (.some hv)

theorem walkFold_uok {app : Val → Val → Trace × Res} {A : Event → Prop} {F : Prop} {a b : LTy}
    (happ : ∀ acc v, LValTy p acc b → LValTy p v a → UOk A (fun r => LValTy p r b) F (app acc v)) :
    ∀ {items : List Val} {acc : Val}, LValTy p acc b → LValsAll p items a →
      UOk A (fun r => LValTy p r b) F (walkFold app acc items)
  | [], _, hacc, _ => uok_done hacc
  | v :: vs, _, hacc, .cons hv hvs => by
    simp only [walkFold]
    exact andThen_uok (happ _ v hacc hv) (fun acc' hacc' => walkFold_uok happ hacc' hvs)

/-- The generic-free part of F1: no function has type parameters, and there is no policy. -/
structure Program.GenericFree (p : Program) : Prop where
  funs : ∀ fn ∈ p.funs, fn.tparams = []
  policies : p.policies = []

/-- Without policies, every function passes T-Fun. -/
theorem funOk_of_find (hwf : p.LWellFormed) (hpol : p.policies = []) {f : String} {fn : Fun}
    (h : p.find f = some fn) : p.FunOk fn := by
  have hmem := find_mem h
  rcases hwf.funs fn hmem with hok | ⟨⟨po, hpo, _⟩, _⟩
  · exact hok
  · rw [hpol] at hpo
    nomatch hpo

/-! ### At an instance -/

section
variable {σ : List (String × LTy)}

theorem latom_ltypedσ {env : Env} {Γ : LCtx} {a : Atom} {t : LTy}
    (henv : LEnvTy p env (Γ.lsub p.recordDeep σ)) (h : LAtomTy Γ a t) :
    ∃ v, evalAtom env a = some v ∧ LValTy p v (t.lsubst p.recordDeep σ) := by
  cases h with
  | var hx => exact lookup_ltyped henv (by rw [lctxLookup_lsub, hx]; rfl)
  | int => exact ⟨_, rfl, .int⟩
  | bool => exact ⟨_, rfl, .bool⟩
  | text => exact ⟨_, rfl, .text⟩
  | unit => exact ⟨_, rfl, .unit⟩

theorem latoms_ltypedσ {env : Env} {Γ : LCtx} {as : List Atom} {ts : List LTy}
    (henv : LEnvTy p env (Γ.lsub p.recordDeep σ)) (h : LAtomsTy Γ as ts) :
    ∃ vs, evalAtoms env as = some vs ∧ LValsTy p vs (ts.map (LTy.lsubst p.recordDeep σ)) := by
  induction h with
  | nil => exact ⟨[], rfl, .nil⟩
  | cons ha _ ih =>
    obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv ha
    obtain ⟨vs, hvs, hvst⟩ := ih
    exact ⟨v :: vs, by simp [evalAtoms, hv, hvs], .cons hvt hvst⟩

theorem LValsTy.flowsσ {vs : List Val} {ts us : List LTy} (h : LValsTy p vs (ts.map (LTy.lsubst p.recordDeep σ)))
    (hf : Flows p.top ts us) : LValsTy p vs (us.map (LTy.lsubst p.recordDeep σ)) := by
  have := Flows.lsubst p.recordDeep σ hf
  rw [LTy.lsubsts_eq, LTy.lsubsts_eq] at this
  exact LValsTy.flows h this

theorem LValsTy.lsubst_unraise : ∀ {vs : List Val} {ps : List LTy} {e : LExpr},
    LValsTy p vs ((ps.map (LTy.raise e)).map (LTy.lsubst p.recordDeep σ)) →
      LValsTy p vs (ps.map (LTy.lsubst p.recordDeep σ))
  | [], [], _, .nil => .nil
  | _ :: _, _ :: _, _, .cons h hs => .cons (LValTy.lsubst_unraise h) (LValsTy.lsubst_unraise hs)

theorem ofScalar_lsubst (rd : String → Label) (t : Ty) (l : LExpr) :
    (ofScalar t l).lsubst rd σ = ofScalar t (l.inst (σ.map (fun q => (q.1, q.2.deep rd)))) := by
  unfold ofScalar; split <;> rfl

theorem LValTy.ground_lsubst {v : Val} {u : LTy} (hu : u.Ground) :
    LValTy p v (u.lsubst p.recordDeep σ) ↔ LValTy p v u :=
  ⟨fun h => LValTy.flow h (Flow.lsubst_ground _ σ hu).1, fun h => LValTy.flow h (Flow.lsubst_ground _ σ hu).2⟩

theorem LValsTy.ground_lsubst : ∀ {vs : List Val} {us : List LTy}, (∀ u ∈ us, u.Ground) →
    LValsTy p vs (us.map (LTy.lsubst p.recordDeep σ)) → LValsTy p vs us
  | [], [], _, .nil => .nil
  | _ :: _, _ :: _, hg, .cons h hs =>
    .cons ((LValTy.ground_lsubst (hg _ List.mem_cons_self)).mp h)
      (LValsTy.ground_lsubst (fun u hu => hg u (List.mem_cons_of_mem _ hu)) hs)

theorem le_inst_join_left (s : List (String × LExpr)) (a b : LExpr) :
    p.le (a.inst s) ((a.join b).inst s) := LExpr.le_inst s (LExpr.le_join_left a b)

theorem Fails.union_left_inst {pc : LExpr} {s : List (String × LExpr)} {ε₁ ε₂ : Eff} :
    Fails p pc (ε₁.inst s) → Fails p pc ((ε₁.union ε₂).inst s) := by
  refine Fails.mono (LExpr.le_refl pc) (fun l hl => ?_)
  simp only [Eff.inst, Option.map_eq_some_iff] at hl
  obtain ⟨l₁, h₁, rfl⟩ := hl
  obtain ⟨l', hl', hle⟩ := union_fails_left (p := p) (ε₂ := ε₂) l₁ h₁
  exact ⟨l'.inst s, by simp [Eff.inst, hl'], LExpr.le_inst s hle⟩

theorem Fails.union_right_inst {pc pc' : LExpr} {s : List (String × LExpr)} {ε₁ ε₂ : Eff}
    (hpc : p.le pc pc') : Fails p pc' (ε₂.inst s) → Fails p pc ((ε₁.union ε₂).inst s) := by
  refine Fails.mono hpc (fun l hl => ?_)
  simp only [Eff.inst, Option.map_eq_some_iff] at hl
  obtain ⟨l₂, h₂, rfl⟩ := hl
  obtain ⟨l', hl', hle⟩ := union_fails_right (p := p) (ε₁ := ε₁) l₂ h₂
  exact ⟨l'.inst s, by simp [Eff.inst, hl'], LExpr.le_inst s hle⟩

theorem allowed_union_left_inst {pc : LExpr} {s : List (String × LExpr)} {ε₁ ε₂ : Eff} :
    ∀ ev, Allowed p pc (ε₁.inst s) ev → Allowed p pc ((ε₁.union ε₂).inst s) ev :=
  Allowed.mono (LExpr.le_refl pc) union_op_left union_get_left

theorem allowed_union_right_inst {pc pc' : LExpr} {s : List (String × LExpr)} {ε₁ ε₂ : Eff}
    (hpc : p.le pc pc') : ∀ ev, Allowed p pc' (ε₂.inst s) ev → Allowed p pc ((ε₁.union ε₂).inst s) ev :=
  Allowed.mono hpc union_op_right union_get_right

/-- At a call whose program counter's instance the function's bound allows. -/
theorem bound_allowed_inst {pc pc' : LExpr} {s : List (String × LExpr)} {o g f : Bool}
    (hb : p.Bound pc' o g f) (hpc : p.le pc (pc'.inst s)) :
    ∀ ev, FnAllowed o g ev → Allowed p pc ((Eff.ofFn o g f).inst s) ev :=
  bound_allowed (hb.inst s) hpc

theorem bound_fails_inst {pc pc' : LExpr} {s : List (String × LExpr)} {o g f : Bool}
    (hb : p.Bound pc' o g f) (hpc : p.le pc (pc'.inst s)) (hf : f = true) :
    Fails p pc ((Eff.ofFn o g f).inst s) := by
  refine ⟨LExpr.bot.inst s, by simp [Eff.ofFn, Eff.inst, hf], ?_⟩
  exact LExpr.le_trans hpc (LExpr.le_trans ((hb.inst s).2.2 hf).1
    (by rw [show LExpr.bot = LExpr.ofLabel Label.bot from rfl, LExpr.inst_ofLabel]; exact LExpr.le_join_right _ _))

theorem fnAllowed_of {pc : LExpr} {s : List (String × LExpr)} {ε : Eff} {o g : Bool}
    (ho : ε.op = true → o = true) (hg : ε.get = true → g = true) :
    ∀ ev, Allowed p pc (ε.inst s) ev → FnAllowed o g ev
  | .out _, h => ho h.1
  | .get _, h => hg h.1

theorem flag_of_fails {pc : LExpr} {s : List (String × LExpr)} {ε : Eff} {f : Bool}
    (hf : ε.fails.isSome = true → f = true) : Fails p pc (ε.inst s) → f = true := by
  rintro ⟨l, hl, _⟩
  apply hf
  simp only [Eff.inst, Option.map_eq_some_iff] at hl
  obtain ⟨l', hl', _⟩ := hl
  simp [hl']

end

/-- **The unary half, at every ground instance.** A run of a body the labeled typing accepts,
in an environment of the instance of its context, keeps to the instance of its typing. -/
theorem unary_sound (hwf : p.LWellFormed) (hpol : p.policies = []) (w : World) : ∀ n : Nat,
    (∀ {σ : List (String × LTy)} {env : Env} {Γ : LCtx} {E : List String} {pc : LExpr} {r : Rhs}
      {t : LTy} {ε : Eff}, GroundSubst σ → LEnvTy p env (Γ.lsub p.recordDeep σ) →
      LRhsTy p E Γ pc r t ε →
      UGood p (pc.inst (p.sd σ)) (t.lsubst p.recordDeep σ) (ε.inst (p.sd σ)) (evalRhs n p w env r)) ∧
    (∀ {σ : List (String × LTy)} {env : Env} {Γ : LCtx} {E : List String} {pc : LExpr} {e : Expr}
      {t : LTy} {ε : Eff}, GroundSubst σ → LEnvTy p env (Γ.lsub p.recordDeep σ) →
      LExprTy p E Γ pc e t ε →
      UGood p (pc.inst (p.sd σ)) (t.lsubst p.recordDeep σ) (ε.inst (p.sd σ)) (eval n p w env e))
  | 0 => ⟨fun _ _ _ => by simp [evalRhs, UGood, UOk, URes],
      fun _ _ _ => by simp [eval, UGood, UOk, URes]⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := unary_sound hwf hpol w n
    have hrun : ∀ {σ : List (String × LTy)} {env : Env} {Γ : LCtx} {E : List String} {e : Expr}
        {t : LTy} {ε : Eff}, GroundSubst σ → LEnvTy p env (Γ.lsub p.recordDeep σ) →
        LExprTy p E Γ .bot e t ε →
        UGood p (LExpr.bot.inst (p.sd σ)) (t.lsubst p.recordDeep σ) (ε.inst (p.sd σ)) (eval n p w env e) :=
      fun hσ henv he => ihE hσ henv he
    constructor
    · intro σ env Γ E pc r t ε hσ henv hr
      have hpc0 : ∀ x : LExpr, p.le (pc.inst (p.sd σ)) ((pc.join x).inst (p.sd σ)) :=
        fun x => le_inst_join_left _ pc x
      cases hr with
      | atom ha =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv ha
        exact ⟨fun _ h => by simp [evalRhs] at h, by simpa [evalRhs, hv, URes] using hvt⟩
      | prim has hp =>
        obtain ⟨vs, hvs, _⟩ := latoms_ltypedσ henv has
        refine ⟨fun _ h => by simp [evalRhs] at h, ?_⟩
        simp only [evalRhs, hvs, Option.bind_some]
        cases he : evalPrim _ vs with
        | none => trivial
        | some v =>
          show LValTy p v _
          rw [ofScalar_lsubst]
          exact prim_ltyped _ hp he
      | @call _ _ f targs args fn ts hf _ hlen _ _ hg has hfl hb =>
        obtain ⟨tb, εb, hbody, hflow, ho, hgo, hfs, _⟩ := funOk_of_find hwf hpol hf
        have hτ : GroundSubst (fn.tparams.zip targs) := zip_ground hg
        have hcov := lookup_zip_cover (xs := fn.tparams) hlen
        obtain ⟨hps, hres⟩ := hwf.sigsScoped fn (find_mem hf)
        have hpg : ∀ u ∈ fn.params.map (fun q => p.lsub (fn.tparams.zip targs) q.2), u.Ground := by
          intro u hu
          obtain ⟨q, hq, rfl⟩ := List.mem_map.mp hu
          exact LTy.lsubst_ground _ hτ hcov (hps q hq)
        obtain ⟨vs, hvs, hvst⟩ := latoms_ltypedσ henv has
        have ha := LValsTy.ground_lsubst hpg (LValsTy.flowsσ hvst hfl)
        have ha' : LValsTy p vs ((LCtx.lsub p.recordDeep (fn.tparams.zip targs) fn.params).map Prod.snd) := by
          rw [LCtx.lsub_snd]; simpa [Program.lsub, List.map_map, Function.comp_def] using ha
        simp only [evalRhs, hf, hvs]
        rw [ite_eq_left (by rw [LValsTy.length ha, List.length_map])]
        have henv' := bindParams_ltyped ha' (LEnvTy.nil (p := p))
        rw [List.append_nil] at henv'
        have henv'' : LEnvTy p (bindParams fn.params vs []) (LCtx.lsub p.recordDeep (fn.tparams.zip targs) fn.params) := by
          simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using henv'
        refine UOk.mono (fun ev hev => bound_allowed_inst hb (LExpr.le_refl _) ev
            (fnAllowed_of ho hgo ev hev)) (fun v hv => ?_)
          (fun h => bound_fails_inst hb (LExpr.le_refl _) (flag_of_fails hfs h))
          (ihE hτ (by simpa using henv'') hbody)
        have hr := LTy.lsubst_ground p.recordDeep hτ hcov hres
        exact (LValTy.ground_lsubst hr).mpr (LValTy.flow hv (Flow.lsubst _ _ hflow))
      | apply hf has hfl hb =>
        obtain ⟨fv, hfv, hft⟩ := latom_ltypedσ henv hf
        obtain ⟨vs, hvs, hvst⟩ := latoms_ltypedσ henv has
        simp only [evalRhs, hfv, hvs]
        have hargs := LValsTy.lsubst_unraise (LValsTy.flowsσ hvst hfl)
        simp only [LTy.lsubst, LTy.lsubsts_eq] at hft
        refine UOk.mono (bound_allowed_inst hb (le_inst_join_left _ pc _))
          (fun v hv => LValTy.lsubst_raise hv)
          (fun h => bound_fails_inst hb (le_inst_join_left _ pc _) h)
          (applyWith_uok hrun hft hargs)
      | @op _ _ line t hl ht hle _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv hl
        cases t <;> simp [LTy.erase] at ht
        simp only [LTy.lsubst] at hvt
        cases hvt
        simp only [evalRhs, hv]
        refine ⟨fun ev hev => ?_, LValTy.unit⟩
        simp only [List.mem_singleton] at hev
        subst hev
        exact ⟨rfl, LExpr.le_trans (hpc0 _) (LExpr.le_of_le_inst_ofLabel (LExpr.le_inst _ hle))⟩
      | @get _ _ url t hu ht hle _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv hu
        cases t <;> simp [LTy.erase] at ht
        simp only [LTy.lsubst] at hvt
        cases hvt
        rename_i _ s
        simp only [evalRhs, hv]
        cases w.get s with
        | some body =>
          refine ⟨fun ev hev => ?_, LValTy.text⟩
          simp only [List.mem_singleton] at hev
          subst hev
          exact ⟨rfl, LExpr.le_trans (hpc0 _) (LExpr.le_of_le_inst_ofLabel (LExpr.le_inst _ hle))⟩
        | none =>
          refine And.intro (fun ev hev => ?_) (Exists.intro _ ⟨rfl, hpc0 _⟩)
          simp only [List.mem_singleton] at hev
          subst hev
          exact ⟨rfl, LExpr.le_trans (hpc0 _) (LExpr.le_of_le_inst_ofLabel (LExpr.le_inst _ hle))⟩
      | construct hrec has hfl =>
        obtain ⟨vs, hvs, hvst⟩ := latoms_ltypedσ henv has
        have := LValsTy.ground_lsubst (hwf.recordsGround _ (record_mem hrec)) (LValsTy.flowsσ hvst hfl)
        exact ⟨fun _ h => by simp [evalRhs] at h,
          by simpa [evalRhs, hvs, URes, LTy.lsubst] using LValTy.record hrec this⟩
      | field hv hrec hi =>
        obtain ⟨v, hv', hvt⟩ := latom_ltypedσ henv hv
        simp only [LTy.lsubst] at hvt
        cases hvt with
        | record hrec' hvals =>
          rw [hrec] at hrec'
          cases hrec'
          obtain ⟨u, hu, hut⟩ := lfield_typed hvals hi
          have hg := hwf.recordsGround _ (record_mem hrec) _ (List.mem_of_getElem? hi)
          exact ⟨fun _ h => by simp [evalRhs, hv'] at h,
            by simpa [evalRhs, hv', hu, URes] using
              LValTy.lsubst_raise ((LValTy.ground_lsubst hg).mpr hut)⟩
      | list has hfl _ =>
        obtain ⟨vs, hvs, hvst⟩ := latoms_ltypedσ henv has
        refine ⟨fun _ h => by simp [evalRhs] at h, ?_⟩
        simp only [evalRhs, hvs, Option.map, Option.elim, URes, LTy.lsubst]
        exact .list (LValsAll.of_flows hvst (fun u hu => by
          obtain ⟨u', hu', rfl⟩ := List.mem_map.mp hu
          exact Flow.lsubst _ _ (hfl u' hu')))
      | none _ => exact uok_done LValTy.none
      | some ha =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv ha
        exact ⟨fun _ h => by simp [evalRhs] at h, by simpa [evalRhs, hv, URes, LTy.lsubst] using LValTy.some hvt⟩
      | lambda hb hfl _ =>
        refine uok_done ?_
        simp only [LTy.lsubst]
        exact .closure hσ henv hb hfl (by rw [LTy.lsubsts_eq]; exact Flows.refl _) (Flow.refl _)
          id id id
      | @funRef _ _ f targs fn hf _ hlen _ _ hg =>
        obtain ⟨tb, εb, hbody, hflow, ho, hgo, hfs, hfl⟩ := funOk_of_find hwf hpol hf
        have hτ : GroundSubst (fn.tparams.zip targs) := zip_ground hg
        have hcov := lookup_zip_cover (xs := fn.tparams) hlen
        obtain ⟨hps, hres⟩ := hwf.sigsScoped fn (find_mem hf)
        simp only [evalRhs, hf]
        refine uok_done ?_
        simp only [LTy.lsubst, Program.lsub]
        refine .closure (Γ := []) (E := fn.eqParams) hτ .nil (by simpa using hbody) hfl ?_ ?_ ho hgo hfs
        · rw [LTy.lsubsts_eq]
          have : ∀ ts : List (String × LTy), (∀ q ∈ ts, q.2.ScopedIn fn.tparams) →
              Flows p.top ((ts.map (fun q => q.2.lsubst p.recordDeep (fn.tparams.zip targs))).map
                (LTy.lsubst p.recordDeep σ))
                ((ts.map Prod.snd).map (LTy.lsubst p.recordDeep (fn.tparams.zip targs))) := by
            intro ts hts
            induction ts with
            | nil => exact .nil
            | cons q qs ih =>
              exact .cons (Flow.lsubst_ground _ σ
                  (LTy.lsubst_ground _ hτ hcov (hts q List.mem_cons_self))).1
                (ih (fun q' hq' => hts q' (List.mem_cons_of_mem _ hq')))
          exact this fn.params hps
        · exact Flow.trans (Flow.lsubst _ _ hflow)
            (Flow.lsubst_ground _ σ (LTy.lsubst_ground _ hτ hcov hres)).2
      | declassify hpo =>
        have := policy_mem hpo
        rw [hpol] at this
        nomatch this
      | map hxs hf hte hb =>
        obtain ⟨xv, hxv, hxt⟩ := latom_ltypedσ henv hxs
        obtain ⟨fv, hfv, hft⟩ := latom_ltypedσ henv hf
        simp only [LTy.lsubst, LTy.lsubsts] at hxt hft
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hfv]
          have hpc : ∀ a b : LExpr, p.le (pc.inst (p.sd σ)) (((pc.join a).join b).inst (p.sd σ)) :=
            fun _ _ => LExpr.le_inst _ (LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_left _ _))
          refine UOk.mono (bound_allowed_inst hb (hpc _ _)) (fun v hv => by simpa [LTy.lsubst] using hv)
            (bound_fails_inst hb (hpc _ _)) ?_
          exact walkMap_uok _ (fun v hv => UOk.mono (fun _ h => h) (fun _ h => LValTy.lsubst_raise h) id
            (applyWith_uok hrun hft (.cons (LValTy.flow hv (Flow.lsubst _ _ hte)) .nil))) hitems
      | filter hxs hk hte hb =>
        obtain ⟨xv, hxv, hxt⟩ := latom_ltypedσ henv hxs
        obtain ⟨fv, hfv, hft⟩ := latom_ltypedσ henv hk
        simp only [LTy.lsubst, LTy.lsubsts] at hxt hft
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hfv]
          have hpc : ∀ a b : LExpr, p.le (pc.inst (p.sd σ)) (((pc.join a).join b).inst (p.sd σ)) :=
            fun _ _ => LExpr.le_inst _ (LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_left _ _))
          refine UOk.mono (bound_allowed_inst hb (hpc _ _)) (fun v hv => by simpa [LTy.lsubst] using hv)
            (bound_fails_inst hb (hpc _ _)) ?_
          exact walkFilter_uok _ (fun v hv =>
            applyWith_uok hrun hft (.cons (LValTy.flow hv (Flow.lsubst _ _ hte)) .nil)) hitems
      | firstWhere hxs hk hte hb =>
        obtain ⟨xv, hxv, hxt⟩ := latom_ltypedσ henv hxs
        obtain ⟨fv, hfv, hft⟩ := latom_ltypedσ henv hk
        simp only [LTy.lsubst, LTy.lsubsts] at hxt hft
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hfv]
          have hpc : ∀ a b c d : LExpr,
              p.le (pc.inst (p.sd σ)) (((((pc.join a).join b).join c).join d).inst (p.sd σ)) :=
            fun _ _ _ _ => LExpr.le_inst _ (LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_trans
              (LExpr.le_join_left _ _) (LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_left _ _))))
          refine UOk.mono (bound_allowed_inst hb (hpc _ _ _ _)) (fun v hv => by simpa [LTy.lsubst] using hv)
            (bound_fails_inst hb (hpc _ _ _ _)) ?_
          exact walkFirst_uok _ (fun v hv =>
            applyWith_uok hrun hft (.cons (LValTy.flow hv (Flow.lsubst _ _ hte)) .nil)) hitems
      | fold hxs hi hf hte hinit hres hb =>
        obtain ⟨xv, hxv, hxt⟩ := latom_ltypedσ henv hxs
        obtain ⟨iv, hiv, hit⟩ := latom_ltypedσ henv hi
        obtain ⟨fv, hfv, hft⟩ := latom_ltypedσ henv hf
        simp only [LTy.lsubst, LTy.lsubsts] at hxt hft
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hiv, hfv]
          have hpc : ∀ a b : LExpr, p.le (pc.inst (p.sd σ)) (((pc.join a).join b).inst (p.sd σ)) :=
            fun _ _ => LExpr.le_inst _ (LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_left _ _))
          refine UOk.mono (bound_allowed_inst hb (hpc _ _)) (fun v hv => LValTy.lsubst_raise hv)
            (bound_fails_inst hb (hpc _ _)) ?_
          exact walkFold_uok (fun acc v hacc hv => UOk.mono (fun _ h => h)
            (fun _ h => LValTy.flow h (Flow.lsubst _ _ hres)) id
            (applyWith_uok hrun hft (.cons hacc (.cons (LValTy.flow hv (Flow.lsubst _ _ hte)) .nil))))
            (LValTy.flow hit (Flow.lsubst _ _ hinit)) hitems
      | @parse _ _ s₀ t hs ht =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv hs
        cases t <;> simp [LTy.erase] at ht
        simp only [LTy.lsubst] at hvt
        cases hvt
        rename_i _ s
        simp only [evalRhs, hv]
        unfold parseInt
        split
        · exact uok_done LValTy.int
        · exact And.intro (fun _ h => nomatch h) (Exists.intro _ ⟨rfl, hpc0 _⟩)
      | eq ha hb _ _ _ =>
        obtain ⟨va, hva, _⟩ := latom_ltypedσ henv ha
        obtain ⟨vb, hvb, _⟩ := latom_ltypedσ henv hb
        simp only [evalRhs, hva, hvb, done]
        exact uok_done LValTy.bool
    · intro σ env Γ E pc e t ε hσ henv he
      have hpc0 : ∀ x : LExpr, p.le (pc.inst (p.sd σ)) ((pc.join x).inst (p.sd σ)) :=
        fun x => le_inst_join_left _ pc x
      cases he with
      | atom ha =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv ha
        exact ⟨fun _ h => by simp [eval] at h, by simpa [eval, hv, URes] using hvt⟩
      | letR hr hflow hb _ =>
        simp only [eval]
        refine andThen_uok (UOk.mono allowed_union_left_inst
          (fun v hv => LValTy.flow hv (Flow.lsubst _ _ hflow)) Fails.union_left_inst
          (ihR hσ henv hr)) (fun v hv => ?_)
        exact UOk.mono (allowed_union_right_inst (hpc0 _)) (fun _ h => h)
          (Fails.union_right_inst (hpc0 _)) (ihE hσ (.cons hv henv) hb)
      | letE hv hflow hb _ =>
        simp only [eval]
        refine andThen_uok (UOk.mono allowed_union_left_inst
          (fun v hv => LValTy.flow hv (Flow.lsubst _ _ hflow)) Fails.union_left_inst
          (ihE hσ henv hv)) (fun v hv => ?_)
        exact UOk.mono (allowed_union_right_inst (hpc0 _)) (fun _ h => h)
          (Fails.union_right_inst (hpc0 _)) (ihE hσ (.cons hv henv) hb)
      | ite hc ht he h₁ h₂ _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv hc
        simp only [LTy.lsubst] at hvt
        cases hvt with
        | @bool b _ =>
          cases b
          · simp only [eval, hv]
            exact UOk.mono (allowed_union_right_inst (hpc0 _))
              (fun _ h => LValTy.lsubst_raise (LValTy.flow h (Flow.lsubst _ _ h₂)))
              (Fails.union_right_inst (hpc0 _)) (ihE hσ henv he)
          · simp only [eval, hv]
            exact UOk.mono (Allowed.mono (hpc0 _) union_op_left union_get_left)
              (fun _ h => LValTy.lsubst_raise (LValTy.flow h (Flow.lsubst _ _ h₁)))
              (fun h => Fails.union_left_inst (Fails.mono (hpc0 _) (fun l hl => ⟨l, hl, LExpr.le_refl l⟩) h))
              (ihE hσ henv ht)
      | caseOption hs hn hsome h₁ h₂ _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv hs
        simp only [LTy.lsubst] at hvt
        cases hvt with
        | none =>
          simp only [eval, hv]
          exact UOk.mono (Allowed.mono (hpc0 _) union_op_left union_get_left)
            (fun _ h => LValTy.lsubst_raise (LValTy.flow h (Flow.lsubst _ _ h₁)))
            (fun h => Fails.union_left_inst (Fails.mono (hpc0 _) (fun l hl => ⟨l, hl, LExpr.le_refl l⟩) h))
            (ihE hσ henv hn)
        | some hw =>
          simp only [eval, hv]
          exact UOk.mono (allowed_union_right_inst (hpc0 _))
            (fun _ h => LValTy.lsubst_raise (LValTy.flow h (Flow.lsubst _ _ h₂)))
            (Fails.union_right_inst (hpc0 _))
            (ihE hσ (.cons (LValTy.lsubst_raise hw) henv) hsome)
      | caseResult hs hok herr h₁ h₂ _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv hs
        simp only [LTy.lsubst] at hvt
        cases hvt with
        | ok hw =>
          simp only [eval, hv]
          exact UOk.mono (Allowed.mono (hpc0 _) union_op_left union_get_left)
            (fun _ h => LValTy.lsubst_raise (LValTy.flow h (Flow.lsubst _ _ h₁)))
            (fun h => Fails.union_left_inst (Fails.mono (hpc0 _) (fun l hl => ⟨l, hl, LExpr.le_refl l⟩) h))
            (ihE hσ (.cons (LValTy.lsubst_raise hw) henv) hok)
        | err =>
          simp only [eval, hv]
          exact UOk.mono (allowed_union_right_inst (hpc0 _))
            (fun _ h => LValTy.lsubst_raise (LValTy.flow h (Flow.lsubst _ _ h₂)))
            (Fails.union_right_inst (hpc0 _)) (ihE hσ (.cons .int henv) herr)
      | fail hpc _ _ =>
        exact ⟨fun _ h => by simp [eval] at h, _, rfl,
          LExpr.le_trans (LExpr.le_inst _ hpc)
            (by rw [show LExpr.bot = LExpr.ofLabel Label.bot from rfl, LExpr.inst_ofLabel]
                exact LExpr.join_le (LExpr.le_join_left _ _) (LExpr.le_join_right _ _))⟩
      | attempt hb =>
        have h := ihE hσ henv hb
        simp only [eval]
        revert h
        cases eval n p w env _ with
        | mk tr r =>
          intro h
          have hev : ∀ ev ∈ tr, Allowed p (pc.inst (p.sd σ)) ((⟨_, _, Option.none⟩ : Eff).inst (p.sd σ)) ev :=
            fun ev hev => Allowed.mono (LExpr.le_refl _) id id ev (h.1 ev hev)
          cases r with
          | ok v => exact ⟨hev, .ok h.2⟩
          | failed c => exact ⟨hev, .err⟩
          | stuck => exact ⟨hev, trivial⟩
          | outOfFuel => exact ⟨hev, trivial⟩
      | @handle _ _ body x handler _ _ _ ε₁ ε₂ hb hh h₁ h₂ _ =>
        have h := ihE hσ henv hb
        simp only [eval]
        revert h
        cases eval n p w env _ with
        | mk tr r =>
          intro h
          have hev : ∀ ev ∈ tr, Allowed p (pc.inst (p.sd σ))
              ((⟨ε₁.op || ε₂.op, ε₁.get || ε₂.get, ε₂.fails⟩ : Eff).inst (p.sd σ)) ev :=
            fun ev hev => Allowed.mono (LExpr.le_refl _) (fun h => by simp only [Eff.inst] at h ⊢; simp [h])
              (fun h => by simp only [Eff.inst] at h ⊢; simp [h]) ev (h.1 ev hev)
          cases r with
          | failed code =>
            have hh' := ihE hσ (.cons (LValTy.int (n := (code : Int))) henv) hh
            rcases hrh : eval n p w ((x, Val.int (code : Int)) :: env) handler with ⟨tr', r'⟩
            rw [hrh] at hh'
            simp only [hrh]
            refine ⟨fun ev hev' => ?_, ?_⟩
            · rcases List.mem_append.mp hev' with hev' | hev'
              · exact hev ev hev'
              · exact Allowed.mono (hpc0 _) (fun h => by simp only [Eff.inst] at h ⊢; simp [h])
                  (fun h => by simp only [Eff.inst] at h ⊢; simp [h]) ev (hh'.1 ev hev')
            · have h2 := hh'.2
              cases r' <;> simp only [URes] at h2 ⊢
              · exact LValTy.lsubst_raise (LValTy.flow h2 (Flow.lsubst _ _ h₂))
              · exact Fails.mono (hpc0 _) (fun l hl => ⟨l, hl, LExpr.le_refl l⟩) h2
          | ok v => exact ⟨hev, LValTy.lsubst_raise (LValTy.flow h.2 (Flow.lsubst _ _ h₁))⟩
          | stuck => exact ⟨hev, trivial⟩
          | outOfFuel => exact ⟨hev, trivial⟩

end Una.F1
