/-
**T6, noninterference, for F1** (`spec/10 §10`, M3 and M4 of `docs/plan/formal-track.md`):
two runs of a body the labeled typing accepts, in environments related for an observer at `L`,
end, when both end at the same fuel, with the same events the observer sees and related outcomes.
`t6` holds at every ground substitution `σ` of the type parameters in scope, so a generic body
is covered at each of its ground instances; `t6_ground` is the case of a body with no type
parameter in scope, every call site and `main`; `t6_ended` (`Mono.lean`) drops the common fuel.

**Scope.** What the theorem covers is F1 as `Syntax.lean` defines it, and no more:
- one capability with one operation (`op`) and one host operation (`get`), each with one
  `accepts` label for the whole program;
- a stateless host, the same `World` in both runs, whose failures depend only on the URL and which
  no operation of the program changes: S-14 (a file's existence written through an internal path
  and seen by a public read) and S-15 (a server's failures decided by a body) show that real
  hosts break this, and their fix discharges the assumption;
- failures are codes without payloads;
- no sums but `Option` and `Result`, no `Map`, no `with` or handlers of capabilities, no
  `together`;
- generic functions at ground instances only: the type arguments of a call are ground (the
  premise `LTy.GroundAll` of T-Call and of a function reference), so a generic body that calls a
  generic function at its own type parameters, two levels of generics, is outside the model;
- no policy (`p.policies = []`): declassification is `t6_release` (`Release.lean`, option B1),
  which `t6_soundR` proves for programs within B1's restrictions, up to the first visible release
  whose values differ; `t6_sound` and `t6` are its case without policies;
- the host's response is labeled at least what its operation accepts (`hget`), which
  `spec/24 §5` meets and `crates/una-check/tests/host_table.rs` checks of the prelude.

**What "related" means** (`V`): at a position the observer sees, scalars and texts are equal,
lists have the same length and related elements, options and results the same tag and related
contents, records related fields; closures are related, not equal (the same code and related
captured environments); a `Unit` position relates any two values, since it carries nothing.
-/
import UnaFormal.F1H.BinaryInst
import UnaFormal.F1H.BinaryR

namespace Una.F1H

open Una.F1 (Label LExpr)

variable {p : Program} {L : Label} {H : Host}

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

/-- Two runs with the same events and outcome, from worlds the observer cannot tell apart, are
related when their outcome is related to itself. -/
theorem brel_same {R : Val → Val → Prop} {vf : Prop} {tr : Trace} {r : Res} {w₁ w₂ : World H}
    (hw : WorldRel p L w₁ w₂) (h : ∀ v, r = .ok v → R v v) : BRel p L R vf (tr, r, w₁) (tr, r, w₂) := by
  intro ht _
  refine ⟨rfl, ?_, hw⟩
  cases r <;> simp only [Term] at ht <;> simp only [OutRel]
  · exact h _ rfl
  · exact fun _ => trivial

/-- Two runs that show the observer the same events, end with values the relation always
relates, fail only unseen, and leave worlds it cannot tell apart. -/
theorem brel_quiet {R : Val → Val → Prop} {vf : Prop} {r₁ r₂ : Run H}
    (hw : WorldRel p L r₁.2.2 r₂.2.2)
    (hp : proj p L r₁.1 = proj p L r₂.1) (hR : ∀ a b, R a b)
    (hf₁ : ∀ c, r₁.2.1 = .failed c → ¬ vf) (hf₂ : ∀ c, r₂.2.1 = .failed c → ¬ vf) :
    BRel p L R vf r₁ r₂ := by
  intro ht₁ ht₂
  refine ⟨hp, ?_, hw⟩
  obtain ⟨tr₁, o₁, _⟩ := r₁
  obtain ⟨tr₂, o₂, _⟩ := r₂
  cases o₁ <;> cases o₂ <;> simp only [Term] at ht₁ ht₂ <;> simp only [OutRel]
  · exact hR _ _
  · exact hf₂ _ rfl
  · exact hf₁ _ rfl
  · exact fun hv => absurd hv (hf₁ _ rfl)

/-- The unary facts of a call of a function value, at the program counter of the call. -/
theorem hof_uok {pc : LExpr} {o : Bool} {g : HostFx} {f : Bool} {P : Val → Prop} {w : World H}
    {run : Run H} (hb : p.Bound pc o g f) (h : UOk (FnAllowed o g) P (f = true) w run) :
    UOk (Allowed p pc (Eff.ofFn o g f)) P (Fails p pc (Eff.ofFn o g f)) w run :=
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

theorem le_pc_xl {top : Label} (pc a b : LExpr) :
    LExpr.le top (a.join b) ((pc.join a).join b) :=
  LExpr.join_mono (LExpr.le_join_right pc a) (LExpr.le_refl b)

theorem le_pc_xlb {top : Label} (pc a b c d : LExpr) :
    LExpr.le top ((a.join b).join d) ((((pc.join a).join b).join c).join d) :=
  LExpr.join_mono (LExpr.le_trans (le_pc_xl pc a b) (LExpr.le_join_left _ c)) (LExpr.le_refl d)

/-! ### The host's operations -/

theorem text_of_erase : ∀ {t : LTy}, t.erase = .text → ∃ l, t = .text l
  | .text l, _ => ⟨l, rfl⟩
  | .int _, h | .bool _, h | .unit _, h | .list _ _, h | .option _ _, h | .result _ _, h
  | .record _ _, h | .fn _ _ _ _ _ _, h | .var _ _, h => by simp [LTy.erase] at h

/-- Related texts the observer sees are equal. -/
theorem VList.eq_texts : ∀ {ts : List LTy} {vs₁ vs₂ : List Val}, VList p L ts vs₁ vs₂ →
    (∀ t ∈ ts, ∃ l, t = .text l ∧ Vis p L l) → vs₁ = vs₂
  | [], _, _, .nil, _ => rfl
  | _ :: _, _, _, .cons h hs, ht => by
    obtain ⟨l, rfl, hl⟩ := ht _ List.mem_cons_self
    rw [V.eq_text h hl, VList.eq_texts hs (fun t ht' => ht t (List.mem_cons_of_mem _ ht'))]

theorem vis_ofLabel_of_inst {s : List (String × LExpr)} {l : Label}
    (h : Vis p L ((LExpr.ofLabel l).inst s)) : Vis p L (.ofLabel l) := by
  rw [LExpr.inst_ofLabel] at h; exact Vis.right h

/-- Seeing an `accepts` shows the instance of what flows into it. -/
theorem vis_of_le_accepts {s : List (String × LExpr)} {x : LExpr} {l : Label}
    (hle : p.le x (.ofLabel l)) (h : Vis p L (.ofLabel l)) : Vis p L (x.inst s) :=
  Vis.of_le (LExpr.le_of_le_inst_ofLabel (LExpr.le_inst s hle)) h

/-- The failure of a `Files` operation decided at `pc ⊔ ℓ`, as the observer sees it: it sees the
path's label and the file-state label. -/
theorem vis_fsFail {s : List (String × LExpr)} (hh : p.HostWF) {pc l : LExpr}
    (h : Vis p L ((p.fsFail (pc.join l)).inst s)) :
    Vis p L (l.inst s) ∧ Vis p L (.ofLabel p.fileState) := by
  simp only [Program.fsFail, hh.oldHost, Bool.false_eq_true, ite_false] at h
  exact ⟨Vis.of_le (LExpr.inst_le_join_inst _ _ _) (Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left _ _)) h),
    vis_ofLabel_of_inst (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h)⟩

/-- A request's failure, under ADR 0085's rules, is at least `internal`. -/
theorem vis_reqFail {s : List (String × LExpr)} (hh : p.HostWF) {pc : LExpr} {ts : List LTy}
    (h : Vis p L ((p.reqFail pc ts).inst s)) : Vis p L (.ofLabel internalLabel) := by
  simp only [Program.reqFail, hh.oldHost, Bool.false_eq_true, ite_false] at h
  exact Vis.of_le hh.reqFloor (vis_ofLabel_of_inst (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))

/-- **T6 for F1 with declassification (option B1), at every ground instance**: runs related up
to their first different release. -/
theorem t6_soundR (hwf : p.LWellFormed) (hb1 : p.B1WellFormed) (hh : p.HostWF)
    (hlaw : H.Lawful) : ∀ n : Nat,
    (∀ {σ : List (String × LTy)} {w₁ w₂ : World H} {e₁ e₂ : Env} {Γ : LCtx} {E : List String}
      {pc : LExpr} {r : Rhs} {t : LTy} {ε : Eff}, GroundSubst σ → p.EqArgsOK σ E → Γ.InPlace →
      pc.PlainFree → WorldRel p L w₁ w₂ →
      LEnvTy p e₁ (Γ.lsub p.recordDeep σ) → LEnvTy p e₂ (Γ.lsub p.recordDeep σ) →
      EnvRel p L (Γ.lsub p.recordDeep σ) e₁ e₂ → LRhsTy p E Γ pc r t ε →
      BRelR p L (V p L (t.lsubst p.recordDeep σ)) (Vis p L (ε.inst (p.sd σ)).failLabel)
        (evalRhs H n p w₁ e₁ r) (evalRhs H n p w₂ e₂ r)) ∧
    (∀ {σ : List (String × LTy)} {w₁ w₂ : World H} {e₁ e₂ : Env} {Γ : LCtx} {E : List String}
      {pc : LExpr} {e : Expr} {t : LTy} {ε : Eff}, GroundSubst σ → p.EqArgsOK σ E → Γ.InPlace →
      pc.PlainFree → WorldRel p L w₁ w₂ →
      LEnvTy p e₁ (Γ.lsub p.recordDeep σ) → LEnvTy p e₂ (Γ.lsub p.recordDeep σ) →
      EnvRel p L (Γ.lsub p.recordDeep σ) e₁ e₂ → LExprTy p E Γ pc e t ε →
      BRelR p L (V p L (t.lsubst p.recordDeep σ)) (Vis p L (ε.inst (p.sd σ)).failLabel)
        (eval H n p w₁ e₁ e) (eval H n p w₂ e₂ e))
  | 0 => ⟨fun _ _ _ _ _ _ _ _ _ h => by simp [evalRhs, Term] at h,
      fun _ _ _ _ _ _ _ _ _ h => by simp [eval, Term] at h⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := t6_soundR hwf hb1 hh hlaw n
    obtain ⟨uR, uE⟩ := unary_soundR (H := H) hwf hb1 n
    have hrunU : ∀ {σ : List (String × LTy)} {w : World H} {env : Env} {Γ : LCtx} {E : List String}
        {e : Expr} {t : LTy} {ε : Eff}, GroundSubst σ → LEnvTy p env (Γ.lsub p.recordDeep σ) →
        LExprTy p E Γ .bot e t ε →
        UGood p (LExpr.bot.inst (p.sd σ)) (t.lsubst p.recordDeep σ) (ε.inst (p.sd σ)) w
          (eval H n p w env e) :=
      fun hσ henv he => uE hσ henv he
    have hrunB : ∀ {σ : List (String × LTy)} {w₁ w₂ : World H} {e₁ e₂ : Env} {Γ : LCtx}
        {E : List String} {body : Expr} {t : LTy} {ε : Eff}, GroundSubst σ → p.EqArgsOK σ E →
        LCtx.InPlace Γ → WorldRel p L w₁ w₂ →
        LEnvTy p e₁ (Γ.lsub p.recordDeep σ) → LEnvTy p e₂ (Γ.lsub p.recordDeep σ) →
        EnvRel p L (Γ.lsub p.recordDeep σ) e₁ e₂ → LExprTy p E Γ .bot body t ε → ε.rels = [] →
        BRel p L (V p L (t.lsubst p.recordDeep σ)) (Vis p L (ε.inst (p.sd σ)).failLabel)
          (eval H n p w₁ e₁ body) (eval H n p w₂ e₂ body) :=
      fun hσ hE hΓ hw h₁ h₂ hr hb hrl =>
        BRelR.to_brel (noRelease_of_ugood (uE hσ h₁ hb) hrl) (ihE hσ hE hΓ rfl hw h₁ h₂ hr hb)
    constructor
    · intro σ w₁ w₂ e₁ e₂ Γ E pc r t ε hσ hE hΓ hpc hw he₁ he₂ hrel hr
      cases hr with
      | atom ha =>
        refine BRelR.of_brel ?_
        obtain ⟨v₁, v₂, h₁, h₂, hv, _, _⟩ := latom_bothσ he₁ he₂ hrel ha
        simp only [evalRhs, h₁, h₂, Option.elim]
        exact brel_done hw hv
      | @prim _ _ pr _ ts ty has hp =>
        refine BRelR.of_brel ?_
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, hu₁, hu₂⟩ := latoms_bothσ he₁ he₂ hrel has
        simp only [evalRhs, h₁, h₂, Option.bind_some]
        rw [ofScalar_lsubst]
        by_cases hv : Vis p L ((p.primLabel pr ts).inst (p.sd σ))
        · rw [prim_agreeσ hp hvs hv]
          apply brel_same hw
          intro v hv'
          cases hev : evalPrim pr vs₂ with
          | none => simp [hev] at hv'
          | some u =>
            simp only [hev, Option.elim] at hv'
            cases hv'
            exact V.ofScalar_refl (prim_ltyped (p := p) _ hp hev)
        · refine brel_quiet hw rfl (fun a b => .hidden (by rw [ofScalar_top]; exact hv)) ?_ ?_
          · intro c hc; cases h : evalPrim pr vs₁ <;> simp [h] at hc
          · intro c hc; cases h : evalPrim pr vs₂ <;> simp [h] at hc
      | @call _ _ f targs args fn ts hf hnv hlen _ heqa hg has hfl hb _ =>
        obtain ⟨tb, εb, hbody, hflow, _, _, _, hfail, _⟩ := funOk_of_nvia hwf hnv hf
        have hτ : GroundSubst (fn.tparams.zip targs) := zip_ground hg
        have hcov := lookup_zip_cover (xs := fn.tparams) hlen
        have hEτ := eqArgsOK_of_call hwf hf hlen hg heqa
        obtain ⟨hps, hres⟩ := hwf.sigsScoped fn (find_mem hf)
        have hpg : ∀ u ∈ fn.params.map (fun q => p.lsub (fn.tparams.zip targs) q.2), u.Ground := by
          intro u hu
          obtain ⟨q, hq, rfl⟩ := List.mem_map.mp hu
          exact LTy.lsubst_ground _ hτ hcov (hps q hq)
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, hu₁, hu₂⟩ := latoms_bothσ he₁ he₂ hrel has
        have ha₁ := LValsTy.ground_lsubst hpg (LValsTy.flowsσ hu₁ hfl)
        have ha₂ := LValsTy.ground_lsubst hpg (LValsTy.flowsσ hu₂ hfl)
        have hva := VList.ground_lsubst hpg (VList.flowsσ hvs hfl)
        simp only [evalRhs, hf, h₁, h₂]
        rw [ite_eq_left (by rw [LValsTy.length ha₁, List.length_map]),
          ite_eq_left (by rw [LValsTy.length ha₂, List.length_map])]
        have hen : ∀ {vs : List Val}, LValsTy p vs (fn.params.map (fun q => p.lsub (fn.tparams.zip targs) q.2)) →
            LEnvTy p (bindParams fn.params vs []) (LCtx.lsub p.recordDeep (fn.tparams.zip targs) fn.params) := by
          intro vs hvs'
          have := bindParams_ltyped (params := LCtx.lsub p.recordDeep (fn.tparams.zip targs) fn.params)
            (by rw [LCtx.lsub_snd]; simpa [Program.lsub, List.map_map, Function.comp_def] using hvs')
            (LEnvTy.nil (p := p))
          simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using this
        have hrel' : EnvRel p L (LCtx.lsub p.recordDeep (fn.tparams.zip targs) fn.params)
            (bindParams fn.params vs₁ []) (bindParams fn.params vs₂ []) := by
          have := bindParams_rel (params := LCtx.lsub p.recordDeep (fn.tparams.zip targs) fn.params)
            (by rw [LCtx.lsub_snd]; simpa [Program.lsub, List.map_map, Function.comp_def] using hva)
            (EnvRel.nil (p := p) (L := L))
          simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using this
        have hΓf : LCtx.InPlace fn.params := fun q hq => (hwf.inPlace.1 fn (find_mem hf)).1 q hq
        refine BRelR.mono (fun _ _ h => ?_) (fun _ => vis_failLabel_inst _ hfail)
          (ihE hτ hEτ hΓf rfl hw (by simpa using hen ha₁) (by simpa using hen ha₂) (by simpa using hrel') hbody)
        have hr := LTy.lsubst_ground p.recordDeep hτ hcov hres
        exact (V.ground_lsubst hr).mpr (V.flow h (Flow.lsubst _ _ hflow))
      | @apply _ _ f args ps r o g fl lf ts hf has hfl hb =>
        refine BRelR.of_brel ?_
        obtain ⟨fv₁, fv₂, hf₁, hf₂, hfv, hft₁, hft₂⟩ := latom_bothσ he₁ he₂ hrel hf
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, hu₁, hu₂⟩ := latoms_bothσ he₁ he₂ hrel has
        simp only [evalRhs, hf₁, hf₂, h₁, h₂]
        have hfi := latom_inPlace hΓ hf
        simp only [LTy.InPlace] at hfi
        have hdp : (lf.join (decision (decisionsOf ts))).PlainFree :=
          LExpr.plainFree_join hfi.2.2 rfl
        simp only [LTy.lsubst, LTy.lsubsts_eq] at hfv hft₁ hft₂
        by_cases hvis : Vis p L ((lf.join (decision (decisionsOf ts))).inst (p.sd σ))
        · have hd : Vis p L ((decision (decisionsOf ts)).inst (p.sd σ)) :=
            Vis.of_le (LExpr.inst_le_join_inst _ _ _) hvis
          have hl : Vis p L (lf.inst (p.sd σ)) := Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left _ _)) hvis
          exact BRel.mono (fun _ _ h => V.raiseσ _ h) id
            (applyWith_brel hrunB hw hfv hl
              (VList.unraiseσ rfl hd (VList.flowsσ hvs hfl))
              (LValsTy.lsubst_unraise (LValsTy.flowsσ hu₁ hfl))
              (LValsTy.lsubst_unraise (LValsTy.flowsσ hu₂ hfl)) _)
        · have hpc' : ¬ Vis p L ((pc.join (lf.join (decision (decisionsOf ts)))).inst (p.sd σ)) :=
            fun h => hvis (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h)
          exact brel_confined hh hw
            (hof_uok (hb.inst _) (applyWith_uok hrunU hft₁ (LValsTy.lsubst_unraise (LValsTy.flowsσ hu₁ hfl)) w₁))
            (hof_uok (hb.inst _) (applyWith_uok hrunU hft₂ (LValsTy.lsubst_unraise (LValsTy.flowsσ hu₂ hfl)) w₂))
            hpc' (fun a b => V.hideσ hdp a b hvis) (fun _ => Vis.ofFn_plain) (fun _ => Vis.ofFn_plain)
      | @op _ _ line t hl ht hle _ =>
        refine BRelR.of_brel ?_
        cases t <;> simp [LTy.erase, Ty.Scalar] at ht
        all_goals
          rename_i l
          obtain ⟨v₁, v₂, h₁, h₂, hv, hu₁, hu₂⟩ := latom_bothσ he₁ he₂ hrel hl
          simp only [LTy.lsubst] at hv hu₁ hu₂
          have hacc : Vis p L (.ofLabel p.acceptsOp) → Vis p L (l.inst (p.sd σ)) := fun h =>
            Vis.of_le (LExpr.le_trans (LExpr.inst_le_join_inst _ _ _)
              (LExpr.le_of_le_inst_ofLabel (LExpr.le_inst _ hle))) h
          by_cases hd : Vis p L (l.inst (p.sd σ))
          · have hs : v₁ = v₂ := by
              first | exact V.eq_int hv hd | exact V.eq_bool hv hd | exact V.eq_text hv hd
            subst hs
            cases hu₁ <;> simp only [evalRhs, h₁, h₂, Val.line] <;>
              exact brel_same hw (fun _ h => by cases h; exact .unit)
          · cases hu₁ <;> cases hu₂ <;> simp only [evalRhs, h₁, h₂, Val.line] <;>
              exact brel_quiet hw
                (by simp only [proj_single_hidden (ev := .out _) (fun h => hd (hacc h))])
                (fun _ _ => .unit) (fun _ h => nomatch h) (fun _ h => nomatch h)
      | @read _ _ path t hu ht hle _ =>
        refine BRelR.of_brel ?_
        cases t <;> simp [LTy.erase] at ht
        rename_i l
        obtain ⟨v₁, v₂, h₁, h₂, hv, hu₁, hu₂⟩ := latom_bothσ he₁ he₂ hrel hu
        simp only [LTy.lsubst] at hv hu₁ hu₂
        cases hu₁
        cases hu₂
        rename_i s₁ s₂
        simp only [evalRhs, h₁, h₂]
        have hacc : Vis p L (.ofLabel p.acceptsRead) → Vis p L (l.inst (p.sd σ)) := fun h =>
          Vis.of_le (LExpr.inst_le_join_inst _ _ _) (vis_of_le_accepts hle h)
        have hpath : Vis p L (l.inst (p.sd σ)) → s₁ = s₂ := fun hd => by
          have := V.eq_text hv hd; injection this
        have hev : proj p L [Event.read s₁] = proj p L [Event.read s₂] := by
          by_cases ha : Vis p L (.ofLabel p.acceptsRead)
          · rw [hpath (hacc ha)]
          · rw [proj_single_hidden (ev := .read s₁) ha, proj_single_hidden (ev := .read s₂) ha]
        by_cases hi : Vis p L (.ofLabel internalLabel)
        · have hweq := hw.2 hi
          subst hweq
          have hs := hpath (hacc (Vis.of_le hh.read hi))
          subst hs
          refine brel_same (WorldRel.refl _) (fun v hv' => ?_)
          split at hv' <;> simp at hv'
          subst hv'
          exact .text
        · have hres : ¬ Vis p L ((LExpr.ofLabel p.readResult).inst (p.sd σ)) := fun h =>
            hi (Vis.of_le hh.readResult (vis_ofLabel_of_inst h))
          have hsame : Vis p L ((p.fsFail (pc.join (p.deep (.text l)))).inst (p.sd σ)) →
              ((H.read s₁ w₁.fs).isSome ↔ (H.read s₂ w₂.fs).isSome) := fun hvf => by
            obtain ⟨hl, hfs⟩ := vis_fsFail hh hvf
            rw [hpath hl]
            exact hlaw.read_fails s₂ (hw.1 hfs)
          intro ht₁ ht₂
          cases hr₁ : H.read s₁ w₁.fs <;> cases hr₂ : H.read s₂ w₂.fs <;>
            simp only [hr₁, hr₂] at ht₁ ht₂ ⊢ <;> refine ⟨hev, ?_, hw⟩ <;> simp only [OutRel]
          all_goals first
            | exact fun _ => rfl
            | trivial
            | exact .hidden hres
            | exact fun _ => trivial
            | exact fun hvf => by
                have := hsame (by simpa [Eff.inst, Eff.failLabel] using hvf)
                simp [hr₁, hr₂] at this
      | @write _ _ path content t c hu ht hc hct hle hst _ =>
        refine BRelR.of_brel ?_
        cases t <;> simp [LTy.erase] at ht
        cases c <;> simp [LTy.erase] at hct
        rename_i l lc
        obtain ⟨v₁, v₂, h₁, h₂, hv, hu₁, hu₂⟩ := latom_bothσ he₁ he₂ hrel hu
        obtain ⟨c₁, c₂, hc₁, hc₂, hcv, hcu₁, hcu₂⟩ := latom_bothσ he₁ he₂ hrel hc
        simp only [LTy.lsubst] at hv hu₁ hu₂ hcv hcu₁ hcu₂
        cases hu₁
        cases hu₂
        cases hcu₁
        cases hcu₂
        rename_i s₁ s₂ cs₁ cs₂
        simp only [evalRhs, h₁, h₂, hc₁, hc₂]
        have hpath : Vis p L (l.inst (p.sd σ)) → s₁ = s₂ := fun hd => by
          have := V.eq_text hv hd; injection this
        have hcont : Vis p L (lc.inst (p.sd σ)) → cs₁ = cs₂ := fun hd => by
          have := V.eq_text hcv hd; injection this
        have hacc : Vis p L (.ofLabel p.acceptsWrite) →
            Vis p L (l.inst (p.sd σ)) ∧ Vis p L (lc.inst (p.sd σ)) := fun h =>
          ⟨Vis.of_le (LExpr.inst_le_join_inst _ _ _)
              (Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left _ _)) (vis_of_le_accepts hle h)),
            Vis.of_le (LExpr.inst_le_join_inst _ _ _) (vis_of_le_accepts hle h)⟩
        have hstate : Vis p L (.ofLabel p.fileState) → Vis p L (l.inst (p.sd σ)) := fun h =>
          Vis.of_le (LExpr.inst_le_join_inst _ _ _) (vis_of_le_accepts (hst hh.oldHost) h)
        have hev : proj p L [Event.write s₁ cs₁] = proj p L [Event.write s₂ cs₂] := by
          by_cases ha : Vis p L (.ofLabel p.acceptsWrite)
          · rw [hpath (hacc ha).1, hcont (hacc ha).2]
          · rw [proj_single_hidden (ev := .write s₁ cs₁) ha, proj_single_hidden (ev := .write s₂ cs₂) ha]
        by_cases hi : Vis p L (.ofLabel internalLabel)
        · have hweq := hw.2 hi
          subst hweq
          have ha := hacc (Vis.of_le hh.write hi)
          have hs := hpath ha.1
          have hcs := hcont ha.2
          subst hs hcs
          refine brel_same (WorldRel.refl _) (fun v hv' => ?_)
          split at hv' <;> simp at hv'
          subst hv'
          exact .unit
        · intro ht₁ ht₂
          by_cases hfv : Vis p L (.ofLabel p.fileState)
          · have hs := hpath (hstate hfv)
            subst hs
            have hlaw' := hlaw.write_shape s₁ cs₁ cs₂ (hw.1 hfv)
            cases hr₁ : H.write s₁ cs₁ w₁.fs <;> cases hr₂ : H.write s₁ cs₂ w₂.fs <;>
              simp only [hr₁, hr₂, Option.map_none, Option.map_some, reduceCtorEq,
                Option.some.injEq] at hlaw' ht₁ ht₂ ⊢
            · exact ⟨hev, fun _ => rfl, hw⟩
            · exact ⟨hev, .unit, fun _ => hlaw', fun h => absurd h hi⟩
          · have hnvf : ¬ Vis p L ((p.fsFail (pc.join (p.deep (.text l)))).inst (p.sd σ)) :=
              fun h => hfv (vis_fsFail hh h).2
            have hw' : ∀ {u₁ u₂ : World H}, WorldRel p L u₁ u₂ :=
              ⟨fun h => absurd h hfv, fun h => absurd h hi⟩
            cases hr₁ : H.write s₁ cs₁ w₁.fs <;> cases hr₂ : H.write s₂ cs₂ w₂.fs <;>
              simp only [hr₁, hr₂] at ht₁ ht₂ ⊢ <;> refine ⟨hev, ?_, hw'⟩ <;> simp only [OutRel]
            all_goals first
              | exact fun _ => rfl
              | trivial
              | exact .unit
              | exact fun h => absurd (by simpa [Eff.inst, Eff.failLabel] using h) hnvf
              | exact fun _ => trivial
      | @req _ _ m args ts has hts hle _ =>
        refine BRelR.of_brel ?_
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, _, _⟩ := latoms_bothσ he₁ he₂ hrel has
        simp only [evalRhs, h₁, h₂]
        have hargs : Vis p L (.ofLabel p.acceptsReq) → vs₁ = vs₂ := fun ha => by
          refine VList.eq_texts hvs (fun t ht => ?_)
          obtain ⟨u, hu, rfl⟩ := List.mem_map.mp ht
          obtain ⟨l, rfl⟩ := text_of_erase (hts u hu)
          refine ⟨_, rfl, ?_⟩
          refine Vis.of_le (LExpr.le_inst _ ?_) (Vis.of_le (LExpr.inst_le_join_inst _ _ _) (vis_of_le_accepts hle ha))
          exact LExpr.le_joinAll (List.mem_map.mpr ⟨_, hu, rfl⟩)
        have hev : proj p L [Event.req m vs₁] = proj p L [Event.req m vs₂] := by
          by_cases ha : Vis p L (.ofLabel p.acceptsReq)
          · rw [hargs ha]
          · rw [proj_single_hidden (ev := .req m vs₁) ha, proj_single_hidden (ev := .req m vs₂) ha]
        by_cases hi : Vis p L (.ofLabel internalLabel)
        · have hweq := hw.2 hi
          subst hweq
          have hv := hargs (Vis.of_le hh.req hi)
          subst hv
          refine brel_same (WorldRel.refl _) (fun v hv' => ?_)
          split at hv' <;> simp at hv'
          subst hv'
          exact .text
        · have hres : ¬ Vis p L ((LExpr.ofLabel p.reqResult).inst (p.sd σ)) := fun h =>
            hi (Vis.of_le hh.reqResult (vis_ofLabel_of_inst h))
          have hnvf : ¬ Vis p L ((p.reqFail pc ts).inst (p.sd σ)) := fun h => hi (vis_reqFail hh h)
          intro ht₁ ht₂
          rcases hq₁ : H.request m vs₁ w₁.srv with ⟨o₁, srv₁⟩
          rcases hq₂ : H.request m vs₂ w₂.srv with ⟨o₂, srv₂⟩
          have hw' : WorldRel p L { w₁ with srv := srv₁ } { w₂ with srv := srv₂ } :=
            ⟨fun h => hw.1 h, fun h => absurd h hi⟩
          cases o₁ <;> cases o₂ <;> simp only [hq₁, hq₂] at ht₁ ht₂ ⊢ <;> refine ⟨hev, ?_, hw'⟩ <;>
            simp only [OutRel]
          all_goals first
            | exact fun _ => rfl
            | trivial
            | exact .hidden hres
            | exact fun h => absurd (by simpa [Eff.inst, Eff.failLabel] using h) hnvf
            | exact fun _ => trivial
      | construct hrec has hfl =>
        refine BRelR.of_brel ?_
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, _, _⟩ := latoms_bothσ he₁ he₂ hrel has
        simp only [evalRhs, h₁, h₂, Option.map, Option.elim, LTy.lsubst]
        exact brel_done hw (.record hrec (VList.ground_lsubst (hwf.recordsGround _ (record_mem hrec))
          (VList.flowsσ hvs hfl)))
      | @field _ _ value record index rec l t hv hrec hi =>
        refine BRelR.of_brel ?_
        obtain ⟨v₁, v₂, h₁, h₂, hvv, hu₁, hu₂⟩ := latom_bothσ he₁ he₂ hrel hv
        have hli := latom_inPlace hΓ hv
        simp only [LTy.InPlace] at hli
        simp only [LTy.lsubst] at hvv hu₁ hu₂
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
            refine brel_done hw ?_
            cases hvv with
            | hidden h => exact V.hideσ hli _ _ h
            | record hr hrel' =>
              rw [hrec] at hr
              cases hr
              obtain ⟨a', b', ha', hb'', hab⟩ := VList.get hrel' hi
              rw [ha] at ha'
              rw [hb'] at hb''
              cases ha'
              cases hb''
              have hg := hwf.recordsGround _ (record_mem hrec) _ (List.mem_of_getElem? hi)
              exact V.raiseσ _ ((V.ground_lsubst hg).mpr hab)
      | list has hfl _ =>
        refine BRelR.of_brel ?_
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, _, _⟩ := latoms_bothσ he₁ he₂ hrel has
        simp only [evalRhs, h₁, h₂, Option.map, Option.elim, LTy.lsubst]
        exact brel_done hw (.list (VAll.of_flows hvs (fun u hu => by
          obtain ⟨u', hu', rfl⟩ := List.mem_map.mp hu
          exact Flow.lsubst _ _ (hfl u' hu'))))
      | none _ => exact .of_brel (brel_done hw .none)
      | some ha =>
        refine BRelR.of_brel ?_
        obtain ⟨v₁, v₂, h₁, h₂, hv, _, _⟩ := latom_bothσ he₁ he₂ hrel ha
        simp only [evalRhs, h₁, h₂, Option.map, Option.elim, LTy.lsubst]
        exact brel_done hw (.some hv)
      | @lambda _ _ params body tb εl hb hfl hrl hps =>
        refine BRelR.of_brel ?_
        have hΓ' : LCtx.InPlace (params ++ Γ) := fun q hq => by
          rcases List.mem_append.mp hq with hq | hq
          · exact hps q hq
          · exact hΓ q hq
        refine brel_done hw ?_
        simp only [LTy.lsubst]
        exact .closure hσ hE hΓ' hrel he₁ he₂ hb hfl hrl (by rw [LTy.lsubsts_eq]; exact Flows.refl _)
          (Flow.refl _) id (HostFx.Le.refl _) id
      | @funRef _ _ f targs fn hf hnv hlen _ heqa hg hfr =>
        refine BRelR.of_brel ?_
        obtain ⟨tb, εb, hbody, hflow, ho, hgo, hfs, hfail, hrs⟩ := funOk_of_nvia hwf hnv hf
        have hrl : εb.rels = [] :=
          List.eq_nil_iff_forall_not_mem.mpr (fun x hx => by simpa [hfr] using hrs x hx)
        have hτ : GroundSubst (fn.tparams.zip targs) := zip_ground hg
        have hcov := lookup_zip_cover (xs := fn.tparams) hlen
        have hEτ := eqArgsOK_of_call hwf hf hlen hg heqa
        obtain ⟨hps, hres⟩ := hwf.sigsScoped fn (find_mem hf)
        have hΓf : LCtx.InPlace (fn.params ++ []) := by
          rw [List.append_nil]; exact fun q hq => (hwf.inPlace.1 fn (find_mem hf)).1 q hq
        simp only [evalRhs, hf]
        refine brel_done hw ?_
        simp only [LTy.lsubst, Program.lsub]
        refine .closure (Γ := []) (E := fn.eqParams) hτ hEτ hΓf .nil .nil .nil (by simpa using hbody)
          hfail hrl ?_ ?_ ho hgo hfs
        · rw [LTy.lsubsts_eq]
          have : ∀ qs : List (String × LTy), (∀ q ∈ qs, q.2.ScopedIn fn.tparams) →
              Flows p.top ((qs.map (fun q => q.2.lsubst p.recordDeep (fn.tparams.zip targs))).map
                (LTy.lsubst p.recordDeep σ))
                ((qs.map Prod.snd).map (LTy.lsubst p.recordDeep (fn.tparams.zip targs))) := by
            intro qs hqs
            induction qs with
            | nil => exact .nil
            | cons q qs ih =>
              exact .cons (Flow.lsubst_ground _ σ
                  (LTy.lsubst_ground _ hτ hcov (hqs q List.mem_cons_self))).1
                (ih (fun q' hq' => hqs q' (List.mem_cons_of_mem _ hq')))
          exact this fn.params hps
        · exact Flow.trans (Flow.lsubst _ _ hflow)
            (Flow.lsubst_ground _ σ (LTy.lsubst_ground _ hτ hcov hres)).2
      | @declassify _ _ pol a po fn t hpo hfn ha hflow hle _ =>
        obtain ⟨v₁, v₂, h₁, h₂, _, hu₁, hu₂⟩ := latom_bothσ he₁ he₂ hrel ha
        have hmem := policy_mem hpo
        obtain ⟨_, _, fn', hfn', hps, hres⟩ := hwf.wellFormed.policies _ hmem
        rw [hfn] at hfn'
        cases hfn'
        obtain ⟨fnv, hfnv, _, _, _, hpure⟩ := hb1.via po hmem
        rw [hfn] at hfnv
        cases hfnv
        obtain ⟨hdg, hds⟩ := hb1.dst po hmem
        have hsrc : (t.lsubst p.recordDeep σ).erase = po.src.erase := by
          rw [LTy.erase_lsubst, Flow.erase hflow, Ty.Scalar.subst _ (hb1.src po hmem)]
        have hlen := paramTys_length fn []
        rw [hps] at hlen
        have hds' : (po.dst.lsubst p.recordDeep σ).erase = po.dst.erase := by
          rw [LTy.erase_lsubst, Ty.Scalar.subst _ hds]
        -- Each run of `via`: no event, no failure, and a value of the scalar `to`.
        have hvia : ∀ {w : World H} {v : Val}, LValTy p v (t.lsubst p.recordDeep σ) →
            Quiet w (eval H n p w (bindParams fn.params [v] []) fn.body) ∧
            ∀ r, (eval H n p w (bindParams fn.params [v] []) fn.body).2.1 = .ok r →
              ValTy p r po.dst.erase := by
          intro w v hvt
          have hvs : ValsTy p [v] (fn.paramTys []) := by
            rw [hps, ← hsrc]
            exact .cons (valTy_of_scalar (by rw [hsrc]; exact hb1.src po hmem) hvt) .nil
          have henv' := bindParams_typed hvs (paramTys_length fn []) (EnvTy.nil (p := p))
          rw [List.append_nil] at henv'
          have hbody := body_typed hwf.wellFormed hfn []
          rw [Ty.subst_nil, hres] at hbody
          refine ⟨(pure_run p w n).2 _ _ hpure, fun r hr => ?_⟩
          have hg := (sound hwf.wellFormed n).2 (w := w) henv' hbody
          rw [hr] at hg
          exact hg
        have q₁ := hvia (w := w₁) hu₁
        have q₂ := hvia (w := w₂) hu₂
        simp only [evalRhs, hpo, h₁, h₂, hfn]
        have hl1 : 1 = fn.params.length := by simpa using hlen
        rw [if_pos hl1, if_pos hl1]
        rcases e₁r : eval H n p w₁ (bindParams fn.params [v₁] []) fn.body with ⟨t₁, o₁, w₁'⟩
        rcases e₂r : eval H n p w₂ (bindParams fn.params [v₂] []) fn.body with ⟨t₂, o₂, w₂'⟩
        rw [e₁r] at q₁
        rw [e₂r] at q₂
        obtain ⟨⟨htr₁, hnf₁, hw₁⟩, hok₁⟩ := q₁
        obtain ⟨⟨htr₂, hnf₂, hw₂⟩, hok₂⟩ := q₂
        simp only at htr₁ htr₂ hnf₁ hnf₂ hok₁ hok₂ hw₁ hw₂
        subst htr₁ htr₂ hw₁ hw₂
        intro ht₁ ht₂
        cases o₁ <;> cases o₂ <;> simp only [Term] at ht₁ ht₂
        all_goals first
          | (exfalso; exact hnf₁ _ rfl)
          | (exfalso; exact hnf₂ _ rfl)
          | skip
        rename_i r₁ r₂
        have hr₁ := hok₁ r₁ rfl
        have hr₂ := hok₂ r₂ rfl
        simp only [List.nil_append]
        have hev : ∀ r, EvVis p L (Event.release pol r) ↔ Vis p L po.dst.top := fun r =>
          ⟨fun ⟨po', hpo', hv⟩ => by rw [hpo] at hpo'; cases hpo'; exact hv, fun hv => ⟨po, hpo, hv⟩⟩
        by_cases hvd : Vis p L po.dst.top
        · rw [proj_single_vis ((hev r₁).mpr hvd), proj_single_vis ((hev r₂).mpr hvd)]
          by_cases heq : r₁ = r₂
          · subst heq
            exact .inl ⟨rfl, V.refl_scalar (by rw [hds']; exact hds) (by rw [hds']; exact hr₁), hw⟩
          · exact .inr ⟨[], pol, r₁, r₂, [], [], heq, by simp [proj_single_vis ((hev r₁).mpr hvd)],
              by simp [proj_single_vis ((hev r₂).mpr hvd)]⟩
        · rw [proj_single_hidden (fun h => hvd ((hev r₁).mp h)),
            proj_single_hidden (fun h => hvd ((hev r₂).mp h))]
          refine .inl ⟨rfl, .hidden ?_, hw⟩
          rw [top_lsubst_of_ground hdg]
          exact fun h => hvd (Vis.of_le (LExpr.le_join_right _ _) h)
      | @map _ _ xs f te lx tp tr o g fl lf hxs hf hte hb =>
        refine BRelR.of_brel ?_
        obtain ⟨x₁, x₂, hx₁, hx₂, hxv, hxu₁, hxu₂⟩ := latom_bothσ he₁ he₂ hrel hxs
        obtain ⟨f₁, f₂, hf₁, hf₂, hfv, hfu₁, hfu₂⟩ := latom_bothσ he₁ he₂ hrel hf
        simp only [LTy.lsubst, LTy.lsubsts] at hxv hxu₁ hxu₂ hfv hfu₁ hfu₂
        have hte' := Flow.lsubst p.recordDeep σ hte
        cases hxu₁ with
        | list hi₁ =>
          cases hxu₂ with
          | list hi₂ =>
            simp only [evalRhs, hx₁, hx₂, hf₁, hf₂, LTy.lsubst]
            by_cases hvis : Vis p L ((lx.join lf).inst (p.sd σ))
            · have hvx := Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left lx lf)) hvis
              have hvf := Vis.of_le (LExpr.inst_le_join_inst _ lx lf) hvis
              cases hxv with
              | hidden h => exact absurd hvx h
              | list hall =>
                refine BRel.mono (fun a b h => ?_) (fun _ => trivial)
                  (walkMap_brel (fun w₁ w₂ x y hw' hxy hx hy => applyWith_brel hrunB hw' hfv hvf
                    (.cons (V.flow hxy hte') .nil) (.cons (LValTy.flow hx hte') .nil)
                    (.cons (LValTy.flow hy hte') .nil) True) hw hall hi₁ hi₂)
                obtain ⟨zs₁, zs₂, rfl, rfl, hz⟩ := h
                exact .list (VAll.flow hz (Flow.lsubst _ _ (Flow.raise _ _)))
            · have hpc' : ¬ Vis p L (((pc.join lx).join lf).inst (p.sd σ)) := fun h =>
                hvis (Vis.of_le (LExpr.le_inst _ (le_pc_xl pc lx lf)) h)
              refine brel_confined hh hw (hof_uok (P := fun v => LValTy p v
                  (.list ((tr.raise lf).lsubst p.recordDeep σ) ((lx.join lf).inst (p.sd σ)))) (hb.inst _) ?_)
                (hof_uok (P := fun v => LValTy p v
                  (.list ((tr.raise lf).lsubst p.recordDeep σ) ((lx.join lf).inst (p.sd σ)))) (hb.inst _) ?_)
                hpc' (fun a b => .hidden hvis) (fun _ => Vis.ofFn_plain) (fun _ => Vis.ofFn_plain)
              · exact walkMap_uok _ (fun w v hv => UOk.mono (fun _ h => h) (fun _ h => LValTy.lsubst_raise h) id
                  (applyWith_uok hrunU hfu₁ (.cons (LValTy.flow hv hte') .nil) w)) w₁ hi₁
              · exact walkMap_uok _ (fun w v hv => UOk.mono (fun _ h => h) (fun _ h => LValTy.lsubst_raise h) id
                  (applyWith_uok hrunU hfu₂ (.cons (LValTy.flow hv hte') .nil) w)) w₂ hi₂
      | @filter _ _ xs keep te lx tp lb o g fl lf hxs hk hte hb =>
        refine BRelR.of_brel ?_
        obtain ⟨x₁, x₂, hx₁, hx₂, hxv, hxu₁, hxu₂⟩ := latom_bothσ he₁ he₂ hrel hxs
        obtain ⟨f₁, f₂, hf₁, hf₂, hfv, hfu₁, hfu₂⟩ := latom_bothσ he₁ he₂ hrel hk
        simp only [LTy.lsubst, LTy.lsubsts] at hxv hxu₁ hxu₂ hfv hfu₁ hfu₂
        have hte' := Flow.lsubst p.recordDeep σ hte
        cases hxu₁ with
        | list hi₁ =>
          cases hxu₂ with
          | list hi₂ =>
            simp only [evalRhs, hx₁, hx₂, hf₁, hf₂, LTy.lsubst]
            by_cases hvis : Vis p L ((lx.join lf).inst (p.sd σ))
            · have hvx := Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left lx lf)) hvis
              have hvf := Vis.of_le (LExpr.inst_le_join_inst _ lx lf) hvis
              cases hxv with
              | hidden h => exact absurd hvx h
              | list hall =>
                refine BRel.mono (fun a b h => ?_) (fun _ => trivial)
                  (walkFilter_brel (fun w₁ w₂ x y hw' hxy hx hy => applyWith_brel hrunB hw' hfv hvf
                    (.cons (V.flow hxy hte') .nil) (.cons (LValTy.flow hx hte') .nil)
                    (.cons (LValTy.flow hy hte') .nil) True) hw hall hi₁ hi₂)
                by_cases hl : Vis p L (((lx.join lf).join lb).inst (p.sd σ))
                · obtain ⟨zs₁, zs₂, rfl, rfl, hz⟩ := h (Vis.of_le (LExpr.inst_le_join_inst _ _ _) hl)
                  exact .list hz
                · exact .hidden hl
            · have hpc' : ¬ Vis p L (((pc.join lx).join lf).inst (p.sd σ)) := fun h =>
                hvis (Vis.of_le (LExpr.le_inst _ (le_pc_xl pc lx lf)) h)
              refine brel_confined hh hw (hof_uok (P := fun v => LValTy p v
                  (.list (te.lsubst p.recordDeep σ) (((lx.join lf).join lb).inst (p.sd σ)))) (hb.inst _) ?_)
                (hof_uok (P := fun v => LValTy p v
                  (.list (te.lsubst p.recordDeep σ) (((lx.join lf).join lb).inst (p.sd σ)))) (hb.inst _) ?_)
                hpc' (fun a b => .hidden (fun h => hvis (Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left _ _)) h)))
                (fun _ => Vis.ofFn_plain) (fun _ => Vis.ofFn_plain)
              · exact walkFilter_uok _ (fun w v hv =>
                  applyWith_uok hrunU hfu₁ (.cons (LValTy.flow hv hte') .nil) w) w₁ hi₁
              · exact walkFilter_uok _ (fun w v hv =>
                  applyWith_uok hrunU hfu₂ (.cons (LValTy.flow hv hte') .nil) w) w₂ hi₂
      | @firstWhere _ _ xs keep te lx tp lb o g fl lf hxs hk hte hb =>
        refine BRelR.of_brel ?_
        obtain ⟨x₁, x₂, hx₁, hx₂, hxv, hxu₁, hxu₂⟩ := latom_bothσ he₁ he₂ hrel hxs
        obtain ⟨f₁, f₂, hf₁, hf₂, hfv, hfu₁, hfu₂⟩ := latom_bothσ he₁ he₂ hrel hk
        simp only [LTy.lsubst, LTy.lsubsts] at hxv hxu₁ hxu₂ hfv hfu₁ hfu₂
        have hte' := Flow.lsubst p.recordDeep σ hte
        cases hxu₁ with
        | list hi₁ =>
          cases hxu₂ with
          | list hi₂ =>
            simp only [evalRhs, hx₁, hx₂, hf₁, hf₂, LTy.lsubst]
            by_cases hvis : Vis p L (((lx.join lf).join lb).inst (p.sd σ))
            · have hvxf := Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left _ lb)) hvis
              have hvx := Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left lx lf)) hvxf
              have hvf := Vis.of_le (LExpr.inst_le_join_inst _ lx lf) hvxf
              have hvb := Vis.of_le (LExpr.inst_le_join_inst _ _ lb) hvis
              cases hxv with
              | hidden h => exact absurd hvx h
              | list hall =>
                refine BRel.mono (fun a b h => ?_) (fun _ => trivial)
                  (walkFirst_brel hvb (fun w₁ w₂ x y hw' hxy hx hy => applyWith_brel hrunB hw' hfv hvf
                    (.cons (V.flow hxy hte') .nil) (.cons (LValTy.flow hx hte') .nil)
                    (.cons (LValTy.flow hy hte') .nil) True) hw hall hi₁ hi₂)
                rcases h with ⟨rfl, rfl⟩ | ⟨x, y, rfl, rfl, hxy⟩
                · exact .none
                · exact .some hxy
            · have hpc' : ¬ Vis p L (((((pc.join lx).join lf).join (p.deep te)).join lb).inst (p.sd σ)) :=
                fun h => hvis (Vis.of_le (LExpr.le_inst _ (le_pc_xlb pc lx lf _ lb)) h)
              refine brel_confined hh hw (hof_uok (P := fun v => LValTy p v
                  (.option (te.lsubst p.recordDeep σ) (((lx.join lf).join lb).inst (p.sd σ)))) (hb.inst _) ?_)
                (hof_uok (P := fun v => LValTy p v
                  (.option (te.lsubst p.recordDeep σ) (((lx.join lf).join lb).inst (p.sd σ)))) (hb.inst _) ?_)
                hpc' (fun a b => .hidden hvis) (fun _ => Vis.ofFn_plain) (fun _ => Vis.ofFn_plain)
              · exact walkFirst_uok _ (fun w v hv =>
                  applyWith_uok hrunU hfu₁ (.cons (LValTy.flow hv hte') .nil) w) w₁ hi₁
              · exact walkFirst_uok _ (fun w v hv =>
                  applyWith_uok hrunU hfu₂ (.cons (LValTy.flow hv hte') .nil) w) w₂ hi₂
      | @fold _ _ xs init f te lx ti ta tp tr o g fl lf hxs hi hf hte hinit hres hb =>
        refine BRelR.of_brel ?_
        obtain ⟨x₁, x₂, hx₁, hx₂, hxv, hxu₁, hxu₂⟩ := latom_bothσ he₁ he₂ hrel hxs
        obtain ⟨i₁, i₂, hi₁', hi₂', hiv, hiu₁, hiu₂⟩ := latom_bothσ he₁ he₂ hrel hi
        obtain ⟨f₁, f₂, hf₁, hf₂, hfv, hfu₁, hfu₂⟩ := latom_bothσ he₁ he₂ hrel hf
        have hxi := latom_inPlace hΓ hxs
        have hfi := latom_inPlace hΓ hf
        simp only [LTy.InPlace] at hxi hfi
        have hdp : (lx.join lf).PlainFree := LExpr.plainFree_join hxi.2 hfi.2.2
        simp only [LTy.lsubst, LTy.lsubsts] at hxv hxu₁ hxu₂ hfv hfu₁ hfu₂
        have hte' := Flow.lsubst p.recordDeep σ hte
        have hinit' := Flow.lsubst p.recordDeep σ hinit
        have hres' := Flow.lsubst p.recordDeep σ hres
        cases hxu₁ with
        | list hl₁ =>
          cases hxu₂ with
          | list hl₂ =>
            simp only [evalRhs, hx₁, hx₂, hi₁', hi₂', hf₁, hf₂]
            have happU : ∀ (fv : Val), LValTy p fv (.fn [ta.lsubst p.recordDeep σ, tp.lsubst p.recordDeep σ]
                (tr.lsubst p.recordDeep σ) o g fl (lf.inst (p.sd σ))) →
                ∀ w c x, LValTy p c (ta.lsubst p.recordDeep σ) → LValTy p x (te.lsubst p.recordDeep σ) →
                  UOk (FnAllowed o g) (fun r => LValTy p r (ta.lsubst p.recordDeep σ)) (fl = true) w
                    (applyWith (eval H n p) w fv [c, x]) := fun fv hfu w c x hc hx =>
              UOk.mono (fun _ h => h) (fun _ h => LValTy.flow h hres') id
                (applyWith_uok hrunU hfu (.cons hc (.cons (LValTy.flow hx hte') .nil)) w)
            by_cases hvis : Vis p L ((lx.join lf).inst (p.sd σ))
            · have hvx := Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left lx lf)) hvis
              have hvf := Vis.of_le (LExpr.inst_le_join_inst _ lx lf) hvis
              cases hxv with
              | hidden h => exact absurd hvx h
              | list hall =>
                refine BRel.mono (fun a b h => V.raiseσ _ h) (fun _ => trivial)
                  (walkFold_brel (fun w₁ w₂ c₁ c₂ x y hw' hc hc₁ hc₂ hxy hx hy =>
                    BRel.mono (fun _ _ h => V.flow h hres') id
                      (applyWith_brel hrunB hw' hfv hvf (.cons hc (.cons (V.flow hxy hte') .nil))
                        (.cons hc₁ (.cons (LValTy.flow hx hte') .nil))
                        (.cons hc₂ (.cons (LValTy.flow hy hte') .nil)) True))
                    (happU _ hfu₁) (happU _ hfu₂) hw (V.flow hiv hinit') (LValTy.flow hiu₁ hinit')
                    (LValTy.flow hiu₂ hinit') hall hl₁ hl₂)
            · have hpc' : ¬ Vis p L (((pc.join lx).join lf).inst (p.sd σ)) := fun h =>
                hvis (Vis.of_le (LExpr.le_inst _ (le_pc_xl pc lx lf)) h)
              refine brel_confined hh hw (hof_uok (P := fun v => LValTy p v
                  ((ta.raise (lx.join lf)).lsubst p.recordDeep σ)) (hb.inst _) ?_)
                (hof_uok (P := fun v => LValTy p v
                  ((ta.raise (lx.join lf)).lsubst p.recordDeep σ)) (hb.inst _) ?_)
                hpc' (fun a b => V.hideσ hdp a b hvis) (fun _ => Vis.ofFn_plain) (fun _ => Vis.ofFn_plain)
              · exact UOk.mono (fun _ h => h) (fun _ h => LValTy.lsubst_raise h) id
                  (walkFold_uok (happU _ hfu₁) w₁ (LValTy.flow hiu₁ hinit') hl₁)
              · exact UOk.mono (fun _ h => h) (fun _ h => LValTy.lsubst_raise h) id
                  (walkFold_uok (happU _ hfu₂) w₂ (LValTy.flow hiu₂ hinit') hl₂)
      | @parse _ _ s₀ t hs ht =>
        refine BRelR.of_brel ?_
        cases t <;> simp [LTy.erase] at ht
        rename_i l
        obtain ⟨v₁, v₂, h₁, h₂, hv, hu₁, hu₂⟩ := latom_bothσ he₁ he₂ hrel hs
        simp only [LTy.lsubst] at hv hu₁ hu₂
        cases hu₁
        cases hu₂
        rename_i s₁ s₂
        simp only [evalRhs, h₁, h₂, LTy.lsubst]
        by_cases hd : Vis p L (l.inst (p.sd σ))
        · have hs' : s₁ = s₂ := by have := V.eq_text hv hd; injection this
          subst hs'
          unfold parseInt
          split
          · exact brel_done hw .int
          · exact brel_same hw (fun _ h => nomatch h)
        · have hvf : ¬ Vis p L ((⟨false, HostFx.none, some (pc.join (p.deep (.text l))), []⟩ : Eff).inst (p.sd σ)).failLabel :=
            fun h => hd (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h)
          refine brel_quiet ?_ ?_ (fun _ _ => .hidden hd) (fun _ _ => hvf) (fun _ _ => hvf)
          · unfold parseInt
            split <;> split <;> exact hw
          · unfold parseInt
            split <;> split <;> rfl
      | @eq _ _ a b t₁ t₂ tt ha hb hfa hfb heq =>
        refine BRelR.of_brel ?_
        obtain ⟨a₁, a₂, ha₁, ha₂, hav, hau₁, hau₂⟩ := latom_bothσ he₁ he₂ hrel ha
        obtain ⟨b₁, b₂, hb₁, hb₂, hbv, hbu₁, hbu₂⟩ := latom_bothσ he₁ he₂ hrel hb
        simp only [evalRhs, ha₁, ha₂, hb₁, hb₂, LTy.lsubst]
        refine brel_done hw ?_
        by_cases hd : Vis p L (((p.deep t₁).join (p.deep t₂)).inst (p.sd σ))
        · have he₁' : t₁.eqIn E (p.recordEq (p.records.length + 1)) = true := by
            rw [eqIn_flow hfa]; exact heq
          have he₂' : t₂.eqIn E (p.recordEq (p.records.length + 1)) = true := by
            rw [eqIn_flow hfb]; exact heq
          have hd₁ : Vis p L (p.deep (t₁.lsubst p.recordDeep σ)) :=
            Vis.of_le (LTy.deep_lsub_le hσ hE (latom_inPlace hΓ ha) he₁')
              (Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left _ _)) hd)
          have hd₂ : Vis p L (p.deep (t₂.lsubst p.recordDeep σ)) :=
            Vis.of_le (LTy.deep_lsub_le hσ hE (latom_inPlace hΓ hb) he₂')
              (Vis.of_le (LExpr.inst_le_join_inst _ _ _) hd)
          rw [V.eq_of_deep hwf hav hau₁ hau₂ (LTy.eqIn_lsub hE he₁') hd₁,
            V.eq_of_deep hwf hbv hbu₁ hbu₂ (LTy.eqIn_lsub hE he₂') hd₂]
          exact .bool
        · exact .hidden hd
    · intro σ w₁ w₂ e₁ e₂ Γ E pc e t ε hσ hE hΓ hpc hw he₁ he₂ hrel he
      have hcons : ∀ {x : String} {u : LTy}, u.InPlace → LCtx.InPlace ((x, u) :: Γ) :=
        fun hu q hq => by
          rcases List.mem_cons.mp hq with rfl | hq
          · exact hu
          · exact hΓ q hq
      cases he with
      | atom ha =>
        obtain ⟨v₁, v₂, h₁, h₂, hv, _, _⟩ := latom_bothσ he₁ he₂ hrel ha
        simp only [eval, h₁, h₂, Option.elim]
        exact .of_brel (brel_done hw hv)
      | @letR _ _ x ann value body ty s ε₁ ε₂ hr hflow hb hann =>
        simp only [eval]
        have u₁ := uR (w := w₁) hσ he₁ hr
        have u₂ := uR (w := w₂) hσ he₂ hr
        have hε₁ := (LRhsTy.inPlace hwf hΓ hpc hr).2
        have hpc' : (pc.join ε₁.failLabel).PlainFree :=
          LExpr.plainFree_join hpc (Eff.failLabel_plainFree hε₁)
        have hflow' := Flow.lsubst p.recordDeep σ hflow
        have hnv : ¬ Vis p L (ε₁.inst (p.sd σ)).failLabel →
            ¬ Vis p L ((pc.join ε₁.failLabel).inst (p.sd σ)) := fun hn h =>
          hn (Vis.failLabel_inst.mpr (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))
        refine andThen_brelR (ihR hσ hE hΓ hpc hw he₁ he₂ hrel hr) Vis.union_left_inst
          (fun v₁ v₂ k₁ k₂ hv hw' => ?_) ?_ ?_
        · have hv₁ : LValTy p v₁ (ty.lsubst p.recordDeep σ) := by
            have := u₁.2.1; rw [k₁] at this; exact this
          have hv₂ : LValTy p v₂ (ty.lsubst p.recordDeep σ) := by
            have := u₂.2.1; rw [k₂] at this; exact this
          exact BRelR.mono (fun _ _ h => h) Vis.union_right_inst
            (ihE hσ hE (hcons hann) hpc' hw' (.cons (LValTy.flow hv₁ hflow') he₁)
              (.cons (LValTy.flow hv₂ hflow') he₂) (.cons (V.flow hv hflow') hrel) hb)
        · intro hn v k
          have hv : LValTy p v (ty.lsubst p.recordDeep σ) := by have := u₁.2.1; rw [k] at this; exact this
          obtain ⟨hp, hf, hwc⟩ := confined (L := L) hh
            (uE (w := (evalRhs H n p w₁ e₁ value).2.2) hσ (.cons (LValTy.flow hv hflow') he₁) hb) (hnv hn)
          exact ⟨hp, fun c hc hvf => hf c hc (Vis.union_right_inst hvf), hwc⟩
        · intro hn v k
          have hv : LValTy p v (ty.lsubst p.recordDeep σ) := by have := u₂.2.1; rw [k] at this; exact this
          obtain ⟨hp, hf, hwc⟩ := confined (L := L) hh
            (uE (w := (evalRhs H n p w₂ e₂ value).2.2) hσ (.cons (LValTy.flow hv hflow') he₂) hb) (hnv hn)
          exact ⟨hp, fun c hc hvf => hf c hc (Vis.union_right_inst hvf), hwc⟩
      | @letE _ _ x ann value body ty s ε₁ ε₂ hr hflow hb hann =>
        simp only [eval]
        have u₁ := uE (w := w₁) hσ he₁ hr
        have u₂ := uE (w := w₂) hσ he₂ hr
        have hε₁ := (LExprTy.inPlace hwf hΓ hpc hr).2
        have hpc' : (pc.join ε₁.failLabel).PlainFree :=
          LExpr.plainFree_join hpc (Eff.failLabel_plainFree hε₁)
        have hflow' := Flow.lsubst p.recordDeep σ hflow
        have hnv : ¬ Vis p L (ε₁.inst (p.sd σ)).failLabel →
            ¬ Vis p L ((pc.join ε₁.failLabel).inst (p.sd σ)) := fun hn h =>
          hn (Vis.failLabel_inst.mpr (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))
        refine andThen_brelR (ihE hσ hE hΓ hpc hw he₁ he₂ hrel hr) Vis.union_left_inst
          (fun v₁ v₂ k₁ k₂ hv hw' => ?_) ?_ ?_
        · have hv₁ : LValTy p v₁ (ty.lsubst p.recordDeep σ) := by
            have := u₁.2.1; rw [k₁] at this; exact this
          have hv₂ : LValTy p v₂ (ty.lsubst p.recordDeep σ) := by
            have := u₂.2.1; rw [k₂] at this; exact this
          exact BRelR.mono (fun _ _ h => h) Vis.union_right_inst
            (ihE hσ hE (hcons hann) hpc' hw' (.cons (LValTy.flow hv₁ hflow') he₁)
              (.cons (LValTy.flow hv₂ hflow') he₂) (.cons (V.flow hv hflow') hrel) hb)
        · intro hn v k
          have hv : LValTy p v (ty.lsubst p.recordDeep σ) := by have := u₁.2.1; rw [k] at this; exact this
          obtain ⟨hp, hf, hwc⟩ := confined (L := L) hh
            (uE (w := (eval H n p w₁ e₁ value).2.2) hσ (.cons (LValTy.flow hv hflow') he₁) hb) (hnv hn)
          exact ⟨hp, fun c hc hvf => hf c hc (Vis.union_right_inst hvf), hwc⟩
        · intro hn v k
          have hv : LValTy p v (ty.lsubst p.recordDeep σ) := by have := u₂.2.1; rw [k] at this; exact this
          obtain ⟨hp, hf, hwc⟩ := confined (L := L) hh
            (uE (w := (eval H n p w₂ e₂ value).2.2) hσ (.cons (LValTy.flow hv hflow') he₂) hb) (hnv hn)
          exact ⟨hp, fun c hc hvf => hf c hc (Vis.union_right_inst hvf), hwc⟩
      | @ite _ _ cond thenE elseE lc t₁ t₂ s ε₁ ε₂ hc ht hel h₁ h₂ _ =>
        obtain ⟨c₁, c₂, hc₁, hc₂, hcv, hcu₁, hcu₂⟩ := latom_bothσ he₁ he₂ hrel hc
        have hlc : lc.PlainFree := latom_inPlace hΓ hc
        have hpc' : (pc.join lc).PlainFree := LExpr.plainFree_join hpc hlc
        simp only [LTy.lsubst] at hcv hcu₁ hcu₂
        have U : ∀ {w : World H} {env : Env} {v : Val}, LEnvTy p env (Γ.lsub p.recordDeep σ) →
            evalAtom env cond = some v → LValTy p v (.bool (lc.inst (p.sd σ))) →
            UOk (Allowed p ((pc.join lc).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) (fun _ => True)
              (Fails p ((pc.join lc).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) w
              (eval H (n + 1) p w env (.ite cond thenE elseE)) := by
          intro w env v henv hcv' hvt
          cases hvt with
          | @bool b _ =>
            cases b
            · simp only [eval, hcv']
              exact UOk.mono (allowed_union_right_inst (LExpr.le_refl _)) (fun _ _ => trivial)
                (Fails.union_right_inst (LExpr.le_refl _)) (uE hσ henv hel)
            · simp only [eval, hcv']
              exact UOk.mono allowed_union_left_inst (fun _ _ => trivial) Fails.union_left_inst
                (uE hσ henv ht)
        by_cases hv : Vis p L (lc.inst (p.sd σ))
        · cases hcu₁ with
          | @bool b _ =>
            have := V.eq_bool hcv hv
            subst this
            cases b
            · simp only [eval, hc₁, hc₂]
              exact BRelR.mono (fun _ _ h => V.raiseσ _ (V.flow h (Flow.lsubst _ _ h₂))) Vis.union_right_inst
                (ihE hσ hE hΓ hpc' hw he₁ he₂ hrel hel)
            · simp only [eval, hc₁, hc₂]
              exact BRelR.mono (fun _ _ h => V.raiseσ _ (V.flow h (Flow.lsubst _ _ h₁))) Vis.union_left_inst
                (ihE hσ hE hΓ hpc' hw he₁ he₂ hrel ht)
        · exact .of_brel <| brel_confined hh hw (U he₁ hc₁ hcu₁) (U he₂ hc₂ hcu₂)
            (fun h => hv (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))
            (fun a b => V.hideσ hlc a b hv) id id
      | @caseOption _ _ scr noneE x someE te ls t₁ t₂ s ε₁ ε₂ hs hn hsome h₁ h₂ _ =>
        obtain ⟨c₁, c₂, hc₁, hc₂, hcv, hcu₁, hcu₂⟩ := latom_bothσ he₁ he₂ hrel hs
        have hsi := latom_inPlace hΓ hs
        simp only [LTy.InPlace] at hsi
        have hpc' : (pc.join ls).PlainFree := LExpr.plainFree_join hpc hsi.2
        have hΓ' : LCtx.InPlace ((x, te.raise ls) :: Γ) := hcons (LTy.raise_inPlace hsi.2 hsi.1)
        simp only [LTy.lsubst] at hcv hcu₁ hcu₂
        have U : ∀ {w : World H} {env : Env} {v : Val}, LEnvTy p env (Γ.lsub p.recordDeep σ) →
            evalAtom env scr = some v →
            LValTy p v (.option (te.lsubst p.recordDeep σ) (ls.inst (p.sd σ))) →
            UOk (Allowed p ((pc.join ls).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) (fun _ => True)
              (Fails p ((pc.join ls).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) w
              (eval H (n + 1) p w env (.caseOption scr noneE x someE)) := by
          intro w env v henv hcv' hvt
          cases hvt with
          | none =>
            simp only [eval, hcv']
            exact UOk.mono allowed_union_left_inst (fun _ _ => trivial) Fails.union_left_inst
              (uE hσ henv hn)
          | some hw =>
            simp only [eval, hcv']
            exact UOk.mono (allowed_union_right_inst (LExpr.le_refl _)) (fun _ _ => trivial)
              (Fails.union_right_inst (LExpr.le_refl _))
              (uE hσ (.cons (LValTy.lsubst_raise hw) henv) hsome)
        by_cases hv : Vis p L (ls.inst (p.sd σ))
        · cases hcv with
          | hidden h => exact absurd hv h
          | none =>
            simp only [eval, hc₁, hc₂]
            exact BRelR.mono (fun _ _ h => V.raiseσ _ (V.flow h (Flow.lsubst _ _ h₁))) Vis.union_left_inst
              (ihE hσ hE hΓ hpc' hw he₁ he₂ hrel hn)
          | some hab =>
            cases hcu₁ with
            | some hw₁ =>
              cases hcu₂ with
              | some hw₂ =>
                simp only [eval, hc₁, hc₂]
                exact BRelR.mono (fun _ _ h => V.raiseσ _ (V.flow h (Flow.lsubst _ _ h₂)))
                  Vis.union_right_inst
                  (ihE hσ hE hΓ' hpc' hw (.cons (LValTy.lsubst_raise hw₁) he₁)
                    (.cons (LValTy.lsubst_raise hw₂) he₂) (.cons (V.raiseσ _ hab) hrel) hsome)
        · exact .of_brel <| brel_confined hh hw (U he₁ hc₁ hcu₁) (U he₂ hc₂ hcu₂)
            (fun h => hv (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))
            (fun a b => V.hideσ hsi.2 a b hv) id id
      | @caseResult _ _ scr x okE y errE te ls t₁ t₂ s ε₁ ε₂ hs hok herr h₁ h₂ _ =>
        obtain ⟨c₁, c₂, hc₁, hc₂, hcv, hcu₁, hcu₂⟩ := latom_bothσ he₁ he₂ hrel hs
        have hsi := latom_inPlace hΓ hs
        simp only [LTy.InPlace] at hsi
        have hpc' : (pc.join ls).PlainFree := LExpr.plainFree_join hpc hsi.2
        have hΓ₁ : LCtx.InPlace ((x, te.raise ls) :: Γ) := hcons (LTy.raise_inPlace hsi.2 hsi.1)
        have hΓ₂ : LCtx.InPlace ((y, .int ls) :: Γ) := hcons hsi.2
        simp only [LTy.lsubst] at hcv hcu₁ hcu₂
        have U : ∀ {w : World H} {env : Env} {v : Val}, LEnvTy p env (Γ.lsub p.recordDeep σ) →
            evalAtom env scr = some v →
            LValTy p v (.result (te.lsubst p.recordDeep σ) (ls.inst (p.sd σ))) →
            UOk (Allowed p ((pc.join ls).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) (fun _ => True)
              (Fails p ((pc.join ls).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) w
              (eval H (n + 1) p w env (.caseResult scr x okE y errE)) := by
          intro w env v henv hcv' hvt
          cases hvt with
          | ok hw =>
            simp only [eval, hcv']
            exact UOk.mono allowed_union_left_inst (fun _ _ => trivial) Fails.union_left_inst
              (uE hσ (.cons (LValTy.lsubst_raise hw) henv) hok)
          | err =>
            simp only [eval, hcv']
            exact UOk.mono (allowed_union_right_inst (LExpr.le_refl _)) (fun _ _ => trivial)
              (Fails.union_right_inst (LExpr.le_refl _)) (uE hσ (.cons .int henv) herr)
        by_cases hv : Vis p L (ls.inst (p.sd σ))
        · cases hcv with
          | hidden h => exact absurd hv h
          | ok hab =>
            cases hcu₁ with
            | ok hw₁ =>
              cases hcu₂ with
              | ok hw₂ =>
                simp only [eval, hc₁, hc₂]
                exact BRelR.mono (fun _ _ h => V.raiseσ _ (V.flow h (Flow.lsubst _ _ h₁)))
                  Vis.union_left_inst
                  (ihE hσ hE hΓ₁ hpc' hw (.cons (LValTy.lsubst_raise hw₁) he₁)
                    (.cons (LValTy.lsubst_raise hw₂) he₂) (.cons (V.raiseσ _ hab) hrel) hok)
          | err =>
            simp only [eval, hc₁, hc₂]
            exact BRelR.mono (fun _ _ h => V.raiseσ _ (V.flow h (Flow.lsubst _ _ h₂))) Vis.union_right_inst
              (ihE hσ hE hΓ₂ hpc' hw (.cons .int he₁) (.cons .int he₂) (.cons .int hrel) herr)
        · exact .of_brel <| brel_confined hh hw (U he₁ hc₁ hcu₁) (U he₂ hc₂ hcu₂)
            (fun h => hv (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))
            (fun a b => V.hideσ hsi.2 a b hv) id id
      | fail _ _ _ =>
        simp only [eval]
        exact .of_brel (brel_same hw (fun _ h => nomatch h))
      | @attempt _ _ body ty ε₀ hb =>
        have h := ihE hσ hE hΓ hpc hw he₁ he₂ hrel hb
        simp only [eval]
        generalize eval H n p w₁ e₁ body = r₁ at h ⊢
        generalize eval H n p w₂ e₂ body = r₂ at h ⊢
        obtain ⟨tr₁, o₁, w₁'⟩ := r₁
        obtain ⟨tr₂, o₂, w₂'⟩ := r₂
        by_cases hd : Diverged p L tr₁ tr₂
        · intro _ _
          right
          cases o₁ <;> cases o₂ <;> exact hd
        replace h : BRel p L _ _ (tr₁, o₁, w₁') (tr₂, o₂, w₂') := fun a b => (h a b).resolve_right hd
        refine BRelR.of_brel ?_
        intro ht₁ ht₂
        simp only [LTy.lsubst]
        cases o₁ <;> cases o₂ <;> simp only [Term] at ht₁ ht₂ <;>
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial <;> refine ⟨hp, ?_, hwr⟩ <;>
          simp only [OutRel] at ho ⊢
        · exact .ok ho
        · exact .hidden (fun hv => ho (Vis.failLabel_inst.mpr hv))
        · exact .hidden (fun hv => ho (Vis.failLabel_inst.mpr hv))
        · by_cases hv : Vis p L (ε₀.failLabel.inst (p.sd σ))
          · rw [ho (Vis.failLabel_inst.mpr hv)]; exact .err
          · exact .hidden hv
      | @handle _ _ body x handler t₁ t₂ s ε₁ ε₂ hb hh' h₁ h₂ _ =>
        have h := ihE hσ hE hΓ hpc hw he₁ he₂ hrel hb
        have hε₁ := (LExprTy.inPlace hwf hΓ hpc hb).2
        have hlF := Eff.failLabel_plainFree hε₁
        have hpc' : (pc.join ε₁.failLabel).PlainFree := LExpr.plainFree_join hpc hlF
        have hΓ' : LCtx.InPlace ((x, .int ε₁.failLabel) :: Γ) := hcons hlF
        simp only [eval]
        generalize eval H n p w₁ e₁ body = r₁ at h ⊢
        generalize eval H n p w₂ e₂ body = r₂ at h ⊢
        obtain ⟨tr₁, o₁, w₁'⟩ := r₁
        obtain ⟨tr₂, o₂, w₂'⟩ := r₂
        by_cases hd : Diverged p L tr₁ tr₂
        · intro _ _
          right
          cases o₁ <;> cases o₂ <;> dsimp only
          all_goals first
            | exact hd
            | exact hd.append _ _
            | simpa using hd.append [] _
            | simpa using hd.append _ []
        replace h : BRel p L _ _ (tr₁, o₁, w₁') (tr₂, o₂, w₂') := fun a b => (h a b).resolve_right hd
        cases o₁ <;> cases o₂
        all_goals first
          | (intro ht₁ ht₂; simp only [Term] at ht₁ ht₂; done)
          | skip
        · intro ht₁ ht₂
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          exact .inl ⟨hp, V.raiseσ _ (V.flow ho (Flow.lsubst _ _ h₁)), hwr⟩
        · rename_i a c
          dsimp only
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          simp only [OutRel] at ho
          have ho' : ¬ Vis p L (ε₁.failLabel.inst (p.sd σ)) := fun hv => ho (Vis.failLabel_inst.mpr hv)
          have uh := uE (w := w₂') hσ (.cons (LValTy.int (n := (c : Int))) he₂) hh'
          generalize eval H n p w₂' ((x, Val.int (c : Int)) :: e₂) handler = rh at uh ⊢
          obtain ⟨th, oh, wh⟩ := rh
          obtain ⟨hph, hfh, hwh⟩ := confined (L := L) hh uh
            (fun hv => ho' (Vis.of_le (LExpr.inst_le_join_inst _ _ _) hv))
          refine .of_brel <| brel_quiet (hwr.trans hwh)
            (by simp only [proj_append]; rw [hph, List.append_nil]; exact hp)
            (fun a b => V.hideσ hlF a b ho') (fun _ h => nomatch h) (fun c' hc' => hfh c' hc')
        · rename_i c b
          dsimp only
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          simp only [OutRel] at ho
          have ho' : ¬ Vis p L (ε₁.failLabel.inst (p.sd σ)) := fun hv => ho (Vis.failLabel_inst.mpr hv)
          have uh := uE (w := w₁') hσ (.cons (LValTy.int (n := (c : Int))) he₁) hh'
          generalize eval H n p w₁' ((x, Val.int (c : Int)) :: e₁) handler = rh at uh ⊢
          obtain ⟨th, oh, wh⟩ := rh
          obtain ⟨hph, hfh, hwh⟩ := confined (L := L) hh uh
            (fun hv => ho' (Vis.of_le (LExpr.inst_le_join_inst _ _ _) hv))
          refine .of_brel <| brel_quiet (hwh.symm.trans hwr)
            (by simp only [proj_append]; rw [hph, List.append_nil]; exact hp)
            (fun a b => V.hideσ hlF a b ho') (fun c' hc' => hfh c' hc') (fun _ h => nomatch h)
        · rename_i c₁ c₂
          dsimp only
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          simp only [OutRel] at ho
          have hcv : V p L ((LTy.int ε₁.failLabel).lsubst p.recordDeep σ) (.int (c₁ : Int)) (.int (c₂ : Int)) := by
            simp only [LTy.lsubst]
            by_cases hv : Vis p L (ε₁.failLabel.inst (p.sd σ))
            · rw [ho (Vis.failLabel_inst.mpr hv)]; exact .int
            · exact .hidden hv
          have hh'' := ihE hσ hE hΓ' hpc' hwr (.cons (LValTy.int (n := (c₁ : Int))) he₁)
            (.cons (LValTy.int (n := (c₂ : Int))) he₂) (.cons hcv hrel) hh'
          generalize eval H n p w₁' ((x, Val.int (c₁ : Int)) :: e₁) handler = rh₁ at hh'' ⊢
          generalize eval H n p w₂' ((x, Val.int (c₂ : Int)) :: e₂) handler = rh₂ at hh'' ⊢
          obtain ⟨th₁, oh₁, wh₁⟩ := rh₁
          obtain ⟨th₂, oh₂, wh₂⟩ := rh₂
          intro ht₁ ht₂
          rcases hh'' ht₁ ht₂ with ⟨hph, hoh, hwh⟩ | hdh
          · exact .inl ⟨by simp only [proj_append]; rw [hp, hph],
              OutRel.mono (fun _ _ h => V.raiseσ _ (V.flow h (Flow.lsubst _ _ h₂))) id hoh, hwh⟩
          · exact .inr (Diverged.prepend hp hdh)

end Una.F1H

namespace Una.F1H

open Una.F1 (Label LExpr)

variable {p : Program} {L : Label}

variable {H : Host}

/-- **T6 for F1 without policies, at every ground instance**, with a stateful host: `t6_soundR`,
where no release is seen, so no run diverges at one. -/
theorem t6_sound (hwf : p.LWellFormed) (hpol : p.policies = []) (hh : p.HostWF) (hlaw : H.Lawful)
    (n : Nat) :
    ∀ {σ : List (String × LTy)} {w₁ w₂ : World H} {e₁ e₂ : Env} {Γ : LCtx} {E : List String}
      {pc : LExpr} {e : Expr} {t : LTy} {ε : Eff}, GroundSubst σ → p.EqArgsOK σ E → Γ.InPlace →
      pc.PlainFree → WorldRel p L w₁ w₂ →
      LEnvTy p e₁ (Γ.lsub p.recordDeep σ) → LEnvTy p e₂ (Γ.lsub p.recordDeep σ) →
      EnvRel p L (Γ.lsub p.recordDeep σ) e₁ e₂ → LExprTy p E Γ pc e t ε →
      BRel p L (V p L (t.lsubst p.recordDeep σ)) (Vis p L (ε.inst (p.sd σ)).failLabel)
        (eval H n p w₁ e₁ e) (eval H n p w₂ e₂ e) :=
  fun hσ hE hΓ hpc hw h₁ h₂ hrel he =>
    BRelR.to_brel_nil hpol ((t6_soundR hwf (.of_nil hpol) hh hlaw n).2 hσ hE hΓ hpc hw h₁ h₂ hrel he)

/-- **T6 with a stateful host** (M5): two runs of a body the labeled typing accepts, at a ground
instance of its type parameters, in environments that the observer at `L` cannot tell apart and
from worlds it cannot tell apart, show it the same events and related outcomes whenever both end
at one fuel, and leave worlds it cannot tell apart. -/
theorem t6_host (hwf : p.LWellFormed) (hpol : p.policies = []) (hh : p.HostWF) (hlaw : H.Lawful)
    {σ : List (String × LTy)} {w₁ w₂ : World H} {e₁ e₂ : Env} {Γ : LCtx} {E : List String}
    {pc : LExpr} {e : Expr} {t : LTy} {ε : Eff} (hσ : GroundSubst σ) (hE : p.EqArgsOK σ E)
    (hΓ : Γ.InPlace) (hpc : pc.PlainFree) (hw : WorldRel p L w₁ w₂)
    (h₁ : LEnvTy p e₁ (Γ.lsub p.recordDeep σ)) (h₂ : LEnvTy p e₂ (Γ.lsub p.recordDeep σ))
    (hrel : EnvRel p L (Γ.lsub p.recordDeep σ) e₁ e₂) (he : LExprTy p E Γ pc e t ε) (n : Nat) :
    Term (eval H n p w₁ e₁ e).2.1 → Term (eval H n p w₂ e₂ e).2.1 →
      proj p L (eval H n p w₁ e₁ e).1 = proj p L (eval H n p w₂ e₂ e).1 ∧
        OutRel (V p L (t.lsubst p.recordDeep σ)) (Vis p L (ε.inst (p.sd σ)).failLabel)
          (eval H n p w₁ e₁ e).2.1 (eval H n p w₂ e₂ e).2.1 ∧
        WorldRel p L (eval H n p w₁ e₁ e).2.2 (eval H n p w₂ e₂ e).2.2 :=
  t6_sound hwf hpol hh hlaw n hσ hE hΓ hpc hw h₁ h₂ hrel he

theorem LEnvTy.lsubst_nil : ∀ {env : Env} {Γ : LCtx}, LEnvTy p env Γ →
    LEnvTy p env (Γ.lsub p.recordDeep [])
  | _, _, .nil => .nil
  | _, _, .cons hv henv => .cons (LValTy.flow hv (Flow.lsubst_nil _ _).2) (LEnvTy.lsubst_nil henv)

theorem EnvRel.lsubst_nil : ∀ {Γ : LCtx} {e₁ e₂ : Env}, EnvRel p L Γ e₁ e₂ →
    EnvRel p L (Γ.lsub p.recordDeep []) e₁ e₂
  | _, _, _, .nil => .nil
  | _, _, _, .cons hv hrel => .cons (V.flow hv (Flow.lsubst_nil _ _).2) (EnvRel.lsubst_nil hrel)

/-- **T6 with a stateful host, for a body that is not generic**, stated without instantiation. -/
theorem t6_host_ground (hwf : p.LWellFormed) (hpol : p.policies = []) (hh : p.HostWF)
    (hlaw : H.Lawful) {w₁ w₂ : World H} {e₁ e₂ : Env} {Γ : LCtx} {pc : LExpr} {e : Expr} {t : LTy}
    {ε : Eff} (hΓ : Γ.InPlace) (hpc : pc.PlainFree) (hw : WorldRel p L w₁ w₂) (h₁ : LEnvTy p e₁ Γ)
    (h₂ : LEnvTy p e₂ Γ) (hrel : EnvRel p L Γ e₁ e₂) (he : LExprTy p [] Γ pc e t ε) (n : Nat) :
    Term (eval H n p w₁ e₁ e).2.1 → Term (eval H n p w₂ e₂ e).2.1 →
      proj p L (eval H n p w₁ e₁ e).1 = proj p L (eval H n p w₂ e₂ e).1 ∧
        OutRel (V p L t) (Vis p L ε.failLabel) (eval H n p w₁ e₁ e).2.1 (eval H n p w₂ e₂ e).2.1 ∧
        WorldRel p L (eval H n p w₁ e₁ e).2.2 (eval H n p w₂ e₂ e).2.2 := by
  intro ht₁ ht₂
  obtain ⟨hp, ho, hwr⟩ := t6_host (σ := []) hwf hpol hh hlaw (fun _ h => nomatch h)
    (fun _ h => by simp at h) hΓ hpc hw (LEnvTy.lsubst_nil h₁) (LEnvTy.lsubst_nil h₂)
    (EnvRel.lsubst_nil hrel) he n ht₁ ht₂
  refine ⟨hp, OutRel.mono (fun _ _ h => V.flow h (Flow.lsubst_nil _ _).1) (fun hv => ?_) ho, hwr⟩
  exact Vis.of_le (LExpr.le_trans (failLabel_inst_le _ ε).1 (LExpr.inst_nil_le _)) hv

end Una.F1H
