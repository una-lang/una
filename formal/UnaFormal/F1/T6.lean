/-
**T6, noninterference, for the generic-free part of F1** (`spec/10 §10`, M3 of
`docs/plan/formal-track.md`): two runs of a body the labeled typing accepts, in environments
related for an observer at `L`, end, when both end, with the same events the observer sees and
related outcomes (`t6`).

Assumptions, stated as hypotheses or in the model:
- the generic-free part: no type parameter and no policy (`Program.GenericFree`); declassification
  and generics are M4's;
- the host's response is labeled at least what its operation accepts (`hget`), which `spec/24 §5`
  meets;
- the host is the same `World` in both runs, a function of the request: its failures depend only
  on the URL, and no operation of the program changes it. S-14 (a file's existence written by an
  internal path and observed by a public read) and S-15 (a server's failures decided by a body)
  are the findings that show real hosts break this; their fix discharges the assumption.
-/
import UnaFormal.F1.BinaryLemmas

namespace Una.F1

variable {p : Program} {L : Label}

theorem latom_both {Γ : LCtx} {e₁ e₂ : Env} {a : Atom} {t : LTy} (he₁ : LEnvTy p e₁ Γ)
    (he₂ : LEnvTy p e₂ Γ) (hrel : EnvRel p L Γ e₁ e₂) (h : LAtomTy Γ a t) :
    ∃ v₁ v₂, evalAtom e₁ a = some v₁ ∧ evalAtom e₂ a = some v₂ ∧ V p L t v₁ v₂ ∧
      LValTy p v₁ t ∧ LValTy p v₂ t := by
  obtain ⟨v₁, v₂, h₁, h₂, hv⟩ := latom_rel hrel h
  obtain ⟨u₁, hu₁, hut₁⟩ := latom_ltyped he₁ h
  obtain ⟨u₂, hu₂, hut₂⟩ := latom_ltyped he₂ h
  rw [h₁] at hu₁
  rw [h₂] at hu₂
  cases hu₁
  cases hu₂
  exact ⟨v₁, v₂, h₁, h₂, hv, hut₁, hut₂⟩

theorem latoms_both {Γ : LCtx} {e₁ e₂ : Env} {as : List Atom} {ts : List LTy}
    (he₁ : LEnvTy p e₁ Γ) (he₂ : LEnvTy p e₂ Γ) (hrel : EnvRel p L Γ e₁ e₂)
    (h : LAtomsTy Γ as ts) :
    ∃ vs₁ vs₂, evalAtoms e₁ as = some vs₁ ∧ evalAtoms e₂ as = some vs₂ ∧ VList p L ts vs₁ vs₂ ∧
      LValsTy p vs₁ ts ∧ LValsTy p vs₂ ts := by
  obtain ⟨v₁, v₂, h₁, h₂, hv⟩ := latoms_rel hrel h
  obtain ⟨u₁, hu₁, hut₁⟩ := latoms_ltyped he₁ h
  obtain ⟨u₂, hu₂, hut₂⟩ := latoms_ltyped he₂ h
  rw [h₁] at hu₁
  rw [h₂] at hu₂
  cases hu₁
  cases hu₂
  exact ⟨v₁, v₂, h₁, h₂, hv, hut₁, hut₂⟩

/-- Two identical runs are related when their outcome is related to itself. -/
theorem brel_same {R : Val → Val → Prop} {vf : Prop} {run : Trace × Res}
    (h : ∀ v, run.2 = .ok v → R v v) : BRel p L R vf run run := by
  intro ht _
  obtain ⟨tr, r⟩ := run
  refine ⟨rfl, ?_⟩
  cases r <;> simp only [Term] at ht <;> simp only [OutRel]
  · exact h _ rfl
  · exact fun _ => trivial

/-- Two runs that show no event, end with values the relation always relates, and fail only
unseen. -/
theorem brel_quiet {R : Val → Val → Prop} {vf : Prop} {r₁ r₂ : Trace × Res}
    (hp : proj p L r₁.1 = proj p L r₂.1) (hR : ∀ a b, R a b)
    (hf₁ : ∀ c, r₁.2 = .failed c → ¬ vf) (hf₂ : ∀ c, r₂.2 = .failed c → ¬ vf) :
    BRel p L R vf r₁ r₂ := by
  intro ht₁ ht₂
  refine ⟨hp, ?_⟩
  obtain ⟨tr₁, o₁⟩ := r₁
  obtain ⟨tr₂, o₂⟩ := r₂
  cases o₁ <;> cases o₂ <;> simp only [Term] at ht₁ ht₂ <;> simp only [OutRel]
  · exact hR _ _
  · exact hf₂ _ rfl
  · exact hf₁ _ rfl
  · exact fun hv => absurd hv (hf₁ _ rfl)

/-- The unary facts of a call of a function value, at the program counter of the call. -/
theorem hof_uok {pc : LExpr} {o g f : Bool} {P : Val → Prop} {run : Trace × Res}
    (hb : p.Bound pc o g f) (h : UOk (FnAllowed o g) P (f = true) run) :
    UOk (Allowed p pc (Eff.ofFn o g f)) P (Fails p pc (Eff.ofFn o g f)) run :=
  UOk.mono (bound_allowed hb (LExpr.le_refl pc)) (fun _ h => h) (bound_fails hb (LExpr.le_refl pc)) h

theorem not_vis_join_left {a b : LExpr} (h : ¬ Vis p L a) : ¬ Vis p L (a.join b) :=
  fun hv => h (Vis.left hv)

theorem not_vis_join_right {a b : LExpr} (h : ¬ Vis p L b) : ¬ Vis p L (a.join b) :=
  fun hv => h (Vis.right hv)

theorem V.ofScalar_refl {t : Ty} {l : LExpr} {u : Val} (h : LValTy p u (ofScalar t l)) :
    V p L (ofScalar t l) u u := by
  unfold ofScalar at h ⊢
  split at h <;> cases h <;> first | exact .bool | exact .int

theorem ofScalar_top (t : Ty) (l : LExpr) : (ofScalar t l).top = l := by
  unfold ofScalar
  split <;> rfl

/-- **T6 for the generic-free part of F1.** -/
theorem t6_sound (hwf : p.LWellFormed) (hgf : p.GenericFree)
    (hget : p.le (.ofLabel p.acceptsGet) (.ofLabel p.getResult)) (w : World) : ∀ n : Nat,
    (∀ {e₁ e₂ : Env} {Γ : LCtx} {E : List String} {pc : LExpr} {r : Rhs} {t : LTy} {ε : Eff},
      LEnvTy p e₁ Γ → LEnvTy p e₂ Γ → EnvRel p L Γ e₁ e₂ → LRhsTy p E Γ pc r t ε →
      BRel p L (V p L t) (Vis p L ε.failLabel) (evalRhs n p w e₁ r) (evalRhs n p w e₂ r)) ∧
    (∀ {e₁ e₂ : Env} {Γ : LCtx} {E : List String} {pc : LExpr} {e : Expr} {t : LTy} {ε : Eff},
      LEnvTy p e₁ Γ → LEnvTy p e₂ Γ → EnvRel p L Γ e₁ e₂ → LExprTy p E Γ pc e t ε →
      BRel p L (V p L t) (Vis p L ε.failLabel) (eval n p w e₁ e) (eval n p w e₂ e))
  | 0 => ⟨fun _ _ _ _ h => by simp [evalRhs, Term] at h,
      fun _ _ _ _ h => by simp [eval, Term] at h⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := t6_sound hwf hgf hget w n
    obtain ⟨uR, uE⟩ := unary_sound hwf hgf w n
    have hrunU : ∀ {env : Env} {Γ : LCtx} {E : List String} {e : Expr} {t : LTy} {ε : Eff},
        LEnvTy p env Γ → LExprTy p E Γ .bot e t ε → UGood p .bot t ε (eval n p w env e) :=
      fun henv he => uE henv he
    have hrunB : ∀ {e₁ e₂ : Env} {Γ : LCtx} {E : List String} {body : Expr} {t : LTy} {ε : Eff},
        LEnvTy p e₁ Γ → LEnvTy p e₂ Γ → EnvRel p L Γ e₁ e₂ → LExprTy p E Γ .bot body t ε →
        BRel p L (V p L t) (Vis p L ε.failLabel) (eval n p w e₁ body) (eval n p w e₂ body) :=
      fun h₁ h₂ hr hb => ihE h₁ h₂ hr hb
    constructor
    · intro e₁ e₂ Γ E pc r t ε he₁ he₂ hrel hr
      cases hr with
      | atom ha =>
        obtain ⟨v₁, v₂, h₁, h₂, hv, _, _⟩ := latom_both he₁ he₂ hrel ha
        simp only [evalRhs, h₁, h₂, Option.elim]
        exact brel_done hv
      | @prim _ _ pr _ ts ty has hp =>
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, hu₁, hu₂⟩ := latoms_both he₁ he₂ hrel has
        simp only [evalRhs, h₁, h₂, Option.bind_some]
        by_cases hv : Vis p L (p.primLabel pr ts)
        · rw [prim_agree hp hvs hv]
          apply brel_same
          intro v hv'
          cases hev : evalPrim pr vs₂ with
          | none => simp [hev] at hv'
          | some u =>
            simp only [hev, Option.elim] at hv'
            cases hv'
            exact V.ofScalar_refl (prim_ltyped (p := p) (p.primLabel pr ts) hp hev)
        · refine brel_quiet rfl (fun a b => .hidden (by rw [ofScalar_top]; exact hv)) ?_ ?_
          · intro c hc; cases h : evalPrim pr vs₁ <;> simp [h] at hc
          · intro c hc; cases h : evalPrim pr vs₂ <;> simp [h] at hc
      | @call _ _ f targs args fn ts hf _ _ _ _ has hfl hb =>
        obtain ⟨⟨tb, εb, hbody, hflow, ho, hg, hfs, hfail⟩, htp⟩ := funOk_of_find hwf hgf hf
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, hu₁, hu₂⟩ := latoms_both he₁ he₂ hrel has
        have hfl' : Flows p.top ts (fn.params.map Prod.snd) := by
          rw [htp, List.zip_nil_left] at hfl
          exact Flows.trans hfl (flows_lsub_nil_params fn).1
        have ha₁ := LValsTy.flows hu₁ hfl'
        have ha₂ := LValsTy.flows hu₂ hfl'
        simp only [evalRhs, hf, h₁, h₂]
        rw [ite_eq_left (by rw [LValsTy.length ha₁, List.length_map]),
          ite_eq_left (by rw [LValsTy.length ha₂, List.length_map])]
        have hb₁ := bindParams_ltyped ha₁ (LEnvTy.nil (p := p))
        have hb₂ := bindParams_ltyped ha₂ (LEnvTy.nil (p := p))
        have hbr := bindParams_rel (VList.flows hvs hfl') (EnvRel.nil (p := p) (L := L))
        rw [List.append_nil] at hb₁ hb₂ hbr
        refine BRel.mono (fun _ _ h => ?_) (fun _ => Vis.of_le hfail Vis.bot)
          (ihE hb₁ hb₂ hbr hbody)
        rw [htp, List.zip_nil_left]
        exact V.flow (V.flow h hflow) (Flow.lsubst_nil _ _).2
      | @apply _ _ f args ps r o g fl lf ts hf has hfl hb =>
        obtain ⟨fv₁, fv₂, hf₁, hf₂, hfv, hft₁, hft₂⟩ := latom_both he₁ he₂ hrel hf
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, hu₁, hu₂⟩ := latoms_both he₁ he₂ hrel has
        simp only [evalRhs, hf₁, hf₂, h₁, h₂]
        by_cases hvis : Vis p L (lf.join (decision (decisionsOf ts)))
        · have hd := Vis.right hvis
          exact BRel.mono (fun _ _ h => V.raise _ h) id
            (applyWith_brel hrunB hfv (Vis.left hvis)
              (VList.unraise hd (VList.flows hvs hfl))
              (LValsTy.unraise (LValsTy.flows hu₁ hfl)) (LValsTy.unraise (LValsTy.flows hu₂ hfl)) _)
        · have hpc : ¬ Vis p L (pc.join (lf.join (decision (decisionsOf ts)))) :=
            not_vis_join_right hvis
          exact brel_confined
            (hof_uok hb (applyWith_uok hrunU hft₁ (LValsTy.unraise (LValsTy.flows hu₁ hfl))))
            (hof_uok hb (applyWith_uok hrunU hft₂ (LValsTy.unraise (LValsTy.flows hu₂ hfl))))
            hpc (fun a b => V.hide a b hvis) id id
      | op hl ht hle _ =>
        obtain ⟨v₁, v₂, h₁, h₂, hv, hu₁, hu₂⟩ := latom_both he₁ he₂ hrel hl
        cases hu₁ <;> simp [LTy.erase] at ht
        cases hu₂
        rename_i s₁ l s₂
        simp only [evalRhs, h₁, h₂]
        by_cases hd : Vis p L l
        · have hs : s₁ = s₂ := by have := V.eq_text hv hd; injection this
          subst hs
          exact brel_same (fun _ h => by cases h; exact .unit)
        · have hacc : ¬ EvVis p L (.out s₁) := fun hv' => hd (Vis.of_le (LExpr.le_of_join_right hle) hv')
          have hacc₂ : ¬ EvVis p L (.out s₂) := hacc
          exact brel_quiet (by simp only [proj_single_hidden hacc, proj_single_hidden hacc₂])
            (fun _ _ => .unit) (fun _ h => nomatch h) (fun _ h => nomatch h)
      | get hu ht hle _ =>
        obtain ⟨v₁, v₂, h₁, h₂, hv, hu₁, hu₂⟩ := latom_both he₁ he₂ hrel hu
        cases hu₁ <;> simp [LTy.erase] at ht
        cases hu₂
        rename_i s₁ l s₂
        simp only [evalRhs, h₁, h₂]
        by_cases hd : Vis p L l
        · have hs : s₁ = s₂ := by have := V.eq_text hv hd; injection this
          subst hs
          refine brel_same (fun v hv' => ?_)
          split at hv' <;> simp at hv'
          subst hv'
          exact .text
        · have hacc : ¬ Vis p L (.ofLabel p.acceptsGet) :=
            fun hv' => hd (Vis.of_le (LExpr.le_of_join_right hle) hv')
          have hres : ¬ Vis p L (.ofLabel p.getResult) := fun hv' => hacc (Vis.of_le hget hv')
          have hvf : ¬ Vis p L (Eff.failLabel ⟨false, true, some (pc.join (p.deep (.text l)))⟩) :=
            fun hv' => hd (Vis.right hv')
          refine brel_quiet ?_ (fun _ _ => .hidden hres) ?_ ?_
          · cases w.get s₁ <;> cases w.get s₂ <;>
              simp only [proj_single_hidden (ev := .get s₁) hacc, proj_single_hidden (ev := .get s₂) hacc]
          · intro c hc; exact hvf
          · intro c hc; exact hvf
      | construct hrec has hfl =>
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, _, _⟩ := latoms_both he₁ he₂ hrel has
        simp only [evalRhs, h₁, h₂, Option.map, Option.elim]
        exact brel_done (.record hrec (VList.flows hvs hfl))
      | field hv hrec hi =>
        obtain ⟨v₁, v₂, h₁, h₂, hvv, hu₁, hu₂⟩ := latom_both he₁ he₂ hrel hv
        cases hu₁ with
        | record hr₁ hfs₁ =>
          cases hu₂ with
          | record hr₂ hfs₂ =>
            rw [hrec] at hr₁ hr₂
            cases hr₁
            cases hr₂
            obtain ⟨a, ha, _⟩ := lfield_typed hfs₁ hi
            obtain ⟨b, hb', _⟩ := lfield_typed hfs₂ hi
            simp only [evalRhs, h₁, h₂, ha, hb', Option.elim]
            refine brel_done ?_
            cases hvv with
            | hidden h => exact V.hide _ _ h
            | record hr hrel' =>
              rw [hrec] at hr
              cases hr
              obtain ⟨a', b', ha', hb'', hab⟩ := VList.get hrel' hi
              rw [ha] at ha'
              rw [hb'] at hb''
              cases ha'
              cases hb''
              exact V.raise _ hab
      | list has hfl =>
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, _, _⟩ := latoms_both he₁ he₂ hrel has
        simp only [evalRhs, h₁, h₂, Option.map, Option.elim]
        exact brel_done (.list (VAll.of_flows hvs hfl))
      | none => exact brel_done .none
      | some ha =>
        obtain ⟨v₁, v₂, h₁, h₂, hv, _, _⟩ := latom_both he₁ he₂ hrel ha
        simp only [evalRhs, h₁, h₂, Option.map, Option.elim]
        exact brel_done (.some hv)
      | lambda hb hfl =>
        exact brel_done (.closure hrel he₁ he₂ hb hfl (Flows.refl _) (Flow.refl _) id id id)
      | @funRef _ _ f targs fn hf _ _ _ _ =>
        obtain ⟨⟨tb, εb, hbody, hflow, ho, hg, hfs, hfail⟩, htp⟩ := funOk_of_find hwf hgf hf
        simp only [evalRhs, hf]
        refine brel_done ?_
        rw [htp, List.zip_nil_left]
        exact .closure .nil .nil .nil (by simpa using hbody) hfail (flows_lsub_nil_params fn).1
          (Flow.trans hflow (Flow.lsubst_nil _ _).2) ho hg hfs
      | declassify hpo =>
        have := policy_mem hpo
        rw [hgf.policies] at this
        nomatch this
      | @map _ _ xs f te lx tp tr o g fl lf hxs hf hte hb =>
        obtain ⟨x₁, x₂, hx₁, hx₂, hxv, hxu₁, hxu₂⟩ := latom_both he₁ he₂ hrel hxs
        obtain ⟨f₁, f₂, hf₁, hf₂, hfv, hfu₁, hfu₂⟩ := latom_both he₁ he₂ hrel hf
        cases hxu₁ with
        | list hi₁ =>
          cases hxu₂ with
          | list hi₂ =>
            simp only [evalRhs, hx₁, hx₂, hf₁, hf₂]
            by_cases hvis : Vis p L (lx.join lf)
            · cases hxv with
              | hidden h => exact absurd (Vis.left hvis) h
              | list hall =>
                refine BRel.mono (fun a b h => ?_) (fun _ => trivial)
                  (walkMap_brel (fun x y hxy hx hy => applyWith_brel hrunB hfv (Vis.right hvis)
                    (.cons (V.flow hxy hte) .nil) (.cons (LValTy.flow hx hte) .nil)
                    (.cons (LValTy.flow hy hte) .nil) True) hall hi₁ hi₂)
                obtain ⟨zs₁, zs₂, rfl, rfl, hz⟩ := h
                exact .list (VAll.flow hz (Flow.raise _ _))
            · have hpc : ¬ Vis p L ((pc.join lx).join lf) := fun h =>
                hvis (Vis.join (Vis.right (Vis.left h)) (Vis.right h))
              refine brel_confined (hof_uok (P := fun v => LValTy p v (.list (tr.raise lf) (lx.join lf))) hb ?_)
                (hof_uok (P := fun v => LValTy p v (.list (tr.raise lf) (lx.join lf))) hb ?_) hpc
                (fun a b => .hidden (by simpa [LTy.top] using hvis)) id id
              · exact walkMap_uok _ (fun v hv => UOk.mono (fun _ h => h) (fun _ h => LValTy.raise _ h) id
                  (applyWith_uok hrunU hfu₁ (.cons (LValTy.flow hv hte) .nil))) hi₁
              · exact walkMap_uok _ (fun v hv => UOk.mono (fun _ h => h) (fun _ h => LValTy.raise _ h) id
                  (applyWith_uok hrunU hfu₂ (.cons (LValTy.flow hv hte) .nil))) hi₂
      | @filter _ _ xs keep te lx tp lb o g fl lf hxs hk hte hb =>
        obtain ⟨x₁, x₂, hx₁, hx₂, hxv, hxu₁, hxu₂⟩ := latom_both he₁ he₂ hrel hxs
        obtain ⟨f₁, f₂, hf₁, hf₂, hfv, hfu₁, hfu₂⟩ := latom_both he₁ he₂ hrel hk
        cases hxu₁ with
        | list hi₁ =>
          cases hxu₂ with
          | list hi₂ =>
            simp only [evalRhs, hx₁, hx₂, hf₁, hf₂]
            by_cases hvis : Vis p L (lx.join lf)
            · cases hxv with
              | hidden h => exact absurd (Vis.left hvis) h
              | list hall =>
                refine BRel.mono (fun a b h => ?_) (fun _ => trivial)
                  (walkFilter_brel (fun x y hxy hx hy => applyWith_brel hrunB hfv (Vis.right hvis)
                    (.cons (V.flow hxy hte) .nil) (.cons (LValTy.flow hx hte) .nil)
                    (.cons (LValTy.flow hy hte) .nil) True) hall hi₁ hi₂)
                by_cases hl : Vis p L ((lx.join lf).join lb)
                · obtain ⟨zs₁, zs₂, rfl, rfl, hz⟩ := h (Vis.right hl)
                  exact .list hz
                · exact .hidden (by simpa [LTy.top] using hl)
            · have hpc : ¬ Vis p L ((pc.join lx).join lf) := fun h =>
                hvis (Vis.join (Vis.right (Vis.left h)) (Vis.right h))
              refine brel_confined (hof_uok (P := fun v => LValTy p v (.list te ((lx.join lf).join lb))) hb ?_)
                (hof_uok (P := fun v => LValTy p v (.list te ((lx.join lf).join lb))) hb ?_) hpc
                (fun a b => .hidden (by simp only [LTy.top]; exact not_vis_join_left hvis)) id id
              · exact walkFilter_uok _ (fun v hv =>
                  applyWith_uok hrunU hfu₁ (.cons (LValTy.flow hv hte) .nil)) hi₁
              · exact walkFilter_uok _ (fun v hv =>
                  applyWith_uok hrunU hfu₂ (.cons (LValTy.flow hv hte) .nil)) hi₂
      | @firstWhere _ _ xs keep te lx tp lb o g fl lf hxs hk hte hb =>
        obtain ⟨x₁, x₂, hx₁, hx₂, hxv, hxu₁, hxu₂⟩ := latom_both he₁ he₂ hrel hxs
        obtain ⟨f₁, f₂, hf₁, hf₂, hfv, hfu₁, hfu₂⟩ := latom_both he₁ he₂ hrel hk
        cases hxu₁ with
        | list hi₁ =>
          cases hxu₂ with
          | list hi₂ =>
            simp only [evalRhs, hx₁, hx₂, hf₁, hf₂]
            by_cases hvis : Vis p L ((lx.join lf).join lb)
            · cases hxv with
              | hidden h => exact absurd (Vis.left (Vis.left hvis)) h
              | list hall =>
                refine BRel.mono (fun a b h => ?_) (fun _ => trivial)
                  (walkFirst_brel (Vis.right hvis) (fun x y hxy hx hy => applyWith_brel hrunB hfv
                    (Vis.right (Vis.left hvis)) (.cons (V.flow hxy hte) .nil)
                    (.cons (LValTy.flow hx hte) .nil) (.cons (LValTy.flow hy hte) .nil) True)
                    hall hi₁ hi₂)
                rcases h with ⟨rfl, rfl⟩ | ⟨x, y, rfl, rfl, hxy⟩
                · exact .none
                · exact .some hxy
            · have hpc : ¬ Vis p L ((((pc.join lx).join lf).join (p.deep te)).join lb) := fun h =>
                hvis (Vis.join (Vis.join (Vis.right (Vis.left (Vis.left (Vis.left h))))
                  (Vis.right (Vis.left (Vis.left h)))) (Vis.right h))
              refine brel_confined (hof_uok (P := fun v => LValTy p v (.option te ((lx.join lf).join lb))) hb ?_)
                (hof_uok (P := fun v => LValTy p v (.option te ((lx.join lf).join lb))) hb ?_) hpc
                (fun a b => .hidden (by simpa [LTy.top] using hvis)) id id
              · exact walkFirst_uok _ (fun v hv =>
                  applyWith_uok hrunU hfu₁ (.cons (LValTy.flow hv hte) .nil)) hi₁
              · exact walkFirst_uok _ (fun v hv =>
                  applyWith_uok hrunU hfu₂ (.cons (LValTy.flow hv hte) .nil)) hi₂
      | @fold _ _ xs init f te lx ti ta tp tr o g fl lf hxs hi hf hte hinit hres hb =>
        obtain ⟨x₁, x₂, hx₁, hx₂, hxv, hxu₁, hxu₂⟩ := latom_both he₁ he₂ hrel hxs
        obtain ⟨i₁, i₂, hi₁', hi₂', hiv, hiu₁, hiu₂⟩ := latom_both he₁ he₂ hrel hi
        obtain ⟨f₁, f₂, hf₁, hf₂, hfv, hfu₁, hfu₂⟩ := latom_both he₁ he₂ hrel hf
        cases hxu₁ with
        | list hl₁ =>
          cases hxu₂ with
          | list hl₂ =>
            simp only [evalRhs, hx₁, hx₂, hi₁', hi₂', hf₁, hf₂]
            have happU : ∀ (fv : Val), LValTy p fv (.fn [ta, tp] tr o g fl lf) →
                ∀ c x, LValTy p c ta → LValTy p x te →
                  UOk (FnAllowed o g) (fun r => LValTy p r ta) (fl = true)
                    (applyWith (eval n p w) fv [c, x]) := fun fv hfu c x hc hx =>
              UOk.mono (fun _ h => h) (fun _ h => LValTy.flow h hres) id
                (applyWith_uok hrunU hfu (.cons hc (.cons (LValTy.flow hx hte) .nil)))
            by_cases hvis : Vis p L (lx.join lf)
            · cases hxv with
              | hidden h => exact absurd (Vis.left hvis) h
              | list hall =>
                refine BRel.mono (fun a b h => V.raise _ h) (fun _ => trivial)
                  (walkFold_brel (fun c₁ c₂ x y hc hc₁ hc₂ hxy hx hy =>
                    BRel.mono (fun _ _ h => V.flow h hres) id
                      (applyWith_brel hrunB hfv (Vis.right hvis) (.cons hc (.cons (V.flow hxy hte) .nil))
                        (.cons hc₁ (.cons (LValTy.flow hx hte) .nil))
                        (.cons hc₂ (.cons (LValTy.flow hy hte) .nil)) True))
                    (happU _ hfu₁) (happU _ hfu₂) (V.flow hiv hinit) (LValTy.flow hiu₁ hinit)
                    (LValTy.flow hiu₂ hinit) hall hl₁ hl₂)
            · have hpc : ¬ Vis p L ((pc.join lx).join lf) := fun h =>
                hvis (Vis.join (Vis.right (Vis.left h)) (Vis.right h))
              refine brel_confined (hof_uok (P := fun v => LValTy p v (ta.raise (lx.join lf))) hb ?_)
                (hof_uok (P := fun v => LValTy p v (ta.raise (lx.join lf))) hb ?_) hpc
                (fun a b => V.hide a b hvis) id id
              · exact UOk.mono (fun _ h => h) (fun _ h => LValTy.raise _ h) id
                  (walkFold_uok (happU _ hfu₁) (LValTy.flow hiu₁ hinit) hl₁)
              · exact UOk.mono (fun _ h => h) (fun _ h => LValTy.raise _ h) id
                  (walkFold_uok (happU _ hfu₂) (LValTy.flow hiu₂ hinit) hl₂)
      | parse hs ht =>
        obtain ⟨v₁, v₂, h₁, h₂, hv, hu₁, hu₂⟩ := latom_both he₁ he₂ hrel hs
        cases hu₁ <;> simp [LTy.erase] at ht
        cases hu₂
        rename_i s₁ l s₂
        simp only [evalRhs, h₁, h₂]
        by_cases hd : Vis p L l
        · have hs : s₁ = s₂ := by have := V.eq_text hv hd; injection this
          subst hs
          refine brel_same (fun v hv' => ?_)
          unfold parseInt at hv'
          split at hv' <;> simp [done] at hv'
          subst hv'
          exact .int
        · have hvf : ¬ Vis p L (Eff.failLabel ⟨false, false, some (pc.join (p.deep (.text l)))⟩) :=
            fun hv' => hd (Vis.right hv')
          refine brel_quiet ?_ (fun _ _ => .hidden hd) (fun _ _ => hvf) (fun _ _ => hvf)
          unfold parseInt
          split <;> split <;> rfl
      | @eq _ _ a b t₁ t₂ tt ha hb hfa hfb heq =>
        obtain ⟨a₁, a₂, ha₁, ha₂, hav, hau₁, hau₂⟩ := latom_both he₁ he₂ hrel ha
        obtain ⟨b₁, b₂, hb₁, hb₂, hbv, hbu₁, hbu₂⟩ := latom_both he₁ he₂ hrel hb
        simp only [evalRhs, ha₁, ha₂, hb₁, hb₂]
        refine brel_done ?_
        by_cases hd : Vis p L ((p.deep t₁).join (p.deep t₂))
        · rw [V.eq_of_deep hwf hav hau₁ hau₂ (by rw [eqIn_flow hfa]; exact heq) (Vis.left hd),
            V.eq_of_deep hwf hbv hbu₁ hbu₂ (by rw [eqIn_flow hfb]; exact heq) (Vis.right hd)]
          exact .bool
        · exact .hidden hd
    · intro e₁ e₂ Γ E pc e t ε he₁ he₂ hrel he
      cases he with
      | atom ha =>
        obtain ⟨v₁, v₂, h₁, h₂, hv, _, _⟩ := latom_both he₁ he₂ hrel ha
        simp only [eval, h₁, h₂, Option.elim]
        exact brel_done hv
      | @letR _ _ x ann value body ty s ε₁ ε₂ hr hflow hb =>
        simp only [eval]
        have u₁ := uR he₁ hr
        have u₂ := uR he₂ hr
        refine andThen_brel (ihR he₁ he₂ hrel hr) Vis.union_left (fun v₁ v₂ k₁ k₂ hv => ?_) ?_ ?_
        · have hv₁ : LValTy p v₁ ty := by have := u₁.2; rw [k₁] at this; exact this
          have hv₂ : LValTy p v₂ ty := by have := u₂.2; rw [k₂] at this; exact this
          exact BRel.mono (fun _ _ h => h) Vis.union_right
            (ihE (.cons (LValTy.flow hv₁ hflow) he₁) (.cons (LValTy.flow hv₂ hflow) he₂)
              (.cons (V.flow hv hflow) hrel) hb)
        · intro hn v k
          have hv : LValTy p v ty := by have := u₁.2; rw [k] at this; exact this
          obtain ⟨hp, hf⟩ := confined (L := L) (uE (.cons (LValTy.flow hv hflow) he₁) hb)
            (not_vis_join_right hn)
          exact ⟨hp, fun c hc hvf => hf c hc (Vis.union_right hvf)⟩
        · intro hn v k
          have hv : LValTy p v ty := by have := u₂.2; rw [k] at this; exact this
          obtain ⟨hp, hf⟩ := confined (L := L) (uE (.cons (LValTy.flow hv hflow) he₂) hb)
            (not_vis_join_right hn)
          exact ⟨hp, fun c hc hvf => hf c hc (Vis.union_right hvf)⟩
      | @letE _ _ x ann value body ty s ε₁ ε₂ hr hflow hb =>
        simp only [eval]
        have u₁ := uE he₁ hr
        have u₂ := uE he₂ hr
        refine andThen_brel (ihE he₁ he₂ hrel hr) Vis.union_left (fun v₁ v₂ k₁ k₂ hv => ?_) ?_ ?_
        · have hv₁ : LValTy p v₁ ty := by have := u₁.2; rw [k₁] at this; exact this
          have hv₂ : LValTy p v₂ ty := by have := u₂.2; rw [k₂] at this; exact this
          exact BRel.mono (fun _ _ h => h) Vis.union_right
            (ihE (.cons (LValTy.flow hv₁ hflow) he₁) (.cons (LValTy.flow hv₂ hflow) he₂)
              (.cons (V.flow hv hflow) hrel) hb)
        · intro hn v k
          have hv : LValTy p v ty := by have := u₁.2; rw [k] at this; exact this
          obtain ⟨hp, hf⟩ := confined (L := L) (uE (.cons (LValTy.flow hv hflow) he₁) hb)
            (not_vis_join_right hn)
          exact ⟨hp, fun c hc hvf => hf c hc (Vis.union_right hvf)⟩
        · intro hn v k
          have hv : LValTy p v ty := by have := u₂.2; rw [k] at this; exact this
          obtain ⟨hp, hf⟩ := confined (L := L) (uE (.cons (LValTy.flow hv hflow) he₂) hb)
            (not_vis_join_right hn)
          exact ⟨hp, fun c hc hvf => hf c hc (Vis.union_right hvf)⟩
      | @ite _ _ cond thenE elseE lc t₁ t₂ s ε₁ ε₂ hc ht hel h₁ h₂ =>
        obtain ⟨c₁, c₂, hc₁, hc₂, hcv, hcu₁, hcu₂⟩ := latom_both he₁ he₂ hrel hc
        have U : ∀ {env : Env} {v : Val}, LEnvTy p env Γ → evalAtom env cond = some v →
            LValTy p v (.bool lc) →
            UOk (Allowed p (pc.join lc) (ε₁.union ε₂)) (fun _ => True)
              (Fails p (pc.join lc) (ε₁.union ε₂)) (eval (n + 1) p w env (.ite cond thenE elseE)) := by
          intro env v henv hcv' hvt
          cases hvt with
          | @bool b _ =>
            cases b
            · simp only [eval, hcv']
              exact UOk.mono (allowed_union_right (LExpr.le_refl _)) (fun _ _ => trivial)
                (fails_union_right (LExpr.le_refl _)) (uE henv hel)
            · simp only [eval, hcv']
              exact UOk.mono allowed_union_left (fun _ _ => trivial) fails_union_left (uE henv ht)
        by_cases hv : Vis p L lc
        · cases hcu₁ with
          | @bool b _ =>
            have := V.eq_bool hcv hv
            subst this
            cases b
            · simp only [eval, hc₁, hc₂]
              exact BRel.mono (fun _ _ h => V.raise _ (V.flow h h₂)) Vis.union_right
                (ihE he₁ he₂ hrel hel)
            · simp only [eval, hc₁, hc₂]
              exact BRel.mono (fun _ _ h => V.raise _ (V.flow h h₁)) Vis.union_left
                (ihE he₁ he₂ hrel ht)
        · exact brel_confined (U he₁ hc₁ hcu₁) (U he₂ hc₂ hcu₂) (not_vis_join_right hv)
            (fun a b => V.hide a b hv) id id
      | @caseOption _ _ scr noneE x someE te ls t₁ t₂ s ε₁ ε₂ hs hn hsome h₁ h₂ =>
        obtain ⟨c₁, c₂, hc₁, hc₂, hcv, hcu₁, hcu₂⟩ := latom_both he₁ he₂ hrel hs
        have U : ∀ {env : Env} {v : Val}, LEnvTy p env Γ → evalAtom env scr = some v →
            LValTy p v (.option te ls) →
            UOk (Allowed p (pc.join ls) (ε₁.union ε₂)) (fun _ => True)
              (Fails p (pc.join ls) (ε₁.union ε₂))
              (eval (n + 1) p w env (.caseOption scr noneE x someE)) := by
          intro env v henv hcv' hvt
          cases hvt with
          | none =>
            simp only [eval, hcv']
            exact UOk.mono allowed_union_left (fun _ _ => trivial) fails_union_left (uE henv hn)
          | some hw =>
            simp only [eval, hcv']
            exact UOk.mono (allowed_union_right (LExpr.le_refl _)) (fun _ _ => trivial)
              (fails_union_right (LExpr.le_refl _)) (uE (.cons (LValTy.raise _ hw) henv) hsome)
        by_cases hv : Vis p L ls
        · cases hcv with
          | hidden h => exact absurd hv h
          | none =>
            simp only [eval, hc₁, hc₂]
            exact BRel.mono (fun _ _ h => V.raise _ (V.flow h h₁)) Vis.union_left (ihE he₁ he₂ hrel hn)
          | some hab =>
            cases hcu₁ with
            | some hw₁ =>
              cases hcu₂ with
              | some hw₂ =>
                simp only [eval, hc₁, hc₂]
                exact BRel.mono (fun _ _ h => V.raise _ (V.flow h h₂)) Vis.union_right
                  (ihE (.cons (LValTy.raise _ hw₁) he₁) (.cons (LValTy.raise _ hw₂) he₂)
                    (.cons (V.raise _ hab) hrel) hsome)
        · exact brel_confined (U he₁ hc₁ hcu₁) (U he₂ hc₂ hcu₂) (not_vis_join_right hv)
            (fun a b => V.hide a b hv) id id
      | @caseResult _ _ scr x okE y errE te ls t₁ t₂ s ε₁ ε₂ hs hok herr h₁ h₂ =>
        obtain ⟨c₁, c₂, hc₁, hc₂, hcv, hcu₁, hcu₂⟩ := latom_both he₁ he₂ hrel hs
        have U : ∀ {env : Env} {v : Val}, LEnvTy p env Γ → evalAtom env scr = some v →
            LValTy p v (.result te ls) →
            UOk (Allowed p (pc.join ls) (ε₁.union ε₂)) (fun _ => True)
              (Fails p (pc.join ls) (ε₁.union ε₂))
              (eval (n + 1) p w env (.caseResult scr x okE y errE)) := by
          intro env v henv hcv' hvt
          cases hvt with
          | ok hw =>
            simp only [eval, hcv']
            exact UOk.mono allowed_union_left (fun _ _ => trivial) fails_union_left
              (uE (.cons (LValTy.raise _ hw) henv) hok)
          | err =>
            simp only [eval, hcv']
            exact UOk.mono (allowed_union_right (LExpr.le_refl _)) (fun _ _ => trivial)
              (fails_union_right (LExpr.le_refl _)) (uE (.cons .int henv) herr)
        by_cases hv : Vis p L ls
        · cases hcv with
          | hidden h => exact absurd hv h
          | ok hab =>
            cases hcu₁ with
            | ok hw₁ =>
              cases hcu₂ with
              | ok hw₂ =>
                simp only [eval, hc₁, hc₂]
                exact BRel.mono (fun _ _ h => V.raise _ (V.flow h h₁)) Vis.union_left
                  (ihE (.cons (LValTy.raise _ hw₁) he₁) (.cons (LValTy.raise _ hw₂) he₂)
                    (.cons (V.raise _ hab) hrel) hok)
          | err =>
            simp only [eval, hc₁, hc₂]
            exact BRel.mono (fun _ _ h => V.raise _ (V.flow h h₂)) Vis.union_right
              (ihE (.cons .int he₁) (.cons .int he₂) (.cons .int hrel) herr)
        · exact brel_confined (U he₁ hc₁ hcu₁) (U he₂ hc₂ hcu₂) (not_vis_join_right hv)
            (fun a b => V.hide a b hv) id id
      | fail _ _ =>
        simp only [eval]
        exact brel_same (fun _ h => nomatch h)
      | @attempt _ _ body ty ε₀ hb =>
        have h := ihE he₁ he₂ hrel hb
        simp only [eval]
        generalize eval n p w e₁ body = r₁ at h ⊢
        generalize eval n p w e₂ body = r₂ at h ⊢
        obtain ⟨tr₁, o₁⟩ := r₁
        obtain ⟨tr₂, o₂⟩ := r₂
        intro ht₁ ht₂
        cases o₁ <;> cases o₂ <;> simp only [Term] at ht₁ ht₂ <;>
          obtain ⟨hp, ho⟩ := h trivial trivial <;> refine ⟨hp, ?_⟩ <;> simp only [OutRel] at ho ⊢
        · exact .ok ho
        · exact .hidden ho
        · exact .hidden ho
        · rename_i c₁ c₂
          by_cases hv : Vis p L ε₀.failLabel
          · rw [ho hv]; exact .err
          · exact .hidden hv
      | @handle _ _ body x handler t₁ t₂ s ε₁ ε₂ hb hh h₁ h₂ =>
        have h := ihE he₁ he₂ hrel hb
        have u₁ := uE he₁ hb
        have u₂ := uE he₂ hb
        simp only [eval]
        generalize eval n p w e₁ body = r₁ at h u₁ ⊢
        generalize eval n p w e₂ body = r₂ at h u₂ ⊢
        obtain ⟨tr₁, o₁⟩ := r₁
        obtain ⟨tr₂, o₂⟩ := r₂
        cases o₁ <;> cases o₂
        all_goals first
          | (intro ht₁ ht₂; simp only [Term] at ht₁ ht₂; done)
          | skip
        · -- both values
          intro ht₁ ht₂
          obtain ⟨hp, ho⟩ := h trivial trivial
          exact ⟨hp, V.raise _ (V.flow ho h₁)⟩
        · -- a value, then a failure the handler takes
          rename_i a c
          dsimp only
          obtain ⟨hp, ho⟩ := h trivial trivial
          simp only [OutRel] at ho
          have uh := uE (.cons (LValTy.int (n := (c : Int))) he₂) hh
          generalize eval n p w ((x, Val.int (c : Int)) :: e₂) handler = rh at uh ⊢
          obtain ⟨th, oh⟩ := rh
          obtain ⟨hph, hfh⟩ := confined (L := L) uh (not_vis_join_right ho)
          refine brel_quiet (by simp only [proj_append]; rw [hph, List.append_nil]; exact hp)
            (fun a b => V.hide a b ho) (fun _ h => nomatch h) (fun c' hc' => hfh c' hc')
        · -- a failure the handler takes, then a value
          rename_i c b
          dsimp only
          obtain ⟨hp, ho⟩ := h trivial trivial
          simp only [OutRel] at ho
          have uh := uE (.cons (LValTy.int (n := (c : Int))) he₁) hh
          generalize eval n p w ((x, Val.int (c : Int)) :: e₁) handler = rh at uh ⊢
          obtain ⟨th, oh⟩ := rh
          obtain ⟨hph, hfh⟩ := confined (L := L) uh (not_vis_join_right ho)
          refine brel_quiet (by simp only [proj_append]; rw [hph, List.append_nil]; exact hp)
            (fun a b => V.hide a b ho) (fun c' hc' => hfh c' hc') (fun _ h => nomatch h)
        · -- both failures
          rename_i c₁ c₂
          dsimp only
          obtain ⟨hp, ho⟩ := h trivial trivial
          simp only [OutRel] at ho
          have hcv : V p L (.int ε₁.failLabel) (.int (c₁ : Int)) (.int (c₂ : Int)) := by
            by_cases hv : Vis p L ε₁.failLabel
            · rw [ho hv]; exact .int
            · exact .hidden hv
          have hh' := ihE (.cons (LValTy.int (n := (c₁ : Int))) he₁)
            (.cons (LValTy.int (n := (c₂ : Int))) he₂) (.cons hcv hrel) hh
          generalize eval n p w ((x, Val.int (c₁ : Int)) :: e₁) handler = rh₁ at hh' ⊢
          generalize eval n p w ((x, Val.int (c₂ : Int)) :: e₂) handler = rh₂ at hh' ⊢
          obtain ⟨th₁, oh₁⟩ := rh₁
          obtain ⟨th₂, oh₂⟩ := rh₂
          intro ht₁ ht₂
          obtain ⟨hph, hoh⟩ := hh' ht₁ ht₂
          exact ⟨by simp only [proj_append]; rw [hp, hph],
            OutRel.mono (fun _ _ h => V.raise _ (V.flow h h₂)) id hoh⟩

end Una.F1

namespace Una.F1

variable {p : Program} {L : Label}

/-- **T6, noninterference, for the generic-free part of F1.** Two runs of a body the labeled
typing accepts, at any fuel, in environments of its context's labeled types that the observer at
`L` cannot tell apart, show that observer the same events and related outcomes whenever both
end: equal values at every position it sees, the same failure when it sees what decides
failure, and one failing while the other ends with a value only when it does not. -/
theorem t6 (hwf : p.LWellFormed) (hgf : p.GenericFree)
    (hget : p.le (.ofLabel p.acceptsGet) (.ofLabel p.getResult)) (w : World)
    {e₁ e₂ : Env} {Γ : LCtx} {E : List String} {pc : LExpr} {e : Expr} {t : LTy} {ε : Eff}
    (h₁ : LEnvTy p e₁ Γ) (h₂ : LEnvTy p e₂ Γ) (hrel : EnvRel p L Γ e₁ e₂)
    (he : LExprTy p E Γ pc e t ε) (n : Nat) :
    Term (eval n p w e₁ e).2 → Term (eval n p w e₂ e).2 →
      proj p L (eval n p w e₁ e).1 = proj p L (eval n p w e₂ e).1 ∧
        OutRel (V p L t) (Vis p L ε.failLabel) (eval n p w e₁ e).2 (eval n p w e₂ e).2 :=
  (t6_sound hwf hgf hget w n).2 h₁ h₂ hrel he

end Una.F1
