/-
M3, the binary half for the generic-free part of F1: the two-run relation T6 is stated over.

An observer at label `L` sees a label `e` when `e ⊑ L` (`Vis`), and an event when it sees the
`accepts` of its sink. Two values are related at a labeled type when its top label is hidden, or
they have the same shape and related parts. Two closures are related at a visible function type
when they have the same code and related captured environments: a visible closure comes from the
same program point in both runs, so no step-indexing is needed.
-/
import UnaFormal.F1.UnarySound

namespace Una.F1

variable (p : Program) (L : Label)

/-- The observer at `L` sees `e`. -/
def Vis (e : LExpr) : Prop := p.le e (.ofLabel L)

mutual
  /-- Two runs' values related at a labeled type, for the observer at `L`. -/
  inductive V : LTy → Val → Val → Prop where
    | hidden {t v₁ v₂} : ¬ Vis p L t.top → V t v₁ v₂
    | int {l k} : V (.int l) (.int k) (.int k)
    | bool {l b} : V (.bool l) (.bool b) (.bool b)
    | text {l s} : V (.text l) (.text s) (.text s)
    /-- A `Unit` position carries no information. -/
    | unit {l v₁ v₂} : V (.unit l) v₁ v₂
    | list {te l xs ys} : VAll te xs ys → V (.list te l) (.list xs) (.list ys)
    | none {te l} : V (.option te l) .none .none
    | some {te l a b} : V te a b → V (.option te l) (.some a) (.some b)
    | ok {te l a b} : V te a b → V (.result te l) (.ok a) (.ok b)
    | err {te l c} : V (.result te l) (.err c) (.err c)
    | record {r l rec fs₁ fs₂} :
        p.record r = some rec → VList rec.fields fs₁ fs₂ → V (.record r l) (.record r fs₁) (.record r fs₂)
    | closure {params body env₁ env₂ Γ E tb ε ps r o g f l} :
        EnvRel Γ env₁ env₂ → LEnvTy p env₁ Γ → LEnvTy p env₂ Γ →
        LExprTy p E (params ++ Γ) .bot body tb ε → p.le ε.failLabel .bot →
        Flows p.top ps (params.map Prod.snd) → Flow p.top tb r →
        (ε.op = true → o = true) → (ε.get = true → g = true) → (ε.fails.isSome = true → f = true) →
        V (.fn ps r o g f l) (.closure params body env₁) (.closure params body env₂)

  inductive VAll : LTy → List Val → List Val → Prop where
    | nil {t} : VAll t [] []
    | cons {t a b as bs} : V t a b → VAll t as bs → VAll t (a :: as) (b :: bs)

  inductive VList : List LTy → List Val → List Val → Prop where
    | nil : VList [] [] []
    | cons {t ts a b as bs} : V t a b → VList ts as bs → VList (t :: ts) (a :: as) (b :: bs)

  /-- Two environments related binding by binding. -/
  inductive EnvRel : LCtx → Env → Env → Prop where
    | nil : EnvRel [] [] []
    | cons {x t v₁ v₂ Γ e₁ e₂} : V t v₁ v₂ → EnvRel Γ e₁ e₂ → EnvRel ((x, t) :: Γ) ((x, v₁) :: e₁) ((x, v₂) :: e₂)
end

/-- The observer sees an event when it sees its sink's `accepts`. -/
def EvVis : Event → Prop
  | .out _ => Vis p L (.ofLabel p.acceptsOp)
  | .get _ => Vis p L (.ofLabel p.acceptsGet)

open Classical in
/-- The events the observer sees, in order. -/
noncomputable def proj (tr : Trace) : Trace := tr.filter (fun ev => decide (EvVis p L ev))

/-- A run that ended, with a value or a failure. -/
def Term : Res → Prop
  | .ok _ | .failed _ => True
  | .stuck | .outOfFuel => False

/-- Two outcomes related: values by `R`, failures with the same code when the observer sees
what decides them, and one value against one failure only when it does not. -/
def OutRel (R : Val → Val → Prop) (vf : Prop) : Res → Res → Prop
  | .ok a, .ok b => R a b
  | .failed c₁, .failed c₂ => vf → c₁ = c₂
  | .ok _, .failed _ | .failed _, .ok _ => ¬ vf
  | _, _ => True

/-- Two runs related, termination-insensitively: when both end, the observer sees the same
events and related outcomes. -/
def BRel (R : Val → Val → Prop) (vf : Prop) (r₁ r₂ : Trace × Res) : Prop :=
  Term r₁.2 → Term r₂.2 → proj p L r₁.1 = proj p L r₂.1 ∧ OutRel R vf r₁.2 r₂.2

variable {p L}

/-! ### Visibility -/

theorem Vis.of_le {a b : LExpr} (h : p.le a b) (hb : Vis p L b) : Vis p L a := LExpr.le_trans h hb

theorem Vis.left {a b : LExpr} (h : Vis p L (a.join b)) : Vis p L a := LExpr.le_of_join_left h
theorem Vis.right {a b : LExpr} (h : Vis p L (a.join b)) : Vis p L b := LExpr.le_of_join_right h
theorem Vis.join {a b : LExpr} (ha : Vis p L a) (hb : Vis p L b) : Vis p L (a.join b) :=
  LExpr.join_le ha hb
theorem Vis.bot : Vis p L LExpr.bot := LExpr.bot_le _

theorem LTy.top_raise (e : LExpr) (t : LTy) : (t.raise e).top = t.top.join e := by
  cases t <;> rfl

/-- A flow raises the top label, but at `Unit`, which carries nothing. -/
theorem Flow.top_le {t t' : LTy} (h : Flow p.top t t') (hu : ∀ l, t' ≠ .unit l) :
    p.le t.top t'.top := by
  cases h with
  | unit => exact absurd rfl (hu _)
  | int h | bool h | text h | list _ h | option _ h | result _ h | record h | fn _ _ _ _ _ h
  | var h => exact h

/-! ### The relation along flows -/

mutual
  theorem V.flow : ∀ {t t' : LTy} {a b : Val}, V p L t a b → Flow p.top t t' → V p L t' a b
    | t, t', _, _, .hidden h, f => by
      by_cases hu : ∃ l, t' = .unit l
      · obtain ⟨l, rfl⟩ := hu
        exact .unit
      · exact .hidden (fun hv => h (Vis.of_le (Flow.top_le f (fun l he => hu ⟨l, he⟩)) hv))
    | _, _, _, _, .int, .int _ => .int
    | _, _, _, _, .bool, .bool _ => .bool
    | _, _, _, _, .text, .text _ => .text
    | _, _, _, _, .unit, .unit => .unit
    | _, _, _, _, .list h, .list f _ => .list (VAll.flow h f)
    | _, _, _, _, .none, .option _ _ => .none
    | _, _, _, _, .some h, .option f _ => .some (V.flow h f)
    | _, _, _, _, .ok h, .result f _ => .ok (V.flow h f)
    | _, _, _, _, .err, .result _ _ => .err
    | _, _, _, _, .record hr hfs, .record _ => .record hr hfs
    | _, _, _, _, .closure hrel h₁ h₂ hb hfl hps hr ho hg hf, .fn fps fr fo fg ff _ =>
      .closure hrel h₁ h₂ hb hfl (Flows.trans fps hps) (Flow.trans hr fr) (fo ∘ ho) (fg ∘ hg)
        (ff ∘ hf)

  theorem VAll.flow : ∀ {t t' : LTy} {as bs : List Val}, VAll p L t as bs → Flow p.top t t' →
      VAll p L t' as bs
    | _, _, _, _, .nil, _ => .nil
    | _, _, _, _, .cons h hs, f => .cons (V.flow h f) (VAll.flow hs f)
end

theorem VList.flows : ∀ {ts ts' : List LTy} {as bs : List Val}, VList p L ts as bs →
    Flows p.top ts ts' → VList p L ts' as bs
  | _, _, _, _, .nil, .nil => .nil
  | _, _, _, _, .cons h hs, .cons f fs => .cons (V.flow h f) (VList.flows hs fs)

theorem V.raise {t : LTy} {a b : Val} (e : LExpr) (h : V p L t a b) : V p L (t.raise e) a b :=
  V.flow h (Flow.raise e t)

/-- A type raised by a label the observer does not see relates any two values. -/
theorem V.hide {t : LTy} {e : LExpr} (a b : Val) (he : ¬ Vis p L e) : V p L (t.raise e) a b :=
  .hidden (fun h => he (by rw [LTy.top_raise] at h; exact Vis.right h))

/-- When the observer sees the label a type was raised by, the raise hides nothing more. -/
theorem V.unraise {t : LTy} {e : LExpr} {a b : Val} (he : Vis p L e) (h : V p L (t.raise e) a b) :
    V p L t a b := by
  by_cases ht : Vis p L t.top
  · cases t <;> simp only [LTy.raise] at h <;> cases h <;>
      first
      | (exfalso; rename_i hh; exact hh (by simp only [LTy.top] at ht ⊢; exact Vis.join ht he))
      | exact .int | exact .bool | exact .text | exact .unit | exact .list ‹_› | exact .none
      | exact .some ‹_› | exact .ok ‹_› | exact .err | exact .record ‹_› ‹_›
      | exact .closure ‹_› ‹_› ‹_› ‹_› ‹_› ‹_› ‹_› ‹_› ‹_› ‹_›
  · exact .hidden ht

theorem VList.unraise : ∀ {ts : List LTy} {e : LExpr} {as bs : List Val}, Vis p L e →
    VList p L (ts.map (LTy.raise e)) as bs → VList p L ts as bs
  | [], _, [], [], _, .nil => .nil
  | _ :: _, _, _ :: _, _ :: _, he, .cons h hs => .cons (V.unraise he h) (VList.unraise he hs)

theorem VAll.of_flows : ∀ {ts : List LTy} {t : LTy} {as bs : List Val},
    VList p L ts as bs → (∀ u ∈ ts, Flow p.top u t) → VAll p L t as bs
  | _, _, _, _, .nil, _ => .nil
  | _, _, _, _, .cons h hs, hf =>
    .cons (V.flow h (hf _ List.mem_cons_self))
      (VAll.of_flows hs (fun u hu => hf u (List.mem_cons_of_mem _ hu)))

theorem VAll.length : ∀ {t : LTy} {as bs : List Val}, VAll p L t as bs → as.length = bs.length
  | _, _, _, .nil => rfl
  | _, _, _, .cons _ hs => by simp [VAll.length hs]

theorem VList.length : ∀ {ts : List LTy} {as bs : List Val}, VList p L ts as bs →
    as.length = ts.length ∧ bs.length = ts.length
  | _, _, _, .nil => ⟨rfl, rfl⟩
  | _, _, _, .cons _ hs => by simp [(VList.length hs).1, (VList.length hs).2]

theorem VList.get {t : LTy} : ∀ {ts : List LTy} {as bs : List Val} {i : Nat},
    VList p L ts as bs → ts[i]? = some t → ∃ a b, as[i]? = some a ∧ bs[i]? = some b ∧ V p L t a b
  | _, _, _, _, .nil, hi => by simp at hi
  | _, _, _, 0, .cons h _, hi => by simp at hi ⊢; subst hi; exact h
  | _, _, _, i + 1, .cons _ hs, hi => by simpa using VList.get hs (by simpa using hi)

/-! ### Related environments -/

theorem lookup_rel {x : String} {t : LTy} :
    ∀ {Γ : LCtx} {e₁ e₂ : Env}, EnvRel p L Γ e₁ e₂ → lctxLookup Γ x = some t →
      ∃ v₁ v₂, lookup e₁ x = some v₁ ∧ lookup e₂ x = some v₂ ∧ V p L t v₁ v₂
  | _, _, _, .nil, h => by simp [lctxLookup] at h
  | _, _, _, .cons (x := y) hv hrel, h => by
    simp only [lctxLookup] at h
    simp only [lookup]
    by_cases hx : x = y
    · simp [hx] at h ⊢
      subst h
      exact hv
    · simp [hx] at h ⊢
      simpa using lookup_rel hrel h

theorem latom_rel {Γ : LCtx} {e₁ e₂ : Env} {a : Atom} {t : LTy} (hrel : EnvRel p L Γ e₁ e₂)
    (h : LAtomTy Γ a t) :
    ∃ v₁ v₂, evalAtom e₁ a = some v₁ ∧ evalAtom e₂ a = some v₂ ∧ V p L t v₁ v₂ := by
  cases h with
  | var hx => exact lookup_rel hrel hx
  | int => exact ⟨_, _, rfl, rfl, .int⟩
  | bool => exact ⟨_, _, rfl, rfl, .bool⟩
  | text => exact ⟨_, _, rfl, rfl, .text⟩
  | unit => exact ⟨_, _, rfl, rfl, .unit⟩

theorem latoms_rel {Γ : LCtx} {e₁ e₂ : Env} {as : List Atom} {ts : List LTy}
    (hrel : EnvRel p L Γ e₁ e₂) (h : LAtomsTy Γ as ts) :
    ∃ vs₁ vs₂, evalAtoms e₁ as = some vs₁ ∧ evalAtoms e₂ as = some vs₂ ∧ VList p L ts vs₁ vs₂ := by
  induction h with
  | nil => exact ⟨[], [], rfl, rfl, .nil⟩
  | cons ha _ ih =>
    obtain ⟨v₁, v₂, h₁, h₂, hv⟩ := latom_rel hrel ha
    obtain ⟨vs₁, vs₂, hs₁, hs₂, hvs⟩ := ih
    exact ⟨v₁ :: vs₁, v₂ :: vs₂, by simp [evalAtoms, h₁, hs₁], by simp [evalAtoms, h₂, hs₂],
      .cons hv hvs⟩

theorem bindParams_rel :
    ∀ {params : List (String × LTy)} {vs₁ vs₂ : List Val} {e₁ e₂ : Env} {Γ : LCtx},
      VList p L (params.map Prod.snd) vs₁ vs₂ → EnvRel p L Γ e₁ e₂ →
      EnvRel p L (params ++ Γ) (bindParams params vs₁ e₁) (bindParams params vs₂ e₂)
  | [], [], [], _, _, _, .nil, hrel => by simpa [bindParams] using hrel
  | (x, t) :: params, v₁ :: vs₁, v₂ :: vs₂, _, _, _, .cons hv hvs, hrel => by
    have := bindParams_rel hvs hrel
    simp only [bindParams, List.map_cons, List.zip_cons_cons, List.cons_append] at this ⊢
    exact .cons hv this

/-! ### Projections and related runs -/

theorem proj_append (t₁ t₂ : Trace) : proj p L (t₁ ++ t₂) = proj p L t₁ ++ proj p L t₂ := by
  simp [proj, List.filter_append]

theorem proj_nil : proj p L [] = [] := rfl

theorem proj_hidden {tr : Trace} (h : ∀ ev ∈ tr, ¬ EvVis p L ev) : proj p L tr = [] := by
  simp only [proj, List.filter_eq_nil_iff, decide_eq_true_eq]
  exact h

theorem proj_single_vis {ev : Event} (h : EvVis p L ev) : proj p L [ev] = [ev] := by
  simp [proj, h]

theorem proj_single_hidden {ev : Event} (h : ¬ EvVis p L ev) : proj p L [ev] = [] := by
  simp [proj, h]

/-- At a program counter the observer does not see, a run that keeps to its typing shows no
event, and a failure it ends with is decided above what the observer sees. -/
theorem confined {pc : LExpr} {ε : Eff} {P : Val → Prop} {run : Trace × Res}
    (h : UOk (Allowed p pc ε) P (Fails p pc ε) run) (hpc : ¬ Vis p L pc) :
    proj p L run.1 = [] ∧ (∀ c, run.2 = .failed c → ¬ Vis p L ε.failLabel) := by
  refine ⟨proj_hidden (fun ev hev hvis => ?_), fun c hc hvis => ?_⟩
  · have ha := h.1 ev hev
    cases ev with
    | out _ => exact hpc (Vis.of_le ha.2 hvis)
    | get _ => exact hpc (Vis.of_le ha.2 hvis)
  · have hf := h.2
    rw [hc] at hf
    obtain ⟨l, hl, hle⟩ := hf
    simp only [Eff.failLabel, hl, Option.getD_some] at hvis
    exact hpc (Vis.of_le hle hvis)

theorem OutRel.mono {R R' : Val → Val → Prop} {vf vf' : Prop} (hR : ∀ a b, R a b → R' a b)
    (hvf : vf' → vf) : ∀ {r₁ r₂ : Res}, OutRel R vf r₁ r₂ → OutRel R' vf' r₁ r₂
  | .ok _, .ok _, h => hR _ _ h
  | .failed _, .failed _, h => fun hv => h (hvf hv)
  | .ok _, .failed _, h => fun hv => h (hvf hv)
  | .failed _, .ok _, h => fun hv => h (hvf hv)
  | .ok _, .stuck, _ | .ok _, .outOfFuel, _ | .failed _, .stuck, _ | .failed _, .outOfFuel, _
  | .stuck, _, _ | .outOfFuel, _, _ => trivial

theorem BRel.mono {R R' : Val → Val → Prop} {vf vf' : Prop} (hR : ∀ a b, R a b → R' a b)
    (hvf : vf' → vf) {r₁ r₂ : Trace × Res} (h : BRel p L R vf r₁ r₂) : BRel p L R' vf' r₁ r₂ :=
  fun t₁ t₂ => ⟨(h t₁ t₂).1, OutRel.mono hR hvf (h t₁ t₂).2⟩

theorem andThen_term {first : Trace × Res} {rest : Val → Trace × Res}
    (h : Term (andThen first rest).2) : Term first.2 := by
  obtain ⟨tr, r⟩ := first
  cases r <;> simp_all [andThen, Term]

/-- Sequencing keeps two runs related. When the observer does not see what decides the first
part's failure, a run that went on must show nothing more and fail only unseen. -/
theorem andThen_brel {R₁ R₂ : Val → Val → Prop} {vf vf₁ : Prop} {f₁ f₂ : Trace × Res}
    {k₁ k₂ : Val → Trace × Res} (h₁ : BRel p L R₁ vf₁ f₁ f₂) (hvf : vf → vf₁)
    (h₂ : ∀ v₁ v₂, f₁.2 = .ok v₁ → f₂.2 = .ok v₂ → R₁ v₁ v₂ → BRel p L R₂ vf (k₁ v₁) (k₂ v₂))
    (hc₁ : ¬ vf₁ → ∀ v, f₁.2 = .ok v → proj p L (k₁ v).1 = [] ∧
      ∀ c, (k₁ v).2 = .failed c → ¬ vf)
    (hc₂ : ¬ vf₁ → ∀ v, f₂.2 = .ok v → proj p L (k₂ v).1 = [] ∧
      ∀ c, (k₂ v).2 = .failed c → ¬ vf) :
    BRel p L R₂ vf (andThen f₁ k₁) (andThen f₂ k₂) := by
  intro ht₁ ht₂
  have hf₁ := andThen_term ht₁
  have hf₂ := andThen_term ht₂
  obtain ⟨hp, ho⟩ := h₁ hf₁ hf₂
  obtain ⟨tr₁, r₁⟩ := f₁
  obtain ⟨tr₂, r₂⟩ := f₂
  cases r₁ <;> cases r₂ <;> simp only [Term] at hf₁ hf₂
  · -- both values
    rename_i v₁ v₂
    have hk := h₂ v₁ v₂ rfl rfl ho
    simp only [andThen] at ht₁ ht₂ ⊢
    revert ht₁ ht₂ hk
    cases k₁ v₁
    cases k₂ v₂
    intro ht₁ ht₂ hk
    obtain ⟨hp', ho'⟩ := hk ht₁ ht₂
    exact ⟨by simp only [proj_append]; rw [hp, hp'], ho'⟩
  · -- a value, then a failure
    rename_i v₁ c₂
    have hnv : ¬ vf₁ := ho
    obtain ⟨hpk, hfk⟩ := hc₁ hnv v₁ rfl
    simp only [andThen] at ht₁ ⊢
    revert ht₁ hpk hfk
    cases k₁ v₁ with
    | mk tk rk =>
      intro ht₁ hpk hfk
      refine ⟨by simp only [proj_append]; rw [hpk, List.append_nil]; exact hp, ?_⟩
      cases rk <;> simp only [Term] at ht₁ <;> simp only [OutRel]
      · exact fun hv => hnv (hvf hv)
      · exact fun hv => absurd hv (hfk _ rfl)
  · -- a failure, then a value
    rename_i c₁ v₂
    have hnv : ¬ vf₁ := ho
    obtain ⟨hpk, hfk⟩ := hc₂ hnv v₂ rfl
    simp only [andThen] at ht₂ ⊢
    revert ht₂ hpk hfk
    cases k₂ v₂ with
    | mk tk rk =>
      intro ht₂ hpk hfk
      refine ⟨by simp only [proj_append]; rw [hpk, List.append_nil]; exact hp, ?_⟩
      cases rk <;> simp only [Term] at ht₂ <;> simp only [OutRel]
      · exact fun hv => hnv (hvf hv)
      · exact fun hv => absurd hv (hfk _ rfl)
  · -- both failures
    exact ⟨hp, fun hv => ho (hvf hv)⟩

end Una.F1
