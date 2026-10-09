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
def BRelR (R : Val → Val → Prop) (vf : Prop) (r₁ r₂ : Trace × Res) : Prop :=
  Term r₁.2 → Term r₂.2 → (proj p L r₁.1 = proj p L r₂.1 ∧ OutRel R vf r₁.2 r₂.2) ∨
    Diverged p L r₁.1 r₂.1

variable {p L}

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

theorem BRelR.of_brel {R : Val → Val → Prop} {vf : Prop} {r₁ r₂ : Trace × Res}
    (h : BRel p L R vf r₁ r₂) : BRelR p L R vf r₁ r₂ :=
  fun t₁ t₂ => .inl (h t₁ t₂)

theorem BRelR.mono {R R' : Val → Val → Prop} {vf vf' : Prop} (hR : ∀ a b, R a b → R' a b)
    (hvf : vf' → vf) {r₁ r₂ : Trace × Res} (h : BRelR p L R vf r₁ r₂) : BRelR p L R' vf' r₁ r₂ :=
  fun t₁ t₂ => (h t₁ t₂).imp (fun h => ⟨h.1, OutRel.mono hR hvf h.2⟩) id

theorem andThen_fst (first : Trace × Res) (rest : Val → Trace × Res) :
    ∃ u, (andThen first rest).1 = first.1 ++ u := by
  obtain ⟨tr, r⟩ := first
  cases r with
  | ok v =>
    simp only [andThen]
    rcases rest v with ⟨t', r'⟩
    exact ⟨t', rfl⟩
  | failed c => exact ⟨[], by simp [andThen]⟩
  | stuck => exact ⟨[], by simp [andThen]⟩
  | outOfFuel => exact ⟨[], by simp [andThen]⟩

/-- **Sequencing keeps two runs related up to their first different release**: a divergence of
the first parts is one of the whole, and so is a divergence of the second parts after first
parts the observer cannot tell apart. -/
theorem andThen_brelR {R₁ R₂ : Val → Val → Prop} {vf vf₁ : Prop} {f₁ f₂ : Trace × Res}
    {k₁ k₂ : Val → Trace × Res} (h₁ : BRelR p L R₁ vf₁ f₁ f₂) (hvf : vf → vf₁)
    (h₂ : ∀ v₁ v₂, f₁.2 = .ok v₁ → f₂.2 = .ok v₂ → R₁ v₁ v₂ → BRelR p L R₂ vf (k₁ v₁) (k₂ v₂))
    (hc₁ : ¬ vf₁ → ∀ v, f₁.2 = .ok v → proj p L (k₁ v).1 = [] ∧
      ∀ c, (k₁ v).2 = .failed c → ¬ vf)
    (hc₂ : ¬ vf₁ → ∀ v, f₂.2 = .ok v → proj p L (k₂ v).1 = [] ∧
      ∀ c, (k₂ v).2 = .failed c → ¬ vf) :
    BRelR p L R₂ vf (andThen f₁ k₁) (andThen f₂ k₂) := by
  intro ht₁ ht₂
  have hf₁ := andThen_term ht₁
  have hf₂ := andThen_term ht₂
  rcases h₁ hf₁ hf₂ with ⟨hp, ho⟩ | hd
  · obtain ⟨tr₁, r₁⟩ := f₁
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
      rcases hk ht₁ ht₂ with ⟨hp', ho'⟩ | hd'
      · exact .inl ⟨by simp only [proj_append]; rw [hp, hp'], ho'⟩
      · exact .inr (Diverged.prepend hp hd')
    · -- a value, then a failure
      rename_i v₁ c₂
      have hnv : ¬ vf₁ := ho
      obtain ⟨hpk, hfk⟩ := hc₁ hnv v₁ rfl
      simp only [andThen] at ht₁ ⊢
      revert ht₁ hpk hfk
      cases k₁ v₁ with
      | mk tk rk =>
        intro ht₁ hpk hfk
        refine .inl ⟨by simp only [proj_append]; rw [hpk, List.append_nil]; exact hp, ?_⟩
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
        refine .inl ⟨by simp only [proj_append]; rw [hpk, List.append_nil]; exact hp, ?_⟩
        cases rk <;> simp only [Term] at ht₂ <;> simp only [OutRel]
        · exact fun hv => hnv (hvf hv)
        · exact fun hv => absurd hv (hfk _ rfl)
    · -- both failures
      exact .inl ⟨hp, fun hv => ho (hvf hv)⟩
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
theorem BRelR.to_brel {R : Val → Val → Prop} {vf : Prop} {r₁ r₂ : Trace × Res}
    (hn : ∀ ev ∈ r₁.1, ev.isRelease = false) (h : BRelR p L R vf r₁ r₂) : BRel p L R vf r₁ r₂ :=
  fun t₁ t₂ => (h t₁ t₂).resolve_right fun hd => by
    obtain ⟨ev, hev, hr⟩ := hd.has_release
    rw [hn ev hev] at hr
    nomatch hr

/-- Effects without releases allow none. -/
theorem allowed_noRelease {pc : LExpr} {ε : Eff} (hr : ε.rels = []) :
    ∀ ev, Allowed p pc ε ev → ev.isRelease = false
  | .out _, _ | .get _, _ => rfl
  | .release _ _, h => by have := h.1; rw [hr] at this; nomatch this

/-- A run that keeps to a typing whose effects release nothing shows no release. -/
theorem noRelease_of_ugood {pc : LExpr} {t : LTy} {ε : Eff} {s : List (String × LExpr)}
    {run : Trace × Res} (h : UGood p pc t (ε.inst s) run) (hr : ε.rels = []) :
    ∀ ev ∈ run.1, ev.isRelease = false :=
  fun ev hev => allowed_noRelease (ε := ε.inst s) hr ev (h.1 ev hev)

/-- Equal values of a scalar type are related. -/
theorem V.refl_scalar {r : Val} : ∀ {t : LTy}, t.erase.Scalar → ValTy p r t.erase → V p L t r r
  | .int _, _, h | .bool _, _, h | .text _, _, h => by
    cases h <;> first | exact .int | exact .bool | exact .text
  | .unit _, hs, _ | .list _ _, hs, _ | .option _ _, hs, _ | .result _ _, hs, _ | .record _ _, hs, _
  | .fn _ _ _ _ _ _, hs, _ | .var _ _, hs, _ => by simp [LTy.erase, Ty.Scalar] at hs

/-- An instance of a ground type has its top label, up to `⊥`. -/
theorem top_lsubst_of_ground {σ : List (String × LTy)} :
    ∀ {t : LTy}, t.Ground → (t.lsubst p.recordDeep σ).top = LExpr.bot.join t.top
  | .int l, h | .bool l, h | .text l, h | .unit l, h | .record _ l, h => by
    simp only [LTy.lsubst, LTy.top]; exact LExpr.inst_symFree _ h.1 h.2
  | .list _ l, h | .option _ l, h | .result _ l, h => by
    simp only [LTy.lsubst, LTy.top]; exact LExpr.inst_symFree _ h.2.1 h.2.2
  | .fn _ _ _ _ _ l, h => by
    simp only [LTy.lsubst, LTy.top]; exact LExpr.inst_symFree _ h.2.2.1 h.2.2.2
  | .var _ _, h => h.elim

open Classical in
/-- Without policies, the observer sees no release, so runs cannot diverge at one. -/
theorem BRelR.to_brel_nil {R : Val → Val → Prop} {vf : Prop} {r₁ r₂ : Trace × Res}
    (hpol : p.policies = []) (h : BRelR p L R vf r₁ r₂) : BRel p L R vf r₁ r₂ :=
  fun t₁ t₂ => (h t₁ t₂).resolve_right fun ⟨_, pol, v₁, _, _, _, _, h₁, _⟩ => by
    have hm : Event.release pol v₁ ∈ proj p L r₁.1 := by rw [h₁]; simp
    have hv := of_decide_eq_true (List.mem_filter.mp hm).2
    obtain ⟨po, hpo, _⟩ := hv
    have := List.mem_of_find?_eq_some hpo
    rw [hpol] at this
    nomatch this

end Una.F1
