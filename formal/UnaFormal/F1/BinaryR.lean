/-
Two runs related up to their first different release (option B1, `Release.lean`): the relation
`BRelR`, which is `BRel` or a divergence at a visible release, and the lemmas that compose it as
`BinaryLemmas.lean` composes `BRel`. A divergence is a property of a prefix of what the observer
sees, so whatever follows it, in either run, keeps it.
-/
import UnaFormal.F1.BinaryLemmas

namespace Una.F1

variable (p : Program) (L : Label)

/-- **The runs diverge at a release**: after a common prefix of what the observer sees, both
release through the same policy, with different values. -/
def Diverged (t₁ t₂ : Trace) : Prop :=
  ∃ pre pol v₁ v₂ rest₁ rest₂, v₁ ≠ v₂ ∧
    proj p L t₁ = pre ++ Event.release pol v₁ :: rest₁ ∧
    proj p L t₂ = pre ++ Event.release pol v₂ :: rest₂

/-- Two runs related up to their first different release: when both end, the observer sees the
same events and related outcomes (`BRel`), or the runs diverge at a release. -/
def BRelR {H : Host} (R : Val → Val → Prop) (vf : Prop) (r₁ r₂ : Run H) : Prop :=
  Term r₁.2.1 → Term r₂.2.1 →
    (proj p L r₁.1 = proj p L r₂.1 ∧ OutRel R vf r₁.2.1 r₂.2.1 ∧ WorldRel p L r₁.2.2 r₂.2.2) ∨
    Diverged p L r₁.1 r₂.1

variable {p L} {H : Host}

/-- What follows a divergence keeps it. -/
theorem Diverged.append {t₁ t₂ : Trace} (u₁ u₂ : Trace) (h : Diverged p L t₁ t₂) :
    Diverged p L (t₁ ++ u₁) (t₂ ++ u₂) := by
  obtain ⟨pre, pol, v₁, v₂, r₁, r₂, hne, h₁, h₂⟩ := h
  refine ⟨pre, pol, v₁, v₂, r₁ ++ proj p L u₁, r₂ ++ proj p L u₂, hne, ?_, ?_⟩
  · rw [proj_append, h₁]; simp
  · rw [proj_append, h₂]; simp

/-- A common prefix before a divergence keeps it. -/
theorem Diverged.prepend {s₁ s₂ t₁ t₂ : Trace} (hs : proj p L s₁ = proj p L s₂)
    (h : Diverged p L t₁ t₂) : Diverged p L (s₁ ++ t₁) (s₂ ++ t₂) := by
  obtain ⟨pre, pol, v₁, v₂, r₁, r₂, hne, h₁, h₂⟩ := h
  refine ⟨proj p L s₁ ++ pre, pol, v₁, v₂, r₁, r₂, hne, ?_, ?_⟩
  · rw [proj_append, h₁]; simp
  · rw [proj_append, h₂, hs]; simp

theorem BRelR.of_brel {R : Val → Val → Prop} {vf : Prop} {r₁ r₂ : Run H}
    (h : BRel p L R vf r₁ r₂) : BRelR p L R vf r₁ r₂ :=
  fun t₁ t₂ => .inl (h t₁ t₂)

theorem BRelR.mono {R R' : Val → Val → Prop} {vf vf' : Prop} (hR : ∀ a b, R a b → R' a b)
    (hvf : vf' → vf) {r₁ r₂ : Run H} (h : BRelR p L R vf r₁ r₂) : BRelR p L R' vf' r₁ r₂ :=
  fun t₁ t₂ => (h t₁ t₂).imp (fun h => ⟨h.1, OutRel.mono hR hvf h.2.1, h.2.2⟩) id

theorem andThen_fst (first : Run H) (rest : World H → Val → Run H) :
    ∃ u, (andThen first rest).1 = first.1 ++ u := by
  obtain ⟨tr, r, w⟩ := first
  cases r with
  | ok v =>
    simp only [andThen]
    rcases rest w v with ⟨t', r', w'⟩
    exact ⟨t', rfl⟩
  | failed c => exact ⟨[], by simp [andThen]⟩
  | stuck => exact ⟨[], by simp [andThen]⟩
  | outOfFuel => exact ⟨[], by simp [andThen]⟩

/-- **Sequencing keeps two runs related up to their first different release**: a divergence of
the first parts is one of the whole, and so is a divergence of the second parts after first
parts the observer cannot tell apart. -/
theorem andThen_brelR {R₁ R₂ : Val → Val → Prop} {vf vf₁ : Prop} {f₁ f₂ : Run H}
    {k₁ k₂ : World H → Val → Run H} (h₁ : BRelR p L R₁ vf₁ f₁ f₂) (hvf : vf → vf₁)
    (h₂ : ∀ v₁ v₂, f₁.2.1 = .ok v₁ → f₂.2.1 = .ok v₂ → R₁ v₁ v₂ → WorldRel p L f₁.2.2 f₂.2.2 →
      BRelR p L R₂ vf (k₁ f₁.2.2 v₁) (k₂ f₂.2.2 v₂))
    (hc₁ : ¬ vf₁ → ∀ v, f₁.2.1 = .ok v → proj p L (k₁ f₁.2.2 v).1 = [] ∧
      (∀ c, (k₁ f₁.2.2 v).2.1 = .failed c → ¬ vf) ∧ WorldRel p L f₁.2.2 (k₁ f₁.2.2 v).2.2)
    (hc₂ : ¬ vf₁ → ∀ v, f₂.2.1 = .ok v → proj p L (k₂ f₂.2.2 v).1 = [] ∧
      (∀ c, (k₂ f₂.2.2 v).2.1 = .failed c → ¬ vf) ∧ WorldRel p L f₂.2.2 (k₂ f₂.2.2 v).2.2) :
    BRelR p L R₂ vf (andThen f₁ k₁) (andThen f₂ k₂) := by
  intro ht₁ ht₂
  have hf₁ := andThen_term ht₁
  have hf₂ := andThen_term ht₂
  rcases h₁ hf₁ hf₂ with ⟨hp, ho, hw⟩ | hd
  · obtain ⟨tr₁, r₁, w₁⟩ := f₁
    obtain ⟨tr₂, r₂, w₂⟩ := f₂
    cases r₁ <;> cases r₂ <;> simp only [Term] at hf₁ hf₂
    · -- both values
      rename_i v₁ v₂
      have hk := h₂ v₁ v₂ rfl rfl ho hw
      simp only [andThen] at ht₁ ht₂ ⊢
      revert ht₁ ht₂ hk
      rcases k₁ w₁ v₁ with ⟨tk₁, rk₁, wk₁⟩
      rcases k₂ w₂ v₂ with ⟨tk₂, rk₂, wk₂⟩
      intro ht₁ ht₂ hk
      rcases hk ht₁ ht₂ with ⟨hp', ho', hw'⟩ | hd'
      · exact .inl ⟨by simp only [proj_append]; rw [hp, hp'], ho', hw'⟩
      · exact .inr (Diverged.prepend hp hd')
    · -- a value, then a failure
      rename_i v₁ c₂
      have hnv : ¬ vf₁ := ho
      obtain ⟨hpk, hfk, hwk⟩ := hc₁ hnv v₁ rfl
      simp only [andThen] at ht₁ ⊢
      revert ht₁ hpk hfk hwk
      rcases k₁ w₁ v₁ with ⟨tk, rk, wk⟩
      intro ht₁ hpk hfk hwk
      refine .inl ⟨by simp only [proj_append]; rw [hpk, List.append_nil]; exact hp, ?_,
        hwk.symm.trans hw⟩
      cases rk <;> simp only [Term] at ht₁ <;> simp only [OutRel]
      · exact fun hv => hnv (hvf hv)
      · exact fun hv => absurd hv (hfk _ rfl)
    · -- a failure, then a value
      rename_i c₁ v₂
      have hnv : ¬ vf₁ := ho
      obtain ⟨hpk, hfk, hwk⟩ := hc₂ hnv v₂ rfl
      simp only [andThen] at ht₂ ⊢
      revert ht₂ hpk hfk hwk
      rcases k₂ w₂ v₂ with ⟨tk, rk, wk⟩
      intro ht₂ hpk hfk hwk
      refine .inl ⟨by simp only [proj_append]; rw [hpk, List.append_nil]; exact hp, ?_, hw.trans hwk⟩
      cases rk <;> simp only [Term] at ht₂ <;> simp only [OutRel]
      · exact fun hv => hnv (hvf hv)
      · exact fun hv => absurd hv (hfk _ rfl)
    · -- both failures
      exact .inl ⟨hp, fun hv => ho (hvf hv), hw⟩
  · obtain ⟨u₁, hu₁⟩ := andThen_fst f₁ k₁
    obtain ⟨u₂, hu₂⟩ := andThen_fst f₂ k₂
    rw [hu₁, hu₂]
    exact .inr (hd.append u₁ u₂)

/-- A release event. -/
def Event.isRelease : Event → Bool
  | .release _ _ => true
  | _ => false

/-- A divergence shows a release in the first run. -/
theorem Diverged.has_release {t₁ t₂ : Trace} (h : Diverged p L t₁ t₂) :
    ∃ ev ∈ t₁, ev.isRelease = true := by
  obtain ⟨pre, pol, v₁, _, r₁, _, _, h₁, _⟩ := h
  have hm : Event.release pol v₁ ∈ proj p L t₁ := by rw [h₁]; simp
  exact ⟨_, (List.mem_filter.mp hm).1, rfl⟩

/-- Runs without a release cannot diverge at one. -/
theorem BRelR.to_brel {R : Val → Val → Prop} {vf : Prop} {r₁ r₂ : Run H}
    (hn : ∀ ev ∈ r₁.1, ev.isRelease = false) (h : BRelR p L R vf r₁ r₂) : BRel p L R vf r₁ r₂ :=
  fun t₁ t₂ => (h t₁ t₂).resolve_right fun hd => by
    obtain ⟨ev, hev, hr⟩ := hd.has_release
    rw [hn ev hev] at hr
    nomatch hr

/-- Effects without releases allow none. -/
theorem allowed_noRelease {pc : LExpr} {ε : Eff} (hr : ε.rels = []) :
    ∀ ev, Allowed p pc ε ev → ev.isRelease = false
  | .out _, _ | .read _, _ | .write _ _, _ | .req _ _, _ => rfl
  | .release _ _, h => by have := h.1; rw [hr] at this; nomatch this

/-- A run that keeps to a typing whose effects release nothing shows no release. -/
theorem noRelease_of_ugood {pc : LExpr} {t : LTy} {ε : Eff} {s : List (String × LExpr)}
    {w : World H} {run : Run H} (h : UGood p pc t (ε.inst s) w run) (hr : ε.rels = []) :
    ∀ ev ∈ run.1, ev.isRelease = false :=
  fun ev hev => allowed_noRelease (ε := ε.inst s) hr ev (h.1 ev hev)

/-- Equal values of a scalar type are related. -/
theorem V.refl_scalar {r : Val} : ∀ {t : LTy}, t.erase.Scalar → ValTy p r t.erase → V p L t r r
  | .int _, _, h | .bool _, _, h | .text _, _, h => by
    cases h <;> first | exact .int | exact .bool | exact .text
  | .unit _, hs, _ | .list _ _, hs, _ | .option _ _, hs, _ | .result _ _ _, hs, _ | .record _ _, hs, _
  | .fn _ _ _ _ _ _, hs, _ | .var _ _, hs, _ | .tuple _ _, hs, _ => by simp [LTy.erase, Ty.Scalar] at hs

/-- An instance of a ground type has its top label, up to `⊥`. -/
theorem top_lsubst_of_ground {σ : List (String × LTy)} :
    ∀ {t : LTy}, t.Ground → (t.lsubst p.recordDeep σ).top = LExpr.bot.join t.top
  | .int l, h | .bool l, h | .text l, h | .unit l, h | .record _ l, h => by
    simp only [LTy.lsubst, LTy.top]; exact LExpr.inst_symFree _ h.1 h.2
  | .list _ l, h | .option _ l, h => by
    simp only [LTy.lsubst, LTy.top]; exact LExpr.inst_symFree _ h.2.1 h.2.2
  | .result _ _ l, h => by
    simp only [LTy.lsubst, LTy.top]; exact LExpr.inst_symFree _ h.2.2.1 h.2.2.2
  | .fn _ _ _ _ _ l, h => by
    simp only [LTy.lsubst, LTy.top]; exact LExpr.inst_symFree _ h.2.2.1 h.2.2.2
  | .tuple _ l, h => by
    simp only [LTy.lsubst, LTy.top]; exact LExpr.inst_symFree _ h.2.1 h.2.2
  | .var _ _, h => h.elim

open Classical in
/-- Without policies, the observer sees no release, so runs cannot diverge at one. -/
theorem BRelR.to_brel_nil {R : Val → Val → Prop} {vf : Prop} {r₁ r₂ : Run H}
    (hpol : p.policies = []) (h : BRelR p L R vf r₁ r₂) : BRel p L R vf r₁ r₂ :=
  fun t₁ t₂ => (h t₁ t₂).resolve_right fun ⟨_, pol, v₁, _, _, _, _, h₁, _⟩ => by
    have hm : Event.release pol v₁ ∈ proj p L r₁.1 := by rw [h₁]; simp
    have hv := of_decide_eq_true (List.mem_filter.mp hm).2
    obtain ⟨po, hpo, _⟩ := hv
    have := List.mem_of_find?_eq_some hpo
    rw [hpol] at this
    nomatch this

/-! ### Loops (M6) -/

/-- Two states of a loop, as tuples related component by component. -/
def StRel (ts : List LTy) (r₁ r₂ : Val) : Prop :=
  ∃ s₁ s₂, r₁ = .tuple s₁ ∧ r₂ = .tuple s₂ ∧ VList p L ts s₁ s₂

/-- A state of a loop, as a tuple of values of its types. -/
def TupTy (p : Program) (ts : List LTy) (r : Val) : Prop := ∃ s, r = .tuple s ∧ LValsTy p s ts

theorem LValsTy.length' : ∀ {vs : List Val} {ts : List LTy}, LValsTy p vs ts → vs.length = ts.length
  | _, _, .nil => rfl
  | _, _, .cons _ hs => by simp [LValsTy.length' hs]

/-- **Two loops over related elements, from related states, are related up to their first
different release** (M6): by induction on the iterations, each related by `hbody`. When an
iteration's failure is one the observer does not see (`¬ vf`), what the other run does after the
iteration it completed shows the observer nothing (`hr₁`, `hr₂`): T-Loop's premise that an
iteration's failure is decided by no more than what decides whether it runs. -/
theorem walkLoop_brelR {body₁ body₂ : World H → Val → List Val → Run H} {a : LTy} {ts : List LTy}
    {A₁ A₂ : Event → Prop} {F₁ F₂ : Prop} {vf : Prop}
    (hbody : ∀ w₁ w₂ v₁ v₂ st₁ st₂, WorldRel p L w₁ w₂ → V p L a v₁ v₂ → LValTy p v₁ a →
      LValTy p v₂ a → VList p L ts st₁ st₂ → LValsTy p st₁ ts → LValsTy p st₂ ts →
      BRelR p L (StRel (p := p) (L := L) ts) vf (body₁ w₁ v₁ st₁) (body₂ w₂ v₂ st₂))
    (hu₁ : ∀ w v st, LValTy p v a → LValsTy p st ts → UOk A₁ (TupTy p ts) F₁ w (body₁ w v st))
    (hu₂ : ∀ w v st, LValTy p v a → LValsTy p st ts → UOk A₂ (TupTy p ts) F₂ w (body₂ w v st))
    (hr₁ : ¬ vf → ∀ w st xs, LValsTy p st ts → (∀ x ∈ xs, LValTy p x a) →
      proj p L (walkLoop body₁ w st xs).1 = [] ∧ (∀ c, (walkLoop body₁ w st xs).2.1 = .failed c → ¬ vf) ∧
        WorldRel p L w (walkLoop body₁ w st xs).2.2)
    (hr₂ : ¬ vf → ∀ w st xs, LValsTy p st ts → (∀ x ∈ xs, LValTy p x a) →
      proj p L (walkLoop body₂ w st xs).1 = [] ∧ (∀ c, (walkLoop body₂ w st xs).2.1 = .failed c → ¬ vf) ∧
        WorldRel p L w (walkLoop body₂ w st xs).2.2) :
    ∀ {xs ys : List Val} {st₁ st₂ : List Val} {w₁ w₂ : World H}, WorldRel p L w₁ w₂ →
      VList p L ts st₁ st₂ → LValsTy p st₁ ts → LValsTy p st₂ ts → VAll p L a xs ys →
      LValsAll p xs a → LValsAll p ys a →
      BRelR p L (StRel (p := p) (L := L) ts) vf (walkLoop body₁ w₁ st₁ xs) (walkLoop body₂ w₂ st₂ ys)
  | [], [], _, _, _, _, hw, hst, _, _, .nil, _, _ => .of_brel (brel_done hw ⟨_, _, rfl, rfl, hst⟩)
  | _ :: xs, _ :: ys, st₁, st₂, w₁, w₂, hw, hst, hs₁, hs₂, .cons h hs, .cons hx hxs, .cons hy hys => by
    simp only [walkLoop]
    refine andThen_brelR (hbody _ _ _ _ _ _ hw h hx hy hst hs₁ hs₂) id
      (fun r₁ r₂ h₁ h₂ hr hw' => ?_) (fun hn v hv => ?_) (fun hn v hv => ?_)
    · obtain ⟨s₁, s₂, rfl, rfl, hss⟩ := hr
      have t₁ := (hu₁ w₁ _ _ hx hs₁).2.1
      have t₂ := (hu₂ w₂ _ _ hy hs₂).2.1
      rw [h₁] at t₁
      rw [h₂] at t₂
      obtain ⟨_, he₁, ht₁⟩ := t₁
      obtain ⟨_, he₂, ht₂⟩ := t₂
      cases he₁
      cases he₂
      simp only
      rw [ite_eq_left (by rw [LValsTy.length' ht₁, LValsTy.length' hs₁]),
        ite_eq_left (by rw [LValsTy.length' ht₂, LValsTy.length' hs₂])]
      exact walkLoop_brelR hbody hu₁ hu₂ hr₁ hr₂ hw' hss ht₁ ht₂ hs hxs hys
    · have t₁ := (hu₁ w₁ _ _ hx hs₁).2.1
      rw [hv] at t₁
      obtain ⟨s, rfl, hts⟩ := t₁
      simp only
      rw [ite_eq_left (by rw [LValsTy.length' hts, LValsTy.length' hs₁])]
      exact hr₁ hn _ _ _ hts (LValsAll.mem hxs)
    · have t₂ := (hu₂ w₂ _ _ hy hs₂).2.1
      rw [hv] at t₂
      obtain ⟨s, rfl, hts⟩ := t₂
      simp only
      rw [ite_eq_left (by rw [LValsTy.length' hts, LValsTy.length' hs₂])]
      exact hr₂ hn _ _ _ hts (LValsAll.mem hys)

end Una.F1
