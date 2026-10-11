/-
M4: the two-run relation at instances. Labels raised in a generic body hide, raise and un-raise
values at every instance as they do in a ground one, provided the raising label has no plain
symbol (`ℓ_T` only in place); related environments give related atoms at their instances.
-/
import UnaFormal.F1.BinaryLemmas

namespace Una.F1

namespace LExpr

variable {top : Label}

theorem inst_join_le (s : List (String × LExpr)) (a b : LExpr) :
    LExpr.le top ((a.join b).inst s) ((a.inst s).join (b.inst s)) := by
  have hhit : ∀ x ∈ (a.join b).plain ++ (a.join b).star, ∀ l, List.lookup x s = some l →
      LExpr.le top l ((a.inst s).join (b.inst s)) := by
    intro x hx l hl
    simp only [join, List.mem_append] at hx
    have ha : x ∈ a.plain ++ a.star → LExpr.le top l ((a.inst s).join (b.inst s)) := fun h =>
      le_trans (le_trans (le_joinAll (List.mem_filterMap.mpr ⟨x, h, hl⟩)) (le_join_left _ _))
        (le_join_left _ _)
    have hb : x ∈ b.plain ++ b.star → LExpr.le top l ((a.inst s).join (b.inst s)) := fun h =>
      le_trans (le_trans (le_joinAll (List.mem_filterMap.mpr ⟨x, h, hl⟩)) (le_join_left _ _))
        (le_join_right _ _)
    rcases hx with (h | h) | (h | h)
    · exact ha (List.mem_append_left _ h)
    · exact hb (List.mem_append_left _ h)
    · exact ha (List.mem_append_right _ h)
    · exact hb (List.mem_append_right _ h)
  unfold inst
  refine join_le ?_ ?_
  · suffices ∀ es : List LExpr, (∀ e ∈ es, LExpr.le top e ((a.inst s).join (b.inst s))) →
        LExpr.le top (joinAll es) ((a.inst s).join (b.inst s)) by
      apply this
      intro e he
      obtain ⟨x, hx, hl⟩ := List.mem_filterMap.mp he
      exact hhit x hx e hl
    intro es hes
    induction es with
    | nil => exact bot_le _
    | cons e es ih => exact join_le (hes e List.mem_cons_self) (ih (fun e' h => hes e' (List.mem_cons_of_mem _ h)))
  · refine .inr ⟨?_, fun x hx => ?_, fun x hx => ?_⟩
    · simp only [join]
      exact Label.join_le
        (Label.le_trans (Label.le_join_right _ _) (Label.le_join_left _ _))
        (Label.le_trans (Label.le_join_right _ _) (Label.le_join_right _ _))
    · simp only [join, List.mem_filter, List.mem_append] at hx
      obtain ⟨hx | hx, hnc⟩ := hx
      · exact .inl (by simp only [join, List.mem_append, List.mem_filter]; exact .inl (.inr ⟨hx, hnc⟩))
      · exact .inl (by simp only [join, List.mem_append, List.mem_filter]; exact .inr (.inr ⟨hx, hnc⟩))
    · simp only [join, List.mem_filter, List.mem_append] at hx
      obtain ⟨hx | hx, hnc⟩ := hx
      · simp only [join, List.mem_append, List.mem_filter]; exact .inl (.inr ⟨hx, hnc⟩)
      · simp only [join, List.mem_append, List.mem_filter]; exact .inr (.inr ⟨hx, hnc⟩)

theorem inst_le_join_inst (s : List (String × LExpr)) (a b : LExpr) :
    LExpr.le top (b.inst s) ((a.join b).inst s) := le_inst s (le_join_right a b)

theorem dropPlain_join_plainFree (a : String) (x : LExpr) {e : LExpr} (he : e.PlainFree) :
    (x.join e).dropPlain a = (x.dropPlain a).join e := by
  simp only [dropPlain, join, List.filter_append]
  rw [show e.plain = [] from he]
  rfl

end LExpr

theorem LExpr.inst_le_inst_join {top : Label} (s : List (String × LExpr)) (a b : LExpr) :
    LExpr.le top ((a.inst s).join (b.inst s)) ((a.join b).inst s) :=
  LExpr.join_le (LExpr.le_inst s (LExpr.le_join_left a b)) (LExpr.le_inst s (LExpr.le_join_right a b))

theorem LExpr.join_swap_le {top : Label} (a b c : LExpr) :
    LExpr.le top ((a.join b).join c) ((a.join c).join b) :=
  LExpr.join_le (LExpr.join_le (LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_left _ _))
    (LExpr.le_join_right _ _)) (LExpr.le_trans (LExpr.le_join_right _ _) (LExpr.le_join_left _ _))


variable {p : Program} {L : Label}

theorem LTy.top_setTop (x : LExpr) (u : LTy) : (u.setTop x).top = x := by cases u <;> rfl

theorem LTy.setTop_self (u : LTy) : u.setTop u.top = u := by cases u <;> rfl

/-- What hides at a type's instance raised by a label without plain symbol: the label's
instance. -/
theorem LTy.lsubst_raise_top (rd : String → Label) (σ : List (String × LTy)) {e : LExpr}
    (he : e.PlainFree) (t : LTy) :
    p.le (e.inst (σ.map (fun q => (q.1, q.2.deep rd)))) ((t.raise e).lsubst rd σ).top ∧
      p.le ((t.raise e).lsubst rd σ).top
        (((t.lsubst rd σ).top).join (e.inst (σ.map (fun q => (q.1, q.2.deep rd))))) := by
  cases t with
  | var a l =>
    show p.le _ ((LTy.var a (l.join e)).lsubst rd σ).top ∧ p.le ((LTy.var a (l.join e)).lsubst rd σ).top _
    simp only [LTy.lsubst]
    cases List.lookup a σ with
    | some u =>
      simp only [LTy.top_raise, LExpr.dropPlain_join_plainFree a l he]
      refine ⟨LExpr.le_trans (LExpr.inst_le_join_inst _ _ _)
        (LExpr.le_join_right _ _), ?_⟩
      exact LExpr.le_trans (LExpr.join_mono (LExpr.le_refl _) (LExpr.inst_join_le _ _ _))
        (LExpr.join_le (LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_left _ _))
          (LExpr.join_le (LExpr.le_trans (LExpr.le_join_right _ _) (LExpr.le_join_left _ _))
            (LExpr.le_join_right _ _)))
    | none =>
      simp only [LTy.top]
      exact ⟨LExpr.le_inst _ (LExpr.le_join_right _ _), LExpr.inst_join_le _ _ _⟩
  | _ =>
    simp only [LTy.raise, LTy.lsubst, LTy.top]
    exact ⟨LExpr.le_inst _ (LExpr.le_join_right _ _), LExpr.inst_join_le _ _ _⟩

/-- A relation does not depend on its type's top label but through what the observer sees. -/
theorem V.retop {u : LTy} {x y : LExpr} {a b : Val} (h : V p L (u.setTop x) a b)
    (hxy : Vis p L y → Vis p L x) : V p L (u.setTop y) a b := by
  cases u <;> simp only [LTy.setTop] at h ⊢ <;> cases h <;>
    first
    | (apply V.hidden; rename_i hh; intro hv; exact hh (by simp only [LTy.top] at hv ⊢; exact hxy hv))
    | exact .int | exact .bool | exact .text | exact .unit | exact .list ‹_› | exact .none
    | exact .some ‹_› | exact .ok ‹_› | exact .err ‹_› | exact .record ‹_› ‹_›
    | exact .variant ‹_› ‹_› ‹_›
    | exact .closure ‹_› ‹_› ‹_› ‹_› ‹_› ‹_› ‹_› ‹_› ‹_› ‹_› ‹_› ‹_› ‹_› ‹_› ‹_›
    | exact .tuple ‹_›

section
variable {σ : List (String × LTy)}

theorem V.raiseσ {t : LTy} {a b : Val} (e : LExpr) (h : V p L (t.lsubst p.recordDeep σ) a b) :
    V p L ((t.raise e).lsubst p.recordDeep σ) a b :=
  V.flow h (Flow.lsubst _ _ (Flow.raise e t))

/-- A type raised by a label without plain symbol, whose instance the observer does not see,
relates any two values at every instance. -/
theorem V.hideσ {t : LTy} {e : LExpr} (he : e.PlainFree) (a b : Val)
    (hv : ¬ Vis p L (e.inst (p.sd σ))) : V p L ((t.raise e).lsubst p.recordDeep σ) a b :=
  .hidden (fun h => hv (Vis.of_le (LTy.lsubst_raise_top (p := p) _ σ he t).1 h))

/-- When the observer sees the instance of the raising label, the raise hides nothing more. -/
theorem V.unraiseσ {t : LTy} {e : LExpr} (he : e.PlainFree) (hv : Vis p L (e.inst (p.sd σ)))
    {a b : Val} (h : V p L ((t.raise e).lsubst p.recordDeep σ) a b) :
    V p L (t.lsubst p.recordDeep σ) a b := by
  obtain ⟨x, hx⟩ := LTy.lsubst_raise p.recordDeep σ e t
  have htop := (LTy.lsubst_raise_top (p := p) p.recordDeep σ he t).2
  rw [hx, LTy.top_setTop] at htop
  rw [hx] at h
  have := V.retop (y := (t.lsubst p.recordDeep σ).top) h
    (fun hy => Vis.of_le htop (Vis.join hy hv))
  rwa [LTy.setTop_self] at this

theorem VList.unraiseσ {e : LExpr} (he : e.PlainFree) (hv : Vis p L (e.inst (p.sd σ))) :
    ∀ {ps : List LTy} {as bs : List Val},
      VList p L ((ps.map (LTy.raise e)).map (LTy.lsubst p.recordDeep σ)) as bs →
      VList p L (ps.map (LTy.lsubst p.recordDeep σ)) as bs
  | [], [], [], .nil => .nil
  | _ :: _, _ :: _, _ :: _, .cons h hs => .cons (V.unraiseσ he hv h) (VList.unraiseσ he hv hs)

theorem VList.flowsσ {ts us : List LTy} {as bs : List Val}
    (h : VList p L (ts.map (LTy.lsubst p.recordDeep σ)) as bs) (hf : Flows p.top ts us) :
    VList p L (us.map (LTy.lsubst p.recordDeep σ)) as bs := by
  have := Flows.lsubst p.recordDeep σ hf
  rw [LTy.lsubsts_eq, LTy.lsubsts_eq] at this
  exact VList.flows h this

theorem latom_bothσ {Γ : LCtx} {e₁ e₂ : Env} {a : Atom} {t : LTy}
    (he₁ : LEnvTy p e₁ (Γ.lsub p.recordDeep σ)) (he₂ : LEnvTy p e₂ (Γ.lsub p.recordDeep σ))
    (hrel : EnvRel p L (Γ.lsub p.recordDeep σ) e₁ e₂) (h : LAtomTy Γ a t) :
    ∃ v₁ v₂, evalAtom e₁ a = some v₁ ∧ evalAtom e₂ a = some v₂ ∧
      V p L (t.lsubst p.recordDeep σ) v₁ v₂ ∧ LValTy p v₁ (t.lsubst p.recordDeep σ) ∧
      LValTy p v₂ (t.lsubst p.recordDeep σ) := by
  obtain ⟨u₁, hu₁, hut₁⟩ := latom_ltypedσ he₁ h
  obtain ⟨u₂, hu₂, hut₂⟩ := latom_ltypedσ he₂ h
  refine ⟨u₁, u₂, hu₁, hu₂, ?_, hut₁, hut₂⟩
  cases h with
  | var hx =>
    obtain ⟨v₁, v₂, h₁, h₂, hv⟩ := lookup_rel hrel (by rw [lctxLookup_lsub, hx]; rfl)
    simp only [evalAtom] at hu₁ hu₂
    rw [h₁] at hu₁; rw [h₂] at hu₂
    cases hu₁; cases hu₂
    exact hv
  | int => cases hu₁; cases hu₂; exact .int
  | bool => cases hu₁; cases hu₂; exact .bool
  | text => cases hu₁; cases hu₂; exact .text
  | unit => cases hu₁; cases hu₂; exact .unit

theorem latoms_bothσ {Γ : LCtx} {e₁ e₂ : Env} {as : List Atom} {ts : List LTy}
    (he₁ : LEnvTy p e₁ (Γ.lsub p.recordDeep σ)) (he₂ : LEnvTy p e₂ (Γ.lsub p.recordDeep σ))
    (hrel : EnvRel p L (Γ.lsub p.recordDeep σ) e₁ e₂) (h : LAtomsTy Γ as ts) :
    ∃ vs₁ vs₂, evalAtoms e₁ as = some vs₁ ∧ evalAtoms e₂ as = some vs₂ ∧
      VList p L (ts.map (LTy.lsubst p.recordDeep σ)) vs₁ vs₂ ∧
      LValsTy p vs₁ (ts.map (LTy.lsubst p.recordDeep σ)) ∧
      LValsTy p vs₂ (ts.map (LTy.lsubst p.recordDeep σ)) := by
  induction h with
  | nil => exact ⟨[], [], rfl, rfl, .nil, .nil, .nil⟩
  | cons ha _ ih =>
    obtain ⟨v₁, v₂, h₁, h₂, hv, hu₁, hu₂⟩ := latom_bothσ he₁ he₂ hrel ha
    obtain ⟨vs₁, vs₂, hs₁, hs₂, hvs, hus₁, hus₂⟩ := ih
    exact ⟨v₁ :: vs₁, v₂ :: vs₂, by simp [evalAtoms, h₁, hs₁], by simp [evalAtoms, h₂, hs₂],
      .cons hv hvs, .cons hu₁ hus₁, .cons hu₂ hus₂⟩

end

/-! ### `ℓ*` at instances -/

theorem LTy.deep_raise_le (rd : String → Label) {top : Label} (e : LExpr) {u : LTy}
    (hu : ∀ a l, u ≠ .var a l) : LExpr.le top ((u.raise e).deep rd) ((u.deep rd).join e) := by
  cases u with
  | var a l => exact absurd rfl (hu a l)
  | int l | bool l | text l | unit l => exact LExpr.le_refl _
  | list t l | option t l | result t l => exact LExpr.join_swap_le _ _ _
  | record r l => exact LExpr.join_swap_le _ _ _
  | fn ps r o g f l => exact LExpr.join_swap_le _ _ _
  | tuple ts l => exact LExpr.join_swap_le _ _ _

theorem LTy.eqIn_raise (E : List String) (f : String → Bool) (e : LExpr) (u : LTy) :
    (u.raise e).eqIn E f = u.eqIn E f := by
  cases u <;> rfl

theorem lookup_sd {σ : List (String × LTy)} {a : String} {u : LTy} (h : List.lookup a σ = some u) :
    List.lookup a (p.sd σ) = some (u.deep p.recordDeep) := by
  simp [Program.sd, lookup_map, h]

/-- The `ℓ*` of the instance of a type with equality is below the instance of its `ℓ*`. -/
theorem LTy.deep_lsub_le {σ : List (String × LTy)} {E : List String} (hσ : GroundSubst σ)
    (hE : p.EqArgsOK σ E) {n : Nat} :
    ∀ {t : LTy}, t.InPlace → t.eqIn E (p.recordEq n) = true →
      p.le ((t.lsubst p.recordDeep σ).deep p.recordDeep) ((t.deep p.recordDeep).inst (p.sd σ))
  | .int _, _, _ | .bool _, _, _ | .text _, _, _ | .unit _, _, _ => LExpr.le_refl _
  | .record _ _, _, _ => by
    simp only [LTy.lsubst, LTy.deep]
    refine LExpr.join_le (LExpr.le_inst _ (LExpr.le_join_left _ _))
      (LExpr.le_trans (LExpr.le_inst_symFree _ ⟨rfl, rfl⟩).2 (LExpr.le_inst _ (LExpr.le_join_right _ _)))
  | .list t _, hi, he | .option t _, hi, he => by
    simp only [LTy.lsubst, LTy.deep]
    simp only [LTy.eqIn] at he
    exact LExpr.join_le (LExpr.le_inst _ (LExpr.le_join_left _ _))
      (LExpr.le_trans (LTy.deep_lsub_le hσ hE hi.1 he) (LExpr.le_inst _ (LExpr.le_join_right _ _)))
  | .result t e _, hi, he => by
    simp only [LTy.lsubst, LTy.deep]
    simp only [LTy.eqIn, Bool.and_eq_true] at he
    exact LExpr.join_le (LExpr.le_inst _ (LExpr.le_join_left _ _))
      (LExpr.join_le
        (LExpr.le_trans (LTy.deep_lsub_le hσ hE hi.1 he.1)
          (LExpr.le_inst _ (LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_right _ _))))
        (LExpr.le_trans (LTy.deep_lsub_le hσ hE hi.2.1 he.2)
          (LExpr.le_inst _ (LExpr.le_trans (LExpr.le_join_right _ _) (LExpr.le_join_right _ _)))))
  | .fn _ _ _ _ _ _, _, he | .tuple _ _, _, he => by simp [LTy.eqIn] at he
  | .var a l, hi, he => by
    simp only [LTy.eqIn, List.contains_iff_mem] at he
    obtain ⟨u, hu, _⟩ := hE a (by simpa using he)
    simp only [LTy.lsubst, hu]
    have hug : u.Ground := hσ _ (mem_of_lookup hu)
    refine LExpr.le_trans (LTy.deep_raise_le _ _ (fun b k h => by subst h; exact hug.elim)) ?_
    refine LExpr.join_le ?_ (LExpr.le_inst _ ?_)
    · -- `ℓ*(σ(a))` is what the instance gives `ℓ*_a`
      refine LExpr.le_trans (LExpr.le_joinAll (List.mem_filterMap.mpr ⟨a, ?_, lookup_sd hu⟩))
        (LExpr.le_join_left _ _)
      simp [LTy.deep, LExpr.join, LExpr.promote]
    · -- the other labels of the position are below its `ℓ*`
      refine .inr ⟨Label.le_join_left _ _, fun x hx => ?_, fun x hx => ?_⟩
      · obtain ⟨hx, hne⟩ := List.mem_filter.mp hx
        simp only [LTy.InPlace] at hi
        have := hi x hx
        subst this
        simp at hne
      · simp only [LTy.deep, LExpr.join, LExpr.promote, List.mem_append, List.mem_singleton, List.append_nil]; exact .inl (.inr hx)

/-- The instance of a type with equality has equality. -/
theorem LTy.eqIn_lsub {σ : List (String × LTy)} {E : List String} (hE : p.EqArgsOK σ E) :
    ∀ {t : LTy}, t.eqIn E (p.recordEq (p.records.length + 1)) = true →
      (t.lsubst p.recordDeep σ).eqIn [] (p.recordEq (p.records.length + 1)) = true
  | .int _, _ | .bool _, _ | .text _, _ | .unit _, _ | .record _ _, h => by
    first | rfl | exact h
  | .list t _, h | .option t _, h => by
    simp only [LTy.eqIn, LTy.lsubst] at h ⊢; exact LTy.eqIn_lsub hE h
  | .result t e _, h => by
    simp only [LTy.eqIn, LTy.lsubst, Bool.and_eq_true] at h ⊢
    exact ⟨LTy.eqIn_lsub hE h.1, LTy.eqIn_lsub hE h.2⟩
  | .fn _ _ _ _ _ _, h | .tuple _ _, h => by simp [LTy.eqIn] at h
  | .var a _, h => by
    simp only [LTy.eqIn, List.contains_iff_mem] at h
    obtain ⟨u, hu, hue⟩ := hE a (by simpa using h)
    simp only [LTy.lsubst, hu, LTy.eqIn_raise]
    exact hue

/-! ### Failure labels at instances -/

theorem failLabel_inst_le (s : List (String × LExpr)) (ε : Eff) :
    p.le (ε.inst s).failLabel (ε.failLabel.inst s) ∧ p.le (ε.failLabel.inst s) (ε.inst s).failLabel := by
  cases hf : ε.fails with
  | none =>
    simp only [Eff.inst, Eff.failLabel, hf, Option.map_none, Option.getD_none]
    refine ⟨LExpr.bot_le _, ?_⟩
    rw [show LExpr.bot = LExpr.ofLabel Label.bot from rfl, LExpr.inst_ofLabel]
    exact LExpr.join_le (LExpr.le_refl _) (LExpr.le_refl _)
  | some l =>
    simp only [Eff.inst, Eff.failLabel, hf, Option.map_some, Option.getD_some]
    exact ⟨LExpr.le_refl _, LExpr.le_refl _⟩

theorem Vis.failLabel_inst {s : List (String × LExpr)} {ε : Eff} :
    Vis p L (ε.inst s).failLabel ↔ Vis p L (ε.failLabel.inst s) :=
  ⟨fun h => Vis.of_le (failLabel_inst_le s ε).2 h, fun h => Vis.of_le (failLabel_inst_le s ε).1 h⟩

theorem Vis.ofFn_failLabel {s : List (String × LExpr)} {o : Bool} {g : HostFx} {f : Bool} :
    Vis p L ((Eff.ofFn o g f).inst s).failLabel := by
  rw [Vis.failLabel_inst]
  cases f <;> simp only [Eff.ofFn, Eff.failLabel, ite_true, Bool.false_eq_true, ite_false,
    Option.getD_some, Option.getD_none] <;>
    exact Vis.of_le (by rw [show LExpr.bot = LExpr.ofLabel Label.bot from rfl, LExpr.inst_ofLabel]
                        exact LExpr.join_le (LExpr.le_refl _) (LExpr.le_refl _)) Vis.bot

/-- Texts whose every label's instance the observer sees are the same texts in both runs. -/
theorem VList.eq_texts_visσ {σ : List (String × LTy)} : ∀ {ts : List LTy} {as bs : List Val},
    VList p L (ts.map (LTy.lsubst p.recordDeep σ)) as bs →
    (∀ u ∈ ts, u.erase = .text ∧ Vis p L ((p.deep u).inst (p.sd σ))) → as = bs
  | [], [], [], .nil, _ => rfl
  | u :: _, _ :: _, _ :: _, .cons h hs, hall => by
    obtain ⟨he, hv⟩ := hall u List.mem_cons_self
    cases u <;> simp [LTy.erase] at he
    simp only [LTy.lsubst] at h
    rw [V.eq_text h hv, VList.eq_texts_visσ hs (fun w hw => hall w (List.mem_cons_of_mem _ hw))]

/-- **Whether a primitive aborts agrees in both runs when the observer sees its label's
instance** (#526): only `div` and `mod` abort, on their divisor, an `Int` whose label the
primitive's carries. -/
theorem aborts_agreeσ {σ : List (String × LTy)} {pr : Prim} {ts : List LTy} {t : Ty}
    {vs₁ vs₂ : List Val} (hp : PrimTy pr (ts.map LTy.erase) t)
    (hvs : VList p L (ts.map (LTy.lsubst p.recordDeep σ)) vs₁ vs₂)
    (hv : Vis p L ((p.primLabel pr ts).inst (p.sd σ))) : Prim.aborts pr vs₁ = Prim.aborts pr vs₂ := by
  have hdeep : ∀ u ∈ ts, pr ≠ .len → Vis p L ((p.deep u).inst (p.sd σ)) := by
    intro u hu hne
    refine Vis.of_le (LExpr.le_inst _ (LExpr.le_joinAll (List.mem_map_of_mem hu))) ?_
    cases pr <;> first | exact absurd rfl hne | (simpa [Program.primLabel] using hv)
  have hint : ∀ {u : LTy} {a b : Val}, u.erase = .int → V p L (u.lsubst p.recordDeep σ) a b →
      Vis p L ((p.deep u).inst (p.sd σ)) → a = b := by
    intro u a b he h hd
    cases u <;> simp [LTy.erase] at he
    exact V.eq_int h hd
  generalize hes : ts.map LTy.erase = es at hp
  cases hp with
  | div | mod =>
    match ts, hvs, hes with
    | [_, _], .cons h₁ (.cons h₂ .nil), hes =>
      simp only [List.map_cons, List.map_nil, List.cons.injEq] at hes
      rw [hint hes.1 h₁ (hdeep _ (by simp) (fun h => Prim.noConfusion h)),
        hint hes.2.1 h₂ (hdeep _ (by simp) (fun h => Prim.noConfusion h))]
    | [], _, hes | [_], _, hes | _ :: _ :: _ :: _, _, hes => simp at hes
  | slice =>
    have htext : ∀ {u : LTy} {a b : Val}, u.erase = .text → V p L (u.lsubst p.recordDeep σ) a b →
        Vis p L ((p.deep u).inst (p.sd σ)) → a = b := by
      intro u a b he h hd
      cases u <;> simp [LTy.erase] at he
      exact V.eq_text h hd
    match ts, hvs, hes with
    | [_, _, _], .cons h₁ (.cons h₂ (.cons h₃ .nil)), hes =>
      simp only [List.map_cons, List.map_nil, List.cons.injEq] at hes
      rw [htext hes.1 h₁ (hdeep _ (by simp) (fun h => Prim.noConfusion h)),
        hint hes.2.1 h₂ (hdeep _ (by simp) (fun h => Prim.noConfusion h)),
        hint hes.2.2.1 h₃ (hdeep _ (by simp) (fun h => Prim.noConfusion h))]
    | [], _, hes | [_], _, hes | [_, _], _, hes | _ :: _ :: _ :: _ :: _, _, hes => simp at hes
  | _ => simp [Prim.aborts]

/-- A primitive gives the same result in both runs when the observer sees its label's instance. -/
theorem prim_agreeσ {σ : List (String × LTy)} {pr : Prim} {ts : List LTy} {t : Ty}
    {vs₁ vs₂ : List Val} (hp : PrimTy pr (ts.map LTy.erase) t)
    (hvs : VList p L (ts.map (LTy.lsubst p.recordDeep σ)) vs₁ vs₂)
    (hv : Vis p L ((p.primLabel pr ts).inst (p.sd σ))) : evalPrim pr vs₁ = evalPrim pr vs₂ := by
  have hdeep : pr ≠ .len → ∀ u ∈ ts, Vis p L ((p.deep u).inst (p.sd σ)) := by
    intro hne u hu
    refine Vis.of_le (LExpr.le_inst _ (LExpr.le_joinAll (List.mem_map_of_mem hu))) ?_
    cases pr <;> first | exact absurd rfl hne | (simpa [Program.primLabel] using hv)
  generalize hes : ts.map LTy.erase = es at hp
  have hint : ∀ {u : LTy} {a b : Val}, u.erase = .int → V p L (u.lsubst p.recordDeep σ) a b →
      Vis p L ((p.deep u).inst (p.sd σ)) → a = b := by
    intro u a b he h hd
    cases u <;> simp [LTy.erase] at he
    exact V.eq_int h hd
  have hbool : ∀ {u : LTy} {a b : Val}, u.erase = .bool → V p L (u.lsubst p.recordDeep σ) a b →
      Vis p L ((p.deep u).inst (p.sd σ)) → a = b := by
    intro u a b he h hd
    cases u <;> simp [LTy.erase] at he
    exact V.eq_bool h hd
  have htext : ∀ {u : LTy} {a b : Val}, u.erase = .text → V p L (u.lsubst p.recordDeep σ) a b →
      Vis p L ((p.deep u).inst (p.sd σ)) → a = b := by
    intro u a b he h hd
    cases u <;> simp [LTy.erase] at he
    exact V.eq_text h hd
  cases hp with
  | concat hts =>
    rw [VList.eq_texts_visσ hvs (fun u hu => ⟨hts _ (hes ▸ List.mem_map_of_mem hu),
      hdeep (fun h => Prim.noConfusion h) u hu⟩)]
  | add | sub | mul | div | mod | lt | eqInt | min | max =>
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
  | abs =>
    match ts, hvs, hes with
    | [_], .cons h₁ .nil, hes =>
      simp only [List.map_cons, List.map_nil, List.cons.injEq] at hes
      rw [hint hes.1 h₁ (hdeep (fun h => Prim.noConfusion h) _ (by simp))]
    | [], _, hes | _ :: _ :: _, _, hes => simp at hes
  | slice =>
    match ts, hvs, hes with
    | [_, _, _], .cons h₁ (.cons h₂ (.cons h₃ .nil)), hes =>
      simp only [List.map_cons, List.map_nil, List.cons.injEq] at hes
      rw [htext hes.1 h₁ (hdeep (fun h => Prim.noConfusion h) _ (by simp)),
        hint hes.2.1 h₂ (hdeep (fun h => Prim.noConfusion h) _ (by simp)),
        hint hes.2.2.1 h₃ (hdeep (fun h => Prim.noConfusion h) _ (by simp))]
    | [], _, hes | [_], _, hes | [_, _], _, hes | _ :: _ :: _ :: _ :: _, _, hes => simp at hes
  | lenList =>
    match ts, hvs, hes with
    | [u], .cons h₁ .nil, hes =>
      simp only [List.map_cons, List.map_nil, List.cons.injEq] at hes
      cases u <;> simp [LTy.erase] at hes
      simp only [Program.primLabel, LTy.top] at hv
      simp only [LTy.lsubst] at h₁
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
      simp only [LTy.lsubst] at h₁
      rw [V.eq_text h₁ hv]
    | [], _, hes | _ :: _ :: _, _, hes => simp at hes
  | sum =>
    match ts, hvs, hes with
    | [u], .cons h₁ .nil, hes =>
      simp only [List.map_cons, List.map_nil, List.cons.injEq] at hes
      cases u <;> simp [LTy.erase] at hes
      rename_i e lx
      cases e <;> simp [LTy.erase] at hes
      rename_i l
      have hd := hdeep (fun h => Prim.noConfusion h) _ (List.mem_singleton_self _)
      simp only [Program.deep, LTy.deep] at hd
      have hx : Vis p L (lx.inst (p.sd σ)) :=
        Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left _ _)) hd
      have he : Vis p L (l.inst (p.sd σ)) :=
        Vis.of_le (LExpr.le_inst _ (LExpr.le_join_right _ _)) hd
      simp only [LTy.lsubst] at h₁
      cases h₁ with
      | hidden h => exact absurd hx h
      | list hall => rw [VAll.eq_int he hall]
    | [], _, hes | _ :: _ :: _, _, hes => simp at hes

mutual
  theorem LTy.Ground.eqIn {E E' : List String} {f : String → Bool} :
      ∀ {u : LTy}, u.Ground → u.eqIn E f = u.eqIn E' f
    | .var _ _, h => h.elim
    | .int _, _ | .bool _, _ | .text _, _ | .unit _, _ | .record _ _, _ | .fn _ _ _ _ _ _, _
    | .tuple _ _, _ => rfl
    | .list _ _, h | .option _ _, h => by
      simp only [LTy.eqIn]; exact LTy.Ground.eqIn h.1
    | .result _ _ _, h => by
      simp only [LTy.eqIn]; rw [LTy.Ground.eqIn h.1, LTy.Ground.eqIn h.2.1]
end

theorem Vis.ofFn_plain {o : Bool} {g : HostFx} {f : Bool} : Vis p L (Eff.ofFn o g f).failLabel := by
  cases f <;> simp [Eff.ofFn, Eff.failLabel] <;> exact Vis.bot

theorem V.ground_lsubst {σ : List (String × LTy)} {u : LTy} (hu : u.Ground) {a b : Val} :
    V p L (u.lsubst p.recordDeep σ) a b ↔ V p L u a b :=
  ⟨fun h => V.flow h (Flow.lsubst_ground _ σ hu).1, fun h => V.flow h (Flow.lsubst_ground _ σ hu).2⟩

theorem VList.ground_lsubst {σ : List (String × LTy)} : ∀ {us : List LTy} {as bs : List Val},
    (∀ u ∈ us, u.Ground) → VList p L (us.map (LTy.lsubst p.recordDeep σ)) as bs → VList p L us as bs
  | [], [], [], _, .nil => .nil
  | _ :: _, _ :: _, _ :: _, hg, .cons h hs =>
    .cons ((V.ground_lsubst (hg _ List.mem_cons_self)).mp h)
      (VList.ground_lsubst (fun u hu => hg u (List.mem_cons_of_mem _ hu)) hs)

/-- The `Eq` parameters of a called function have type arguments with equality. -/
theorem eqArgsOK_of_call (hwf : p.LWellFormed) {f : String} {fn : Fun} {targs : List LTy}
    {E : List String} (hf : p.find f = some fn) (hlen : targs.length = fn.tparams.length)
    (hg : LTy.GroundAll targs) (heq : fn.EqArgs p E targs) :
    p.EqArgsOK (fn.tparams.zip targs) fn.eqParams := by
  intro a ha
  obtain ⟨u, hu⟩ := lookup_zip_cover hlen a (hwf.eqScoped fn (find_mem hf) a ha)
  refine ⟨u, hu, ?_⟩
  have hm := mem_of_lookup hu
  have := heq _ hm ha
  simp only [Program.eqIn] at this ⊢
  rwa [LTy.Ground.eqIn (zip_ground hg _ hm)] at this

theorem Vis.union_left_inst {s : List (String × LExpr)} {ε₁ ε₂ : Eff}
    (h : Vis p L ((ε₁.union ε₂).inst s).failLabel) : Vis p L (ε₁.inst s).failLabel :=
  Vis.failLabel_inst.mpr (Vis.of_le (LExpr.le_inst s (failLabel_union_left ε₁ ε₂))
    (Vis.failLabel_inst.mp h))

theorem Vis.union_right_inst {s : List (String × LExpr)} {ε₁ ε₂ : Eff}
    (h : Vis p L ((ε₁.union ε₂).inst s).failLabel) : Vis p L (ε₂.inst s).failLabel :=
  Vis.failLabel_inst.mpr (Vis.of_le (LExpr.le_inst s (failLabel_union_right ε₁ ε₂))
    (Vis.failLabel_inst.mp h))

end Una.F1

