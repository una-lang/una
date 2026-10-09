/-
Lemmas for T6 on the generic-free part: values the observer sees agree, effects compose, and two
runs at a program counter it does not see are related whatever they do.
-/
import UnaFormal.F1.Binary

namespace Una.F1

variable {p : Program} {L : Label}

theorem failLabel_union_left (ε₁ ε₂ : Eff) : p.le ε₁.failLabel (ε₁.union ε₂).failLabel := by
  cases h₁ : ε₁.fails with
  | none => simp only [Eff.failLabel, h₁, Option.getD_none]; exact LExpr.bot_le _
  | some l₁ =>
    cases h₂ : ε₂.fails with
    | none => simp [Eff.failLabel, Eff.union, h₁, h₂]; exact LExpr.le_refl _
    | some l₂ => simp [Eff.failLabel, Eff.union, h₁, h₂]; exact LExpr.le_join_left _ _

theorem failLabel_union_right (ε₁ ε₂ : Eff) : p.le ε₂.failLabel (ε₁.union ε₂).failLabel := by
  cases h₂ : ε₂.fails with
  | none => simp only [Eff.failLabel, h₂, Option.getD_none]; exact LExpr.bot_le _
  | some l₂ =>
    cases h₁ : ε₁.fails with
    | none => simp [Eff.failLabel, Eff.union, h₁, h₂]; exact LExpr.le_refl _
    | some l₁ => simp [Eff.failLabel, Eff.union, h₁, h₂]; exact LExpr.le_join_right _ _

theorem Vis.union_left {ε₁ ε₂ : Eff} (h : Vis p L (ε₁.union ε₂).failLabel) : Vis p L ε₁.failLabel :=
  Vis.of_le (failLabel_union_left ε₁ ε₂) h

theorem Vis.union_right {ε₁ ε₂ : Eff} (h : Vis p L (ε₁.union ε₂).failLabel) :
    Vis p L ε₂.failLabel :=
  Vis.of_le (failLabel_union_right ε₁ ε₂) h

/-- Two runs at a program counter the observer does not see, of a type whose values it does not
see, are related. -/
theorem brel_confined {R : Val → Val → Prop} {vf : Prop} {pc : LExpr} {ε₁ ε₂ : Eff}
    {P₁ P₂ : Val → Prop} {r₁ r₂ : Trace × Res}
    (h₁ : UOk (Allowed p pc ε₁) P₁ (Fails p pc ε₁) r₁) (h₂ : UOk (Allowed p pc ε₂) P₂ (Fails p pc ε₂) r₂)
    (hpc : ¬ Vis p L pc) (hR : ∀ a b, R a b) (hvf₁ : vf → Vis p L ε₁.failLabel)
    (hvf₂ : vf → Vis p L ε₂.failLabel) : BRel p L R vf r₁ r₂ := by
  intro ht₁ ht₂
  obtain ⟨hp₁, hf₁⟩ := confined (L := L) h₁ hpc
  obtain ⟨hp₂, hf₂⟩ := confined (L := L) h₂ hpc
  refine ⟨by rw [hp₁, hp₂], ?_⟩
  obtain ⟨tr₁, o₁⟩ := r₁
  obtain ⟨tr₂, o₂⟩ := r₂
  cases o₁ <;> cases o₂ <;> simp only [Term] at ht₁ ht₂ <;> simp only [OutRel]
  · exact hR _ _
  · exact fun hv => hf₂ _ rfl (hvf₂ hv)
  · exact fun hv => hf₁ _ rfl (hvf₁ hv)
  · exact fun hv => absurd (hvf₁ hv) (hf₁ _ rfl)

/-- Scalars the observer sees are equal. -/
theorem V.eq_int {l : LExpr} {a b : Val} (h : V p L (.int l) a b) (hv : Vis p L l) : a = b := by
  cases h with
  | hidden h => exact absurd hv h
  | int => rfl

/-- A list of integers whose labels the observer sees is the same list in both runs. -/
theorem VAll.eq_int {l : LExpr} (hv : Vis p L l) :
    ∀ {as bs : List Val}, VAll p L (.int l) as bs → as = bs
  | _, _, .nil => rfl
  | _, _, .cons h hs => by rw [V.eq_int h hv, VAll.eq_int hv hs]

theorem V.eq_bool {l : LExpr} {a b : Val} (h : V p L (.bool l) a b) (hv : Vis p L l) : a = b := by
  cases h with
  | hidden h => exact absurd hv h
  | bool => rfl

theorem V.eq_text {l : LExpr} {a b : Val} (h : V p L (.text l) a b) (hv : Vis p L l) : a = b := by
  cases h with
  | hidden h => exact absurd hv h
  | text => rfl

theorem LTy.top_le_deep (rd : String → Label) {top : Label} :
    ∀ t : LTy, (∀ a l, t ≠ .var a l) → LExpr.le top t.top (t.deep rd)
  | .int _, _ | .bool _, _ | .text _, _ | .unit _, _ => LExpr.le_refl _
  | .record _ _, _ | .list _ _, _ | .option _ _, _ | .result _ _, _ | .fn _ _ _ _ _ _, _ =>
    LExpr.le_join_left _ _
  | .var a l, h => absurd rfl (h a l)

theorem joinAll_vis {es : List LExpr} (h : Vis p L (LExpr.joinAll es)) : ∀ e ∈ es, Vis p L e := by
  induction es with
  | nil => intro _ h; nomatch h
  | cons e es ih =>
    intro x hx
    simp only [LExpr.joinAll] at h
    rcases List.mem_cons.mp hx with rfl | hx
    · exact Vis.left h
    · exact ih (Vis.right h) x hx

/-- A primitive gives the same result in both runs when the observer sees its label. -/
theorem prim_agree {pr : Prim} {ts : List LTy} {t : Ty} {vs₁ vs₂ : List Val}
    (hp : PrimTy pr (ts.map LTy.erase) t) (hvs : VList p L ts vs₁ vs₂)
    (hv : Vis p L (p.primLabel pr ts)) : evalPrim pr vs₁ = evalPrim pr vs₂ := by
  have hdeep : pr ≠ .len → ∀ u ∈ ts, Vis p L (p.deep u) := by
    intro hne u hu
    apply joinAll_vis (es := ts.map p.deep) _ _ (List.mem_map_of_mem hu)
    cases pr <;> first | exact absurd rfl hne | (simpa [Program.primLabel] using hv)
  generalize hes : ts.map LTy.erase = es at hp
  have hint : ∀ {u : LTy} {a b : Val}, u.erase = .int → V p L u a b → Vis p L (p.deep u) → a = b := by
    intro u a b he h hd
    cases u <;> simp [LTy.erase] at he
    exact V.eq_int h hd
  have hbool : ∀ {u : LTy} {a b : Val}, u.erase = .bool → V p L u a b → Vis p L (p.deep u) →
      a = b := by
    intro u a b he h hd
    cases u <;> simp [LTy.erase] at he
    exact V.eq_bool h hd
  cases hp with
  | add | sub | lt | eqInt =>
    match ts, hvs, hes with
    | [_, _], .cons h₁ (.cons h₂ .nil), hes =>
      simp only [List.map_cons, List.map_nil, List.cons.injEq] at hes
      rw [hint hes.1 h₁ (hdeep (fun h => Prim.noConfusion h) _ (by simp)),
        hint hes.2.1 h₂ (hdeep (fun h => Prim.noConfusion h) _ (by simp))]
    | [], _, hes | [_], _, hes | _ :: _ :: _ :: _, _, hes => simp at hes
  | not =>
    match ts, hvs, hes with
    | [_], .cons h₁ .nil, hes =>
      simp only [List.map_cons, List.map_nil, List.cons.injEq] at hes
      rw [hbool hes.1 h₁ (hdeep (fun h => Prim.noConfusion h) _ (by simp))]
    | [], _, hes | _ :: _ :: _, _, hes => simp at hes
  | lenList =>
    match ts, hvs, hes with
    | [u], .cons h₁ .nil, hes =>
      simp only [List.map_cons, List.map_nil, List.cons.injEq] at hes
      cases u <;> simp [LTy.erase] at hes
      simp only [Program.primLabel, LTy.top] at hv
      cases h₁ with
      | hidden h => exact absurd hv h
      | list hall => simp [evalPrim, VAll.length hall]
    | [], _, hes | _ :: _ :: _, _, hes => simp at hes
  | lenText =>
    match ts, hvs, hes with
    | [u], .cons h₁ .nil, hes =>
      simp only [List.map_cons, List.map_nil, List.cons.injEq] at hes
      cases u <;> simp [LTy.erase] at hes
      simp only [Program.primLabel, LTy.top] at hv
      rw [V.eq_text h₁ hv]
    | [], _, hes | _ :: _ :: _, _, hes => simp at hes
  | sum =>
    match ts, hvs, hes with
    | [u], .cons h₁ .nil, hes =>
      simp only [List.map_cons, List.map_nil, List.cons.injEq] at hes
      cases u <;> simp [LTy.erase] at hes
      rename_i e lx
      cases e <;> simp [LTy.erase] at hes
      have hd := hdeep (fun h => Prim.noConfusion h) _ (List.mem_singleton_self _)
      simp only [Program.deep, LTy.deep] at hd
      cases h₁ with
      | hidden h => exact absurd (Vis.left hd) h
      | list hall => rw [VAll.eq_int (Vis.right hd) hall]
    | [], _, hes | _ :: _ :: _, _, hes => simp at hes

theorem eqIn_flow {E : List String} {f : String → Bool} :
    ∀ {t₁ t₂ : LTy}, Flow p.top t₁ t₂ → t₁.eqIn E f = t₂.eqIn E f
  | _, _, .int _ | _, _, .bool _ | _, _, .text _ | _, _, .unit | _, _, .record _
  | _, _, .fn _ _ _ _ _ _ | _, _, .var _ => rfl
  | _, _, .list h _ | _, _, .option h _ | _, _, .result h _ => by
    simp only [LTy.eqIn]; exact eqIn_flow h

theorem recordDeep_of {r : String} {rec : Record} (h : p.record r = some rec) :
    p.recordDeep r = rec.deep := by
  simp [Program.recordDeep, h]

mutual
  /-- **Values the observer fully sees are equal.** Two related values of a type with equality,
  every label of which the observer sees, are the same value: why `==` over them is labeled
  with every label of their type. -/
  theorem V.eq_of_deep (hwf : p.LWellFormed) :
      ∀ {E : List String} {n : Nat} {t : LTy} {a b : Val}, V p L t a b → LValTy p a t →
        LValTy p b t → t.eqIn E (p.recordEq n) = true → Vis p L (p.deep t) → a = b
    | _, _, t, _, _, .hidden h, ha, _, _, hd => by
      cases t with
      | var => cases ha
      | _ => exact absurd (Vis.of_le (LTy.top_le_deep _ _ (fun _ _ h => LTy.noConfusion h)) hd) h
    | _, _, _, _, _, .int, _, _, _, _ => rfl
    | _, _, _, _, _, .bool, _, _, _, _ => rfl
    | _, _, _, _, _, .text, _, _, _, _ => rfl
    | _, _, _, _, _, .unit, ha, hb, _, _ => by cases ha; cases hb; rfl
    | _, _, _, _, _, .list hall, .list ha, .list hb, he, hd => by
      simp only [LTy.eqIn] at he
      simp only [Program.deep, LTy.deep] at hd
      rw [VAll.eq_of_deep hwf hall ha hb he (Vis.right hd)]
    | _, _, _, _, _, .none, _, _, _, _ => rfl
    | _, _, _, _, _, .some h, .some ha, .some hb, he, hd => by
      simp only [LTy.eqIn] at he
      simp only [Program.deep, LTy.deep] at hd
      rw [V.eq_of_deep hwf h ha hb he (Vis.right hd)]
    | _, _, _, _, _, .ok h, .ok ha, .ok hb, he, hd => by
      simp only [LTy.eqIn] at he
      simp only [Program.deep, LTy.deep] at hd
      rw [V.eq_of_deep hwf h ha hb he (Vis.right hd)]
    | _, _, _, _, _, .err, _, _, _, _ => rfl
    | _, n, _, _, _, .record (r := r) (rec := rec) hrec hfs, .record hra hfa, .record hrb hfb,
      he, hd => by
      rw [hrec] at hra hrb
      cases hra
      cases hrb
      simp only [LTy.eqIn] at he
      cases n with
      | zero => simp [Program.recordEq] at he
      | succ n =>
        simp only [Program.recordEq, hrec, List.all_eq_true] at he
        simp only [Program.deep, LTy.deep, recordDeep_of hrec] at hd
        have hall : ∀ u ∈ rec.fields, Vis p L (p.deep u) := fun u hu =>
          Vis.of_le (hwf.recordDeep rec (record_mem hrec) u hu) (Vis.right hd)
        rw [VList.eq_of_deep hwf hfs hfa hfb he hall]
    | _, _, _, _, _, .closure .., _, _, he, _ => by simp [LTy.eqIn] at he

  theorem VAll.eq_of_deep (hwf : p.LWellFormed) :
      ∀ {E : List String} {n : Nat} {t : LTy} {as bs : List Val}, VAll p L t as bs →
        LValsAll p as t → LValsAll p bs t → t.eqIn E (p.recordEq n) = true →
        Vis p L (p.deep t) → as = bs
    | _, _, _, _, _, .nil, _, _, _, _ => rfl
    | _, _, _, _, _, .cons h hs, .cons ha has, .cons hb hbs, he, hd => by
      rw [V.eq_of_deep hwf h ha hb he hd, VAll.eq_of_deep hwf hs has hbs he hd]

  theorem VList.eq_of_deep (hwf : p.LWellFormed) :
      ∀ {n : Nat} {ts : List LTy} {as bs : List Val}, VList p L ts as bs → LValsTy p as ts →
        LValsTy p bs ts → (∀ u ∈ ts, u.eqIn [] (p.recordEq n) = true) →
        (∀ u ∈ ts, Vis p L (p.deep u)) → as = bs
    | _, _, _, _, .nil, _, _, _, _ => rfl
    | _, _, _, _, .cons h hs, .cons ha has, .cons hb hbs, he, hd => by
      rw [V.eq_of_deep hwf h ha hb (he _ List.mem_cons_self) (hd _ List.mem_cons_self),
        VList.eq_of_deep hwf hs has hbs (fun u hu => he u (List.mem_cons_of_mem _ hu))
          (fun u hu => hd u (List.mem_cons_of_mem _ hu))]
end

end Una.F1

namespace Una.F1

variable {p : Program} {L : Label}

theorem brel_stuck_left {R : Val → Val → Prop} {vf : Prop} {r : Trace × Res} :
    BRel p L R vf stuck r := fun h _ => by simp [stuck, Term] at h

theorem brel_stuck_right {R : Val → Val → Prop} {vf : Prop} {r : Trace × Res} :
    BRel p L R vf r stuck := fun _ h => by simp [stuck, Term] at h

theorem brel_done {R : Val → Val → Prop} {vf : Prop} {a b : Val} (h : R a b) :
    BRel p L R vf (done a) (done b) := fun _ _ => ⟨rfl, h⟩

theorem Flows.length' {top : Label} : ∀ {a b : List LTy}, Flows top a b → a.length = b.length
  | _, _, .nil => rfl
  | _, _, .cons _ fs => by simp [Flows.length' fs]

/-- A failure decided at `⊥` stays visible at every instance. -/
theorem vis_failLabel_inst {ε : Eff} (s : List (String × LExpr)) (h : p.le ε.failLabel .bot) :
    Vis p L (ε.inst s).failLabel := by
  cases hf : ε.fails with
  | none => simp [Eff.inst, Eff.failLabel, hf]; exact Vis.bot
  | some l =>
    simp only [Eff.failLabel, hf, Option.getD_some] at h
    simp only [Eff.inst, Eff.failLabel, hf, Option.map_some, Option.getD_some]
    refine Vis.of_le (LExpr.le_trans (LExpr.le_inst s h) ?_) Vis.bot
    rw [show LExpr.bot = LExpr.ofLabel Label.bot from rfl, LExpr.inst_ofLabel]
    exact LExpr.join_le (LExpr.le_refl _) (LExpr.le_refl _)

/-- Related closures, at a function type the observer sees, called on related arguments, when
every body keeps its runs related at every ground instance. -/
theorem applyWith_brel {run : Env → Expr → Trace × Res}
    (hrun : ∀ {σ : List (String × LTy)} {e₁ e₂ : Env} {Γ : LCtx} {E : List String} {body : Expr}
      {t : LTy} {ε : Eff}, GroundSubst σ → p.EqArgsOK σ E → LCtx.InPlace Γ →
      LEnvTy p e₁ (Γ.lsub p.recordDeep σ) → LEnvTy p e₂ (Γ.lsub p.recordDeep σ) →
      EnvRel p L (Γ.lsub p.recordDeep σ) e₁ e₂ → LExprTy p E Γ .bot body t ε →
      BRel p L (V p L (t.lsubst p.recordDeep σ)) (Vis p L (ε.inst (p.sd σ)).failLabel)
        (run e₁ body) (run e₂ body))
    {ps : List LTy} {r : LTy} {o g f : Bool} {l : LExpr} {c₁ c₂ : Val} {as₁ as₂ : List Val}
    (hc : V p L (.fn ps r o g f l) c₁ c₂) (hl : Vis p L l) (hvs : VList p L ps as₁ as₂)
    (hu₁ : LValsTy p as₁ ps) (hu₂ : LValsTy p as₂ ps) (vf : Prop) :
    BRel p L (V p L r) vf (applyWith run c₁ as₁) (applyWith run c₂ as₂) := by
  cases hc with
  | hidden h => exact absurd hl h
  | @closure params _ cenv₁ cenv₂ Γc _ _ _ _ _ _ _ _ _ σc hσ hE hΓ hrel he₁ he₂ hb hfl _ hps hr _ _ _ =>
    have hlen₁ : as₁.length = _ := (LValsTy.length hu₁).trans (Flows.length' hps)
    have hlen₂ : as₂.length = _ := (LValsTy.length hu₂).trans (Flows.length' hps)
    simp only [List.length_map] at hlen₁ hlen₂
    rw [applyWith, applyWith, ite_eq_left hlen₁, ite_eq_left hlen₂]
    have hen : ∀ {as : List Val} {cenv : Env}, LValsTy p as ps →
        LEnvTy p cenv (Γc.lsub p.recordDeep σc) →
        LEnvTy p (bindParams params as cenv) ((params ++ Γc).lsub p.recordDeep σc) := by
      intro as cenv hu he
      rw [LCtx.lsub_append]
      have := bindParams_ltyped (params := params.lsub p.recordDeep σc)
        (by rw [LCtx.lsub_snd]; exact LValsTy.flows hu hps) he
      simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using this
    have hrel' : EnvRel p L ((params ++ Γc).lsub p.recordDeep σc)
        (bindParams params as₁ cenv₁) (bindParams params as₂ cenv₂) := by
      rw [LCtx.lsub_append]
      have := bindParams_rel (params := params.lsub p.recordDeep σc)
        (by rw [LCtx.lsub_snd]; exact VList.flows hvs hps) hrel
      simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using this
    exact BRel.mono (fun _ _ h => V.flow h hr) (fun _ => vis_failLabel_inst _ hfl)
      (hrun hσ hE hΓ (hen hu₁ he₁) (hen hu₂ he₂) hrel' hb)

/-- Two related lists, as values of a list type. -/
def ListRel (p : Program) (L : Label) (t : LTy) (v₁ v₂ : Val) : Prop :=
  ∃ xs ys, v₁ = .list xs ∧ v₂ = .list ys ∧ VAll p L t xs ys

theorem walkMap_brel {app₁ app₂ : Val → Trace × Res} {a b : LTy}
    (happ : ∀ x y, V p L a x y → LValTy p x a → LValTy p y a →
      BRel p L (V p L b) True (app₁ x) (app₂ y)) :
    ∀ {xs ys : List Val}, VAll p L a xs ys → LValsAll p xs a → LValsAll p ys a →
      BRel p L (ListRel p L b) True (walkMap app₁ xs) (walkMap app₂ ys)
  | [], [], .nil, _, _ => brel_done ⟨[], [], rfl, rfl, .nil⟩
  | _ :: _, _ :: _, .cons h hs, .cons hx hxs, .cons hy hys => by
    simp only [walkMap]
    refine andThen_brel (happ _ _ h hx hy) id (fun r₁ r₂ _ _ hr => ?_) (fun h => absurd trivial h)
      (fun h => absurd trivial h)
    refine andThen_brel (walkMap_brel happ hs hxs hys) id (fun k₁ k₂ _ _ hk => ?_)
      (fun h => absurd trivial h) (fun h => absurd trivial h)
    obtain ⟨zs₁, zs₂, rfl, rfl, hz⟩ := hk
    exact brel_done ⟨_, _, rfl, rfl, .cons hr hz⟩

/-- The lists `filter` gives, related when the observer sees the predicate's results. -/
def FilterRel (p : Program) (L : Label) (t : LTy) (lb : LExpr) (v₁ v₂ : Val) : Prop :=
  Vis p L lb → ListRel p L t v₁ v₂

theorem walkFilter_brel {app₁ app₂ : Val → Trace × Res} {a : LTy} {lb : LExpr}
    (happ : ∀ x y, V p L a x y → LValTy p x a → LValTy p y a →
      BRel p L (V p L (.bool lb)) True (app₁ x) (app₂ y)) :
    ∀ {xs ys : List Val}, VAll p L a xs ys → LValsAll p xs a → LValsAll p ys a →
      BRel p L (FilterRel p L a lb) True (walkFilter app₁ xs) (walkFilter app₂ ys)
  | [], [], .nil, _, _ => brel_done (fun _ => ⟨[], [], rfl, rfl, .nil⟩)
  | _ :: _, _ :: _, .cons h hs, .cons hx hxs, .cons hy hys => by
    simp only [walkFilter]
    refine andThen_brel (happ _ _ h hx hy) id (fun b₁ b₂ e₁ e₂ hb => ?_) (fun h => absurd trivial h)
      (fun h => absurd trivial h)
    clear e₁ e₂
    cases b₁ with
    | bool k₁ =>
      cases b₂ with
      | bool k₂ =>
        refine andThen_brel (walkFilter_brel happ hs hxs hys) id (fun r₁ r₂ f₁ f₂ hk => ?_)
          (fun h => absurd trivial h) (fun h => absurd trivial h)
        clear f₁ f₂
        cases r₁ with
        | list zs₁ =>
          cases r₂ with
          | list zs₂ =>
            refine brel_done (fun hv => ?_)
            obtain ⟨_, _, h₁, h₂, hz⟩ := hk hv
            cases h₁
            cases h₂
            have := V.eq_bool hb hv
            cases this
            cases k₁
            · exact ⟨_, _, rfl, rfl, hz⟩
            · exact ⟨_, _, rfl, rfl, .cons h hz⟩
          | _ => exact brel_stuck_right
        | _ => exact brel_stuck_left
      | _ => exact brel_stuck_right
    | _ => exact brel_stuck_left

/-- The options `first_where` gives. -/
def FirstRel (p : Program) (L : Label) (t : LTy) (v₁ v₂ : Val) : Prop :=
  (v₁ = .none ∧ v₂ = .none) ∨ ∃ x y, v₁ = .some x ∧ v₂ = .some y ∧ V p L t x y

theorem walkFirst_brel {app₁ app₂ : Val → Trace × Res} {a : LTy} {lb : LExpr} (hv : Vis p L lb)
    (happ : ∀ x y, V p L a x y → LValTy p x a → LValTy p y a →
      BRel p L (V p L (.bool lb)) True (app₁ x) (app₂ y)) :
    ∀ {xs ys : List Val}, VAll p L a xs ys → LValsAll p xs a → LValsAll p ys a →
      BRel p L (FirstRel p L a) True (walkFirst app₁ xs) (walkFirst app₂ ys)
  | [], [], .nil, _, _ => brel_done (.inl ⟨rfl, rfl⟩)
  | _ :: _, _ :: _, .cons h hs, .cons hx hxs, .cons hy hys => by
    simp only [walkFirst]
    refine andThen_brel (happ _ _ h hx hy) id (fun b₁ b₂ e₁ e₂ hb => ?_) (fun h => absurd trivial h)
      (fun h => absurd trivial h)
    clear e₁ e₂
    have := V.eq_bool hb hv
    subst this
    cases b₁ with
    | bool k =>
      cases k
      · exact walkFirst_brel hv happ hs hxs hys
      · exact brel_done (.inr ⟨_, _, rfl, rfl, h⟩)
    | _ => exact brel_stuck_left

theorem walkFold_brel {app₁ app₂ : Val → Val → Trace × Res} {a b : LTy} {A : Event → Prop}
    {F : Prop}
    (happ : ∀ c₁ c₂ x y, V p L b c₁ c₂ → LValTy p c₁ b → LValTy p c₂ b → V p L a x y →
      LValTy p x a → LValTy p y a → BRel p L (V p L b) True (app₁ c₁ x) (app₂ c₂ y))
    (hu₁ : ∀ c x, LValTy p c b → LValTy p x a → UOk A (fun r => LValTy p r b) F (app₁ c x))
    (hu₂ : ∀ c x, LValTy p c b → LValTy p x a → UOk A (fun r => LValTy p r b) F (app₂ c x)) :
    ∀ {xs ys : List Val} {c₁ c₂ : Val}, V p L b c₁ c₂ → LValTy p c₁ b → LValTy p c₂ b →
      VAll p L a xs ys → LValsAll p xs a → LValsAll p ys a →
      BRel p L (V p L b) True (walkFold app₁ c₁ xs) (walkFold app₂ c₂ ys)
  | [], [], _, _, hc, _, _, .nil, _, _ => brel_done hc
  | _ :: _, _ :: _, _, _, hc, hc₁, hc₂, .cons h hs, .cons hx hxs, .cons hy hys => by
    simp only [walkFold]
    refine andThen_brel (happ _ _ _ _ hc hc₁ hc₂ h hx hy) id (fun r₁ r₂ h₁ h₂ hr => ?_)
      (fun h => absurd trivial h) (fun h => absurd trivial h)
    have hr₁ := (hu₁ _ _ hc₁ hx).2
    have hr₂ := (hu₂ _ _ hc₂ hy).2
    rw [h₁] at hr₁
    rw [h₂] at hr₂
    exact walkFold_brel happ hu₁ hu₂ hr hr₁ hr₂ hs hxs hys

end Una.F1
