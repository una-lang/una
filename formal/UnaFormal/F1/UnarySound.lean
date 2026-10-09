/-
The unary half's theorem for the generic-free part of F1: a run keeps to its labeled typing
(`unary_sound`).
-/
import UnaFormal.F1.Unary

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

/-- In the generic-free part, every function passes T-Fun. -/
theorem funOk_of_find (hwf : p.LWellFormed) (hgf : p.GenericFree) {f : String} {fn : Fun}
    (h : p.find f = some fn) : p.FunOk fn ∧ fn.tparams = [] := by
  have hmem := find_mem h
  refine ⟨?_, hgf.funs fn hmem⟩
  rcases hwf.funs fn hmem with hok | ⟨⟨po, hpo, _⟩, _⟩
  · exact hok
  · rw [hgf.policies] at hpo
    nomatch hpo

/-- The arguments of a generic-free call have its parameters' labeled types. -/
theorem args_of_call {fn : Fun} {vs : List Val} {ts : List LTy} {targs : List LTy}
    (htp : fn.tparams = []) (hvs : LValsTy p vs ts)
    (hfl : Flows p.top ts (fn.params.map (fun q => p.lsub (fn.tparams.zip targs) q.2))) :
    LValsTy p vs (fn.params.map Prod.snd) := by
  rw [htp, List.zip_nil_left] at hfl
  exact LValsTy.flows (LValsTy.flows hvs hfl) (flows_lsub_nil_params fn).1

theorem unary_sound (hwf : p.LWellFormed) (hgf : p.GenericFree) (w : World) : ∀ n : Nat,
    (∀ {env : Env} {Γ : LCtx} {E : List String} {pc : LExpr} {r : Rhs} {t : LTy} {ε : Eff},
      LEnvTy p env Γ → LRhsTy p E Γ pc r t ε → UGood p pc t ε (evalRhs n p w env r)) ∧
    (∀ {env : Env} {Γ : LCtx} {E : List String} {pc : LExpr} {e : Expr} {t : LTy} {ε : Eff},
      LEnvTy p env Γ → LExprTy p E Γ pc e t ε → UGood p pc t ε (eval n p w env e))
  | 0 => ⟨fun _ _ => by simp [evalRhs, UGood, UOk, URes],
      fun _ _ => by simp [eval, UGood, UOk, URes]⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := unary_sound hwf hgf w n
    have hrun : ∀ {env : Env} {Γ : LCtx} {E : List String} {e : Expr} {t : LTy} {ε : Eff},
        LEnvTy p env Γ → LExprTy p E Γ .bot e t ε → UGood p .bot t ε (eval n p w env e) :=
      fun henv he => ihE henv he
    constructor
    · intro env Γ E pc r t ε henv hr
      cases hr with
      | atom ha =>
        obtain ⟨v, hv, hvt⟩ := latom_ltyped henv ha
        exact ⟨fun _ h => by simp [evalRhs] at h, by simpa [evalRhs, hv, URes] using hvt⟩
      | prim has hp =>
        obtain ⟨vs, hvs, _⟩ := latoms_ltyped henv has
        refine ⟨fun _ h => by simp [evalRhs] at h, ?_⟩
        simp only [evalRhs, hvs, Option.bind_some]
        cases he : evalPrim _ vs with
        | none => trivial
        | some v => exact prim_ltyped _ hp he
      | @call _ _ f targs args fn ts hf _ _ _ _ has hfl hb =>
        obtain ⟨⟨tb, εb, hbody, hflow, ho, hg, hfs, _⟩, htp⟩ := funOk_of_find hwf hgf hf
        obtain ⟨vs, hvs, hvst⟩ := latoms_ltyped henv has
        have hargs := args_of_call htp hvst hfl
        simp only [evalRhs, hf, hvs]
        rw [ite_eq_left (by rw [LValsTy.length hargs, List.length_map])]
        have henv' := bindParams_ltyped hargs (LEnvTy.nil (p := p))
        rw [List.append_nil] at henv'
        refine UOk.mono ?_ (fun v hv => ?_) ?_ (ihE henv' hbody)
        · intro ev hev
          refine bound_allowed hb (LExpr.le_refl pc) ev ?_
          cases ev with
          | out _ => exact ho hev.1
          | get _ => exact hg hev.1
        · rw [htp, List.zip_nil_left]
          exact LValTy.flow (LValTy.flow hv hflow) (Flow.lsubst_nil _ _).2
        · rintro ⟨l, hl, _⟩
          exact bound_fails hb (LExpr.le_refl pc) (hfs (by simp [hl]))
      | apply hf has hfl hb =>
        obtain ⟨fv, hfv, hft⟩ := latom_ltyped henv hf
        obtain ⟨vs, hvs, hvst⟩ := latoms_ltyped henv has
        simp only [evalRhs, hfv, hvs]
        refine UOk.mono (bound_allowed hb (LExpr.le_join_left _ _))
          (fun v hv => LValTy.raise _ hv) (bound_fails hb (LExpr.le_join_left _ _))
          (applyWith_uok hrun hft (LValsTy.unraise (LValsTy.flows hvst hfl)))
      | op hl ht hle _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltyped henv hl
        cases hvt <;> simp [LTy.erase] at ht
        simp only [evalRhs, hv]
        refine ⟨fun ev hev => ?_, LValTy.unit⟩
        simp only [List.mem_singleton] at hev
        subst hev
        exact ⟨rfl, LExpr.le_of_join_left hle⟩
      | get hu ht hle _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltyped henv hu
        cases hvt <;> simp [LTy.erase] at ht
        rename_i s _
        simp only [evalRhs, hv]
        cases w.get s with
        | some body =>
          refine ⟨fun ev hev => ?_, LValTy.text⟩
          simp only [List.mem_singleton] at hev
          subst hev
          exact ⟨rfl, LExpr.le_of_join_left hle⟩
        | none =>
          refine ⟨fun ev hev => ?_, _, rfl, LExpr.le_join_left _ _⟩
          simp only [List.mem_singleton] at hev
          subst hev
          exact ⟨rfl, LExpr.le_of_join_left hle⟩
      | construct hrec has hfl =>
        obtain ⟨vs, hvs, hvst⟩ := latoms_ltyped henv has
        exact ⟨fun _ h => by simp [evalRhs] at h,
          by simpa [evalRhs, hvs, URes] using LValTy.record hrec (LValsTy.flows hvst hfl)⟩
      | field hv hrec hi =>
        obtain ⟨v, hv', hvt⟩ := latom_ltyped henv hv
        cases hvt with
        | record hrec' hvals =>
          rw [hrec] at hrec'
          cases hrec'
          obtain ⟨u, hu, hut⟩ := lfield_typed hvals hi
          exact ⟨fun _ h => by simp [evalRhs, hv'] at h,
            by simpa [evalRhs, hv', hu, URes] using LValTy.raise _ hut⟩
      | list has hfl =>
        obtain ⟨vs, hvs, hvst⟩ := latoms_ltyped henv has
        exact ⟨fun _ h => by simp [evalRhs] at h,
          by simpa [evalRhs, hvs, URes] using LValTy.list (l := .bot) (LValsAll.of_flows hvst hfl)⟩
      | none => exact uok_done LValTy.none
      | some ha =>
        obtain ⟨v, hv, hvt⟩ := latom_ltyped henv ha
        exact ⟨fun _ h => by simp [evalRhs] at h, by simpa [evalRhs, hv, URes] using LValTy.some hvt⟩
      | lambda hb hfl =>
        exact uok_done (LValTy.closure henv hb hfl (Flows.refl _) (Flow.refl _) id id id)
      | @funRef _ _ f targs fn hf _ _ _ _ =>
        obtain ⟨⟨tb, εb, hbody, hflow, ho, hg, hfs, hfl⟩, htp⟩ := funOk_of_find hwf hgf hf
        simp only [evalRhs, hf]
        apply uok_done
        rw [htp, List.zip_nil_left]
        exact .closure .nil (by simpa using hbody) hfl (flows_lsub_nil_params fn).1
          (Flow.trans hflow (Flow.lsubst_nil _ _).2) ho hg hfs
      | declassify hpo =>
        have := policy_mem hpo
        rw [hgf.policies] at this
        nomatch this
      | map hxs hf hte hb =>
        obtain ⟨xv, hxv, hxt⟩ := latom_ltyped henv hxs
        obtain ⟨fv, hfv, hft⟩ := latom_ltyped henv hf
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hfv]
          refine UOk.mono (bound_allowed hb (LExpr.le_trans (LExpr.le_join_left _ _)
            (LExpr.le_join_left _ _))) (fun v hv => hv)
            (bound_fails hb (LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_left _ _))) ?_
          exact walkMap_uok _ (fun v hv => UOk.mono (fun _ h => h) (fun _ h => LValTy.raise _ h)
            id (applyWith_uok hrun hft (.cons (LValTy.flow hv hte) .nil))) hitems
      | filter hxs hk hte hb =>
        obtain ⟨xv, hxv, hxt⟩ := latom_ltyped henv hxs
        obtain ⟨fv, hfv, hft⟩ := latom_ltyped henv hk
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hfv]
          refine UOk.mono (bound_allowed hb (LExpr.le_trans (LExpr.le_join_left _ _)
            (LExpr.le_join_left _ _))) (fun v hv => hv)
            (bound_fails hb (LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_left _ _))) ?_
          exact walkFilter_uok _ (fun v hv => applyWith_uok hrun hft (.cons (LValTy.flow hv hte) .nil))
            hitems
      | firstWhere hxs hk hte hb =>
        obtain ⟨xv, hxv, hxt⟩ := latom_ltyped henv hxs
        obtain ⟨fv, hfv, hft⟩ := latom_ltyped henv hk
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hfv]
          have hpc : ∀ a b c d : LExpr, p.le pc ((((pc.join a).join b).join c).join d) :=
            fun _ _ _ _ => LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_trans
              (LExpr.le_join_left _ _) (LExpr.le_trans (LExpr.le_join_left _ _)
                (LExpr.le_join_left _ _)))
          refine UOk.mono (bound_allowed hb (hpc _ _ _ _)) (fun v hv => hv)
            (bound_fails hb (hpc _ _ _ _)) ?_
          exact walkFirst_uok _ (fun v hv => applyWith_uok hrun hft (.cons (LValTy.flow hv hte) .nil))
            hitems
      | fold hxs hi hf hte hinit hres hb =>
        obtain ⟨xv, hxv, hxt⟩ := latom_ltyped henv hxs
        obtain ⟨iv, hiv, hit⟩ := latom_ltyped henv hi
        obtain ⟨fv, hfv, hft⟩ := latom_ltyped henv hf
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hiv, hfv]
          refine UOk.mono (bound_allowed hb (LExpr.le_trans (LExpr.le_join_left _ _)
            (LExpr.le_join_left _ _))) (fun v hv => LValTy.raise _ hv)
            (bound_fails hb (LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_left _ _))) ?_
          exact walkFold_uok (fun acc v hacc hv => UOk.mono (fun _ h => h)
            (fun _ h => LValTy.flow h hres) id
            (applyWith_uok hrun hft (.cons hacc (.cons (LValTy.flow hv hte) .nil))))
            (LValTy.flow hit hinit) hitems
      | parse hs ht =>
        obtain ⟨v, hv, hvt⟩ := latom_ltyped henv hs
        cases hvt <;> simp [LTy.erase] at ht
        rename_i s _
        simp only [evalRhs, hv]
        unfold parseInt
        split
        · exact uok_done LValTy.int
        · exact And.intro (fun _ h => nomatch h) (Exists.intro _ ⟨rfl, LExpr.le_join_left _ _⟩)
      | eq ha hb _ _ _ =>
        obtain ⟨va, hva, _⟩ := latom_ltyped henv ha
        obtain ⟨vb, hvb, _⟩ := latom_ltyped henv hb
        simp only [evalRhs, hva, hvb, done]
        exact uok_done LValTy.bool
    · intro env Γ E pc e t ε henv he
      cases he with
      | atom ha =>
        obtain ⟨v, hv, hvt⟩ := latom_ltyped henv ha
        exact ⟨fun _ h => by simp [eval] at h, by simpa [eval, hv, URes] using hvt⟩
      | letR hr hflow hb =>
        simp only [eval]
        refine andThen_uok (UOk.mono allowed_union_left (fun v hv => LValTy.flow hv hflow)
          fails_union_left (ihR henv hr)) (fun v hv => ?_)
        exact UOk.mono (allowed_union_right (LExpr.le_join_left _ _)) (fun _ h => h)
          (fails_union_right (LExpr.le_join_left _ _)) (ihE (.cons hv henv) hb)
      | letE hv hflow hb =>
        simp only [eval]
        refine andThen_uok (UOk.mono allowed_union_left (fun v hv => LValTy.flow hv hflow)
          fails_union_left (ihE henv hv)) (fun v hv => ?_)
        exact UOk.mono (allowed_union_right (LExpr.le_join_left _ _)) (fun _ h => h)
          (fails_union_right (LExpr.le_join_left _ _)) (ihE (.cons hv henv) hb)
      | ite hc ht he h₁ h₂ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltyped henv hc
        cases hvt with
        | @bool b =>
          cases b
          · simp only [eval, hv]
            exact UOk.mono (allowed_union_right (LExpr.le_join_left _ _))
              (fun _ h => LValTy.raise _ (LValTy.flow h h₂))
              (fails_union_right (LExpr.le_join_left _ _)) (ihE henv he)
          · simp only [eval, hv]
            exact UOk.mono (Allowed.mono (LExpr.le_join_left _ _) union_op_left union_get_left)
              (fun _ h => LValTy.raise _ (LValTy.flow h h₁))
              (Fails.mono (LExpr.le_join_left _ _) union_fails_left) (ihE henv ht)
      | caseOption hs hn hsome h₁ h₂ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltyped henv hs
        cases hvt with
        | none =>
          simp only [eval, hv]
          exact UOk.mono (Allowed.mono (LExpr.le_join_left _ _) union_op_left union_get_left)
            (fun _ h => LValTy.raise _ (LValTy.flow h h₁))
            (Fails.mono (LExpr.le_join_left _ _) union_fails_left) (ihE henv hn)
        | some hw =>
          simp only [eval, hv]
          exact UOk.mono (allowed_union_right (LExpr.le_join_left _ _))
            (fun _ h => LValTy.raise _ (LValTy.flow h h₂))
            (fails_union_right (LExpr.le_join_left _ _)) (ihE (.cons (LValTy.raise _ hw) henv) hsome)
      | caseResult hs hok herr h₁ h₂ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltyped henv hs
        cases hvt with
        | ok hw =>
          simp only [eval, hv]
          exact UOk.mono (Allowed.mono (LExpr.le_join_left _ _) union_op_left union_get_left)
            (fun _ h => LValTy.raise _ (LValTy.flow h h₁))
            (Fails.mono (LExpr.le_join_left _ _) union_fails_left) (ihE (.cons (LValTy.raise _ hw) henv) hok)
        | err =>
          simp only [eval, hv]
          exact UOk.mono (allowed_union_right (LExpr.le_join_left _ _))
            (fun _ h => LValTy.raise _ (LValTy.flow h h₂))
            (fails_union_right (LExpr.le_join_left _ _)) (ihE (.cons .int henv) herr)
      | fail hpc _ => exact ⟨fun _ h => by simp [eval] at h, _, rfl, hpc⟩
      | attempt hb =>
        have h := ihE henv hb
        simp only [eval]
        revert h
        cases eval n p w env _ with
        | mk tr r =>
          intro h
          have hev : ∀ ev ∈ tr, Allowed p pc ⟨_, _, Option.none⟩ ev :=
            fun ev hev => Allowed.mono (LExpr.le_refl pc) id id ev (h.1 ev hev)
          cases r with
          | ok v => exact ⟨hev, .ok h.2⟩
          | failed c => exact ⟨hev, .err⟩
          | stuck => exact ⟨hev, trivial⟩
          | outOfFuel => exact ⟨hev, trivial⟩
      | @handle _ _ body x handler _ _ _ ε₁ ε₂ hb hh h₁ h₂ =>
        have h := ihE henv hb
        simp only [eval]
        revert h
        cases eval n p w env _ with
        | mk tr r =>
          intro h
          have hev : ∀ ev ∈ tr, Allowed p pc ⟨ε₁.op || ε₂.op, ε₁.get || ε₂.get, ε₂.fails⟩ ev :=
            fun ev hev => Allowed.mono (LExpr.le_refl pc) (fun h => by simp [h])
              (fun h => by simp [h]) ev (h.1 ev hev)
          cases r with
          | failed code =>
            have hh' := ihE (.cons (LValTy.int (n := (code : Int))) henv) hh
            rcases hrh : eval n p w ((x, Val.int (code : Int)) :: env) handler with ⟨tr', r'⟩
            rw [hrh] at hh'
            simp only [hrh]
            refine ⟨fun ev hev' => ?_, ?_⟩
            · rcases List.mem_append.mp hev' with hev' | hev'
              · exact hev ev hev'
              · exact Allowed.mono (LExpr.le_join_left _ _) (fun h => by simp [h])
                  (fun h => by simp [h]) ev (hh'.1 ev hev')
            · have h2 := hh'.2
              cases r' <;> simp only [URes] at h2 ⊢
              · exact LValTy.raise _ (LValTy.flow h2 h₂)
              · exact Fails.mono (LExpr.le_join_left _ _) (fun l hl => ⟨l, hl, LExpr.le_refl l⟩) h2
          | ok v => exact ⟨hev, LValTy.raise _ (LValTy.flow h.2 h₁)⟩
          | stuck => exact ⟨hev, trivial⟩
          | outOfFuel => exact ⟨hev, trivial⟩

end Una.F1
