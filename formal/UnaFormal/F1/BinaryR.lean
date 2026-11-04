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
    (proj p L r₁.1 = proj p L r₂.1 ∧ OutRel p L R vf r₁.2.1 r₂.2.1 ∧ WorldRel p L r₁.2.2 r₂.2.2) ∨
    Diverged p L r₁.1 r₂.1

/-- `BRelR` in a loop's body (ADR 0116 rule 20): runs that end with an exit too, related by
`OutRelX`. -/
def BRelRX {H : Host} (R : Val → Val → Prop) (XE : Bool → List Val → Bool → List Val → Prop)
    (vf : Prop) (r₁ r₂ : Run H) : Prop :=
  TermX r₁.2.1 → TermX r₂.2.1 →
    (proj p L r₁.1 = proj p L r₂.1 ∧ OutRelX p L R XE vf r₁.2.1 r₂.2.1 ∧
      WorldRel p L r₁.2.2 r₂.2.2) ∨ Diverged p L r₁.1 r₂.1

/-- Two exits of a loop's iteration related (ADR 0116 rule 20): the same tag, and states related
at the state's types `ts`. -/
def ExitRel (ts : List LTy) (b₁ : Bool) (vs₁ : List Val) (b₂ : Bool) (vs₂ : List Val) : Prop :=
  b₁ = b₂ ∧ VList p L ts vs₁ vs₂

variable {p L} {H : Host}

/-- An event no observer sees leaves the projection as it was (#561). -/
theorem proj_ghost {ev : Event} (h : ¬ EvVis p L ev) (t : Trace) : proj p L (ev :: t) = proj p L t := by
  simp [proj, List.filter_cons, h]

/-- The ghosts of performances, dropped by every observer's projection (#561). -/
theorem proj_perf (o : String) (t : Trace) : proj p L (.perf o :: t) = proj p L t :=
  proj_ghost (ev := .perf o) (fun h => by simp [EvVis] at h) t

theorem proj_perfFn (f : String) (t : Trace) : proj p L (.perfFn f :: t) = proj p L t :=
  proj_ghost (ev := .perfFn f) (fun h => by simp [EvVis] at h) t

/-- Ghosts in front of two related runs keep them related (#561). -/
theorem BRelR.ghost {R : Val → Val → Prop} {vf : Prop} {r₁ r₂ : Run H} {ev₁ ev₂ : Event}
    (h₁ : ¬ EvVis p L ev₁) (h₂ : ¬ EvVis p L ev₂) (h : BRelR p L R vf r₁ r₂) :
    BRelR p L R vf (Run.ghost ev₁ r₁) (Run.ghost ev₂ r₂) := by
  obtain ⟨tr₁, o₁, w₁⟩ := r₁
  obtain ⟨tr₂, o₂, w₂⟩ := r₂
  intro t₁ t₂
  have hh := h t₁ t₂
  simp only [Run.ghost, Diverged, proj_ghost h₁, proj_ghost h₂] at hh ⊢
  exact hh

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
  | raised _ _ _ => exact ⟨[], by simp [andThen]⟩
  | stuck => exact ⟨[], by simp [andThen]⟩
  | outOfFuel | aborted => exact ⟨[], by simp [andThen]⟩
  | exit _ _ => exact ⟨[], by simp [andThen]⟩

/-- **Sequencing keeps two runs related up to their first different release**: a divergence of
the first parts is one of the whole, and so is a divergence of the second parts after first
parts the observer cannot tell apart. -/
theorem andThen_brelR {R₁ R₂ : Val → Val → Prop} {vf vf₁ : Prop} {f₁ f₂ : Run H}
    {k₁ k₂ : World H → Val → Run H} (h₁ : BRelR p L R₁ vf₁ f₁ f₂) (hvf : vf → vf₁)
    (h₂ : ∀ v₁ v₂, f₁.2.1 = .ok v₁ → f₂.2.1 = .ok v₂ → R₁ v₁ v₂ → WorldRel p L f₁.2.2 f₂.2.2 →
      BRelR p L R₂ vf (k₁ f₁.2.2 v₁) (k₂ f₂.2.2 v₂))
    (hc₁ : ¬ vf₁ → ∀ v, f₁.2.1 = .ok v → proj p L (k₁ f₁.2.2 v).1 = [] ∧
      ((k₁ f₁.2.2 v).2.1.IsFail → ¬ vf) ∧ WorldRel p L f₁.2.2 (k₁ f₁.2.2 v).2.2)
    (hc₂ : ¬ vf₁ → ∀ v, f₂.2.1 = .ok v → proj p L (k₂ f₂.2.2 v).1 = [] ∧
      ((k₂ f₂.2.2 v).2.1.IsFail → ¬ vf) ∧ WorldRel p L f₂.2.2 (k₂ f₂.2.2 v).2.2) :
    BRelR p L R₂ vf (andThen f₁ k₁) (andThen f₂ k₂) := by
  intro ht₁ ht₂
  have hf₁ := andThen_term ht₁
  have hf₂ := andThen_term ht₂
  rcases h₁ hf₁ hf₂ with ⟨hp, ho, hw⟩ | hd
  · obtain ⟨tr₁, r₁, w₁⟩ := f₁
    obtain ⟨tr₂, r₂, w₂⟩ := f₂
    simp only at hf₁ hf₂ ho h₂ hc₁ hc₂ hp hw
    have hfail : ∀ {r : Res}, Term r → (∀ v, r ≠ .ok v) → r.IsFail := by
      intro r hr hn; cases r <;> simp_all [Term, Res.IsFail]
    by_cases ok₁ : ∃ v, r₁ = .ok v <;> by_cases ok₂ : ∃ v, r₂ = .ok v
    · -- both values
      obtain ⟨v₁, rfl⟩ := ok₁
      obtain ⟨v₂, rfl⟩ := ok₂
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
      obtain ⟨v₁, rfl⟩ := ok₁
      have hn₂ : ∀ v, r₂ ≠ .ok v := fun v h => ok₂ ⟨v, h⟩
      have hF₂ := hfail hf₂ hn₂
      have hnv : ¬ vf₁ := by cases r₂ <;> simp_all [OutRel, Res.IsFail]
      obtain ⟨hpk, hfk, hwk⟩ := hc₁ hnv v₁ rfl
      rw [andThen_not_ok hn₂]
      simp only [andThen] at ht₁ ⊢
      revert ht₁ hpk hfk hwk
      rcases k₁ w₁ v₁ with ⟨tk, rk, wk⟩
      intro ht₁ hpk hfk hwk
      refine .inl ⟨by simp only [proj_append]; rw [hpk, List.append_nil]; exact hp, ?_,
        hwk.symm.trans hw⟩
      simp only at ht₁ hfk ⊢
      have hnvf : ¬ vf := fun hv => hnv (hvf hv)
      cases rk <;> cases r₂ <;> simp_all [OutRel, Term, Res.IsFail]
    · -- a failure, then a value
      obtain ⟨v₂, rfl⟩ := ok₂
      have hn₁ : ∀ v, r₁ ≠ .ok v := fun v h => ok₁ ⟨v, h⟩
      have hnv : ¬ vf₁ := by cases r₁ <;> simp_all [OutRel, Res.IsFail, Term]
      obtain ⟨hpk, hfk, hwk⟩ := hc₂ hnv v₂ rfl
      rw [andThen_not_ok hn₁]
      simp only [andThen] at ht₂ ⊢
      revert ht₂ hpk hfk hwk
      rcases k₂ w₂ v₂ with ⟨tk, rk, wk⟩
      intro ht₂ hpk hfk hwk
      refine .inl ⟨by simp only [proj_append]; rw [hpk, List.append_nil]; exact hp, ?_, hw.trans hwk⟩
      simp only at ht₂ hfk ⊢
      have hnvf : ¬ vf := fun hv => hnv (hvf hv)
      cases rk <;> cases r₁ <;> simp_all [OutRel, Term, Res.IsFail]
    · -- both failures
      have hn₁ : ∀ v, r₁ ≠ .ok v := fun v h => ok₁ ⟨v, h⟩
      have hn₂ : ∀ v, r₂ ≠ .ok v := fun v h => ok₂ ⟨v, h⟩
      rw [andThen_not_ok hn₁, andThen_not_ok hn₂]
      refine .inl ⟨hp, ?_, hw⟩
      cases r₁ <;> cases r₂ <;> simp only [Term] at hf₁ hf₂ <;>
        first
        | exact absurd rfl (hn₁ _)
        | exact absurd rfl (hn₂ _)
        | exact fun hv => ho (hvf hv)
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
  | .out _, _ | .read _, _ | .write _ _, _ | .req _ _, _ | .perf _, _ | .perfFn _, _ | .keyUse _, _ => rfl
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

end Una.F1

namespace Una.F1

variable {p : Program} {L : Label} {H : Host}

/-! ### ADR 0116: exits of a loop's iteration, and the loops of `until` and `ever` -/

theorem BRelRX.mono {R R' : Val → Val → Prop} {XE XE' : Bool → List Val → Bool → List Val → Prop}
    {vf vf' : Prop} (hR : ∀ a b, R a b → R' a b)
    (hXE : ∀ b₁ vs₁ b₂ vs₂, XE b₁ vs₁ b₂ vs₂ → XE' b₁ vs₁ b₂ vs₂) (hvf : vf' → vf) {r₁ r₂ : Run H}
    (h : BRelRX p L R XE vf r₁ r₂) : BRelRX p L R' XE' vf' r₁ r₂ :=
  fun t₁ t₂ => (h t₁ t₂).imp (fun h => ⟨h.1, OutRelX.mono hR hXE hvf h.2.1, h.2.2⟩) id

theorem termX_of_term {r : Res} (h : Term r) : TermX r := by
  cases r <;> simp_all [Term, TermX]

theorem noExit_of_term {r : Res} (h : Term r) : ∀ b vs, r ≠ .exit b vs := by
  intro b vs he; subst he; exact h

theorem term_of_termX {r : Res} (h : TermX r) (hn : ∀ b vs, r ≠ .exit b vs) : Term r := by
  cases r <;> simp_all [Term, TermX]

/-- A relation in a loop's body gives one outside it, for runs that end other than by an exit. -/
theorem BRelRX.toR {R : Val → Val → Prop} {XE : Bool → List Val → Bool → List Val → Prop}
    {vf : Prop} {r₁ r₂ : Run H} (h : BRelRX p L R XE vf r₁ r₂) : BRelR p L R vf r₁ r₂ :=
  fun t₁ t₂ => (h (termX_of_term t₁) (termX_of_term t₂)).imp
    (fun h => ⟨h.1, (OutRelX.of_noExit (noExit_of_term t₁) (noExit_of_term t₂)).mp h.2.1, h.2.2⟩) id

/-- Runs that never exit relate in a loop's body as outside it. -/
theorem BRelR.toX {R : Val → Val → Prop} {XE : Bool → List Val → Bool → List Val → Prop}
    {vf : Prop} {r₁ r₂ : Run H} (hn₁ : ∀ b vs, r₁.2.1 ≠ .exit b vs)
    (hn₂ : ∀ b vs, r₂.2.1 ≠ .exit b vs) (h : BRelR p L R vf r₁ r₂) : BRelRX p L R XE vf r₁ r₂ :=
  fun t₁ t₂ => (h (term_of_termX t₁ hn₁) (term_of_termX t₂ hn₂)).imp
    (fun h => ⟨h.1, (OutRelX.of_noExit hn₁ hn₂).mpr h.2.1, h.2.2⟩) id

theorem andThen_termX {first : Run H} {rest : World H → Val → Run H}
    (h : TermX (andThen first rest).2.1) : TermX first.2.1 := by
  obtain ⟨tr, r, w⟩ := first
  cases r <;> simp_all [andThen, TermX]

/-- **Sequencing in a loop's body** (`andThen_brelR`, ADR 0116 rule 20): an exit of the first part
is the whole's, and it is never against another outcome. When the observer does not see what
decides the first part's failure, a run that went on must show nothing more, fail only unseen,
not exit, and leave a world the observer cannot tell from the one it started from. -/
theorem andThen_brelRX {R₁ R₂ : Val → Val → Prop} {XE : Bool → List Val → Bool → List Val → Prop}
    {vf vf₁ : Prop} {f₁ f₂ : Run H} {k₁ k₂ : World H → Val → Run H}
    (h₁ : BRelRX p L R₁ XE vf₁ f₁ f₂) (hvf : vf → vf₁)
    (h₂ : ∀ v₁ v₂, f₁.2.1 = .ok v₁ → f₂.2.1 = .ok v₂ → R₁ v₁ v₂ → WorldRel p L f₁.2.2 f₂.2.2 →
      BRelRX p L R₂ XE vf (k₁ f₁.2.2 v₁) (k₂ f₂.2.2 v₂))
    (hc₁ : ¬ vf₁ → ∀ v, f₁.2.1 = .ok v → proj p L (k₁ f₁.2.2 v).1 = [] ∧
      ((k₁ f₁.2.2 v).2.1.IsFail → ¬ vf) ∧ (∀ b vs, (k₁ f₁.2.2 v).2.1 ≠ .exit b vs) ∧
        WorldRel p L f₁.2.2 (k₁ f₁.2.2 v).2.2)
    (hc₂ : ¬ vf₁ → ∀ v, f₂.2.1 = .ok v → proj p L (k₂ f₂.2.2 v).1 = [] ∧
      ((k₂ f₂.2.2 v).2.1.IsFail → ¬ vf) ∧ (∀ b vs, (k₂ f₂.2.2 v).2.1 ≠ .exit b vs) ∧
        WorldRel p L f₂.2.2 (k₂ f₂.2.2 v).2.2) :
    BRelRX p L R₂ XE vf (andThen f₁ k₁) (andThen f₂ k₂) := by
  intro ht₁ ht₂
  have hf₁ := andThen_termX ht₁
  have hf₂ := andThen_termX ht₂
  rcases h₁ hf₁ hf₂ with ⟨hp, ho, hw⟩ | hd
  · obtain ⟨tr₁, r₁, w₁⟩ := f₁
    obtain ⟨tr₂, r₂, w₂⟩ := f₂
    simp only at hf₁ hf₂ ho h₂ hc₁ hc₂ hp hw
    by_cases ok₁ : ∃ v, r₁ = .ok v <;> by_cases ok₂ : ∃ v, r₂ = .ok v
    · obtain ⟨v₁, rfl⟩ := ok₁
      obtain ⟨v₂, rfl⟩ := ok₂
      have hk := h₂ v₁ v₂ rfl rfl ho hw
      simp only [andThen] at ht₁ ht₂ ⊢
      revert ht₁ ht₂ hk
      rcases k₁ w₁ v₁ with ⟨tk₁, rk₁, wk₁⟩
      rcases k₂ w₂ v₂ with ⟨tk₂, rk₂, wk₂⟩
      intro ht₁ ht₂ hk
      rcases hk ht₁ ht₂ with ⟨hp', ho', hw'⟩ | hd'
      · exact .inl ⟨by simp only [proj_append]; rw [hp, hp'], ho', hw'⟩
      · exact .inr (Diverged.prepend hp hd')
    · obtain ⟨v₁, rfl⟩ := ok₁
      have hn₂ : ∀ v, r₂ ≠ .ok v := fun v h => ok₂ ⟨v, h⟩
      have hnv : ¬ vf₁ := by cases r₂ <;> simp_all [OutRelX, OutRel, TermX]
      obtain ⟨hpk, hfk, hxk, hwk⟩ := hc₁ hnv v₁ rfl
      rw [andThen_not_ok hn₂]
      simp only [andThen] at ht₁ ⊢
      revert ht₁ hpk hfk hxk hwk
      rcases k₁ w₁ v₁ with ⟨tk, rk, wk⟩
      intro ht₁ hpk hfk hxk hwk
      refine .inl ⟨by simp only [proj_append]; rw [hpk, List.append_nil]; exact hp, ?_,
        hwk.symm.trans hw⟩
      simp only at ht₁ hfk hxk ⊢
      have hnvf : ¬ vf := fun hv => hnv (hvf hv)
      cases rk <;> cases r₂ <;> simp_all [OutRelX, OutRel, TermX, Res.IsFail]
    · obtain ⟨v₂, rfl⟩ := ok₂
      have hn₁ : ∀ v, r₁ ≠ .ok v := fun v h => ok₁ ⟨v, h⟩
      have hnv : ¬ vf₁ := by cases r₁ <;> simp_all [OutRelX, OutRel, TermX]
      obtain ⟨hpk, hfk, hxk, hwk⟩ := hc₂ hnv v₂ rfl
      rw [andThen_not_ok hn₁]
      simp only [andThen] at ht₂ ⊢
      revert ht₂ hpk hfk hxk hwk
      rcases k₂ w₂ v₂ with ⟨tk, rk, wk⟩
      intro ht₂ hpk hfk hxk hwk
      refine .inl ⟨by simp only [proj_append]; rw [hpk, List.append_nil]; exact hp, ?_, hw.trans hwk⟩
      simp only at ht₂ hfk hxk ⊢
      have hnvf : ¬ vf := fun hv => hnv (hvf hv)
      cases rk <;> cases r₁ <;> simp_all [OutRelX, OutRel, TermX, Res.IsFail]
    · have hn₁ : ∀ v, r₁ ≠ .ok v := fun v h => ok₁ ⟨v, h⟩
      have hn₂ : ∀ v, r₂ ≠ .ok v := fun v h => ok₂ ⟨v, h⟩
      rw [andThen_not_ok hn₁, andThen_not_ok hn₂]
      refine .inl ⟨hp, ?_, hw⟩
      cases r₁ <;> cases r₂ <;>
        first
        | exact absurd rfl (hn₁ _)
        | exact absurd rfl (hn₂ _)
        | exact fun hv => ho (hvf hv)
        | exact ho
        | simp_all [OutRelX, OutRel, TermX]
  · obtain ⟨u₁, hu₁⟩ := andThen_fst f₁ k₁
    obtain ⟨u₂, hu₂⟩ := andThen_fst f₂ k₂
    rw [hu₁, hu₂]
    exact .inr (hd.append u₁ u₂)

/-- Catching `skip` keeps two iterations related: two exits with the same tag and related states,
a `skip` of each, give related states (ADR 0116 rule 20). -/
theorem iter_brelRX {ts : List LTy} {vf : Prop} {r₁ r₂ : Run H}
    (h : BRelRX p L (StRel (p := p) (L := L) ts) (ExitRel p L ts) vf r₁ r₂) :
    BRelRX p L (StRel (p := p) (L := L) ts) (ExitRel p L ts) vf (Run.iter r₁) (Run.iter r₂) := by
  obtain ⟨tr₁, o₁, w₁⟩ := r₁
  obtain ⟨tr₂, o₂, w₂⟩ := r₂
  have hT : ∀ {o : Res} {tr : Trace} {w : World H}, TermX (Run.iter (tr, o, w)).2.1 → TermX o := by
    intro o tr w ht
    cases o with
    | exit b _ => cases b <;> trivial
    | _ => exact ht
  intro t₁ t₂
  rcases h (hT t₁) (hT t₂) with ⟨hp, ho, hw⟩ | hd
  · refine .inl ⟨?_, ?_, ?_⟩
    · cases o₁ with
      | exit b _ => cases b <;> cases o₂ <;> first | exact hp | (rename_i b' _; cases b' <;> exact hp)
      | _ => cases o₂ with
        | exit b _ => cases b <;> exact hp
        | _ => exact hp
    · cases o₁ with
      | exit b₁ vs₁ =>
        cases o₂ with
        | exit b₂ vs₂ =>
          obtain ⟨rfl, hvs⟩ := ho
          cases b₁
          · exact ⟨_, _, rfl, rfl, hvs⟩
          · exact ⟨rfl, hvs⟩
        | _ => first | (have h2 := hT t₂; simp [TermX] at h2; done) | simp [OutRelX] at ho
      | _ =>
        cases o₂ with
        | exit b₂ vs₂ => cases b₂ <;> simp_all [OutRelX, Run.iter, TermX]
        | _ => exact ho
    · cases o₁ with
      | exit b _ => cases b <;> cases o₂ <;> first | exact hw | (rename_i b' _; cases b' <;> exact hw)
      | _ => cases o₂ with
        | exit b _ => cases b <;> exact hw
        | _ => exact hw
  · refine .inr ?_
    cases o₁ with
    | exit b _ => cases b <;> cases o₂ <;> first | exact hd | (rename_i b' _; cases b' <;> exact hd)
    | _ => cases o₂ with
      | exit b _ => cases b <;> exact hd
      | _ => exact hd

/-- The end of a loop after a first iteration that gave a value. -/
theorem endLoop_andThen_ok (n : Nat) (t : Trace) (v : Val) (w : World H)
    (k : World H → Val → Run H) :
    Run.endLoop n (andThen (t, .ok v, w) k) =
      (t ++ (Run.endLoop n (k w v)).1, (Run.endLoop n (k w v)).2.1, (Run.endLoop n (k w v)).2.2) := by
  simp only [andThen]
  rcases k w v with ⟨t', r', w'⟩
  cases r' with
  | exit b st => cases b <;> simp only [Run.endLoop] <;> split <;> rfl
  | _ => rfl

/-- The end of a loop after a first iteration that gave no value: the loop's end of it. -/
theorem endLoop_andThen_not_ok (n : Nat) (t : Trace) (r : Res) (w : World H)
    (k : World H → Val → Run H) (h : ∀ v, r ≠ .ok v) :
    Run.endLoop n (andThen (t, r, w) k) = Run.endLoop n (t, r, w) := by
  rw [andThen_not_ok h]

/-- **One iteration of a loop, then the rest** (ADR 0116 rule 20): when two iterations are related,
their states typed, and the rest of the loops from related states are related, the loops are. An
iteration's `stop` ends its loop with its state; when the observer does not see what decides an
iteration's failure, what the other run does after an iteration it completed shows the observer
nothing (`hc₁`, `hc₂`). -/
theorem iterThen_brelR {ts : List LTy} {vf : Prop} {n : Nat} {i₁ i₂ : Run H}
    {K₁ K₂ : World H → Val → Run H} (hn : ts.length = n)
    (hi : BRelRX p L (StRel (p := p) (L := L) ts) (ExitRel p L ts) vf i₁ i₂)
    (hk : ∀ st₁ st₂, i₁.2.1 = .ok (.tuple st₁) → i₂.2.1 = .ok (.tuple st₂) →
      VList p L ts st₁ st₂ → WorldRel p L i₁.2.2 i₂.2.2 →
      BRelR p L (StRel (p := p) (L := L) ts) vf (Run.endLoop n (K₁ i₁.2.2 (.tuple st₁)))
        (Run.endLoop n (K₂ i₂.2.2 (.tuple st₂))))
    (hc₁ : ¬ vf → ∀ v, i₁.2.1 = .ok v → proj p L (Run.endLoop n (K₁ i₁.2.2 v)).1 = [] ∧
      ((Run.endLoop n (K₁ i₁.2.2 v)).2.1.IsFail → ¬ vf) ∧
        WorldRel p L i₁.2.2 (Run.endLoop n (K₁ i₁.2.2 v)).2.2)
    (hc₂ : ¬ vf → ∀ v, i₂.2.1 = .ok v → proj p L (Run.endLoop n (K₂ i₂.2.2 v)).1 = [] ∧
      ((Run.endLoop n (K₂ i₂.2.2 v)).2.1.IsFail → ¬ vf) ∧
        WorldRel p L i₂.2.2 (Run.endLoop n (K₂ i₂.2.2 v)).2.2)
    (hx₁ : ∀ b vs, i₁.2.1 = .exit b vs → b = true ∧ vs.length = n)
    (hx₂ : ∀ b vs, i₂.2.1 = .exit b vs → b = true ∧ vs.length = n) :
    BRelR p L (StRel (p := p) (L := L) ts) vf (Run.endLoop n (andThen i₁ K₁))
      (Run.endLoop n (andThen i₂ K₂)) := by
  obtain ⟨tr₁, r₁, w₁⟩ := i₁
  obtain ⟨tr₂, r₂, w₂⟩ := i₂
  simp only at hk hc₁ hc₂ hx₁ hx₂
  -- An end of the whole is an end of the iteration.
  have hT : ∀ {tr : Trace} {r : Res} {w : World H} {K : World H → Val → Run H},
      Term (Run.endLoop n (andThen (tr, r, w) K)).2.1 → TermX r := by
    intro tr r w K ht
    cases r with
    | ok _ => trivial
    | failed _ | raised _ _ _ | exit _ _ => trivial
    | _ => simp [andThen, Run.endLoop, Term] at ht
  intro t₁ t₂
  rcases hi (hT t₁) (hT t₂) with ⟨hp, ho, hw⟩ | hd
  · simp only at hp ho hw
    cases r₁ with
    | ok v₁ =>
      cases r₂ with
      | ok v₂ =>
        obtain ⟨st₁, st₂, rfl, rfl, hst⟩ := ho
        have hk' := hk st₁ st₂ rfl rfl hst hw
        simp only [endLoop_andThen_ok] at t₁ t₂ ⊢
        rcases hk' t₁ t₂ with ⟨hp', ho', hw'⟩ | hd'
        · exact .inl ⟨by simp only [proj_append]; rw [hp, hp'], ho', hw'⟩
        · exact .inr (Diverged.prepend hp hd')
      | exit _ _ => simp [OutRelX] at ho
      | stuck | outOfFuel | aborted =>
        simp [andThen, Run.endLoop, Term] at t₂
      | failed _ | raised _ _ _ =>
        have hnv : ¬ vf := by simpa [OutRelX, OutRel] using ho
        obtain ⟨hpk, hfk, hwk⟩ := hc₁ hnv v₁ rfl
        rw [endLoop_andThen_ok] at t₁ ⊢
        rw [endLoop_andThen_not_ok _ _ _ _ _ (fun v h => by cases h)] at t₂ ⊢
        refine .inl ⟨by simp only [proj_append]; rw [hpk, List.append_nil]; exact hp, ?_,
          hwk.symm.trans hw⟩
        revert t₁ hfk
        generalize (Run.endLoop n (K₁ w₁ v₁)).2.1 = rk
        intro t₁ hfk
        simp only [Run.endLoop] at t₁ t₂ ⊢
        cases rk <;> simp_all [OutRel, Term, Res.IsFail]
    | exit b₁ vs₁ =>
      cases r₂ with
      | exit b₂ vs₂ =>
        obtain ⟨rfl, hvs⟩ := ho
        obtain ⟨rfl, hl₁⟩ := hx₁ b₁ vs₁ rfl
        obtain ⟨_, hl₂⟩ := hx₂ true vs₂ rfl
        rw [endLoop_andThen_not_ok _ _ _ _ _ (fun v h => by cases h),
          endLoop_andThen_not_ok _ _ _ _ _ (fun v h => by cases h)]
        simp only [Run.endLoop, if_pos hl₁, if_pos hl₂]
        exact .inl ⟨hp, ⟨_, _, rfl, rfl, hvs⟩, hw⟩
      | stuck | outOfFuel | aborted => simp [andThen, Run.endLoop, Term] at t₂
      | _ => simp [OutRelX] at ho
    | stuck | outOfFuel | aborted => simp [andThen, Run.endLoop, Term] at t₁
    | failed _ | raised _ _ _ =>
      cases r₂ with
      | ok v₂ =>
        have hnv : ¬ vf := by simpa [OutRelX, OutRel] using ho
        obtain ⟨hpk, hfk, hwk⟩ := hc₂ hnv v₂ rfl
        rw [endLoop_andThen_ok] at t₂ ⊢
        rw [endLoop_andThen_not_ok _ _ _ _ _ (fun v h => by cases h)] at t₁ ⊢
        refine .inl ⟨by simp only [proj_append]; rw [hpk, List.append_nil]; exact hp, ?_, hw.trans hwk⟩
        revert t₂ hfk
        generalize (Run.endLoop n (K₂ w₂ v₂)).2.1 = rk
        intro t₂ hfk
        simp only [Run.endLoop] at t₁ t₂ ⊢
        cases rk <;> simp_all [OutRel, Term, Res.IsFail]
      | exit _ _ => simp [OutRelX] at ho
      | stuck | outOfFuel | aborted => simp [andThen, Run.endLoop, Term] at t₂
      | failed _ | raised _ _ _ =>
        rw [endLoop_andThen_not_ok _ _ _ _ _ (fun v h => by cases h),
          endLoop_andThen_not_ok _ _ _ _ _ (fun v h => by cases h)]
        exact .inl ⟨hp, by simpa [Run.endLoop, OutRelX] using ho, hw⟩
  · obtain ⟨u₁, hu₁⟩ := andThen_fst (tr₁, r₁, w₁) K₁
    obtain ⟨u₂, hu₂⟩ := andThen_fst (tr₂, r₂, w₂) K₂
    have he : ∀ (r : Run H), (Run.endLoop n r).1 = r.1 := by
      intro r; obtain ⟨_, o, _⟩ := r; cases o with
      | exit b _ => cases b <;> simp only [Run.endLoop] <;> split <;> rfl
      | _ => rfl
    rw [he, he, hu₁, hu₂]
    exact .inr (hd.append u₁ u₂)

/-- A typed iteration's run, once its `skip` is caught: a typed state when it gives a value, and a
`stop` with a typed state when it exits. -/
theorem iter_facts {A : Event → Prop} {F : FailCond} {ts : List LTy} {w : World H} {r : Run H}
    (h : UOk A (TupTy p ts) F w r) (hex : ∀ b vs, F.exit b vs → LValsTy p vs ts) :
    (∀ v, (Run.iter r).2.1 = .ok v → ∃ st, v = .tuple st ∧ LValsTy p st ts) ∧
      (∀ b vs, (Run.iter r).2.1 = .exit b vs → b = true ∧ vs.length = ts.length) := by
  have hi := (iter_uok (p := p) h hex).2.1
  refine ⟨fun v hv => ?_, fun b vs hv => ?_⟩
  · rw [hv] at hi; exact hi
  · rw [hv] at hi
    exact ⟨hi.1, LValsTy.length' hi.2⟩

/-- **Two loops over related elements, from related states, are related up to their first
different release, once their `stop` ends them** (M6, ADR 0116 rule 20): `walkLoop_brelR` for
iterations that may exit. Two iterations exit together, with the same tag and related states
(`ExitRel`), as T-Exit's public program counter makes them; a `skip` goes on with its state, a
`stop` ends both loops with theirs. -/
theorem walkLoop_brelRX {body₁ body₂ : World H → Val → List Val → Run H} {a : LTy} {ts : List LTy}
    {A₁ A₂ : Event → Prop} {F₁ F₂ : FailCond} {vf : Prop}
    (hbody : ∀ w₁ w₂ v₁ v₂ st₁ st₂, WorldRel p L w₁ w₂ → V p L a v₁ v₂ → LValTy p v₁ a →
      LValTy p v₂ a → VList p L ts st₁ st₂ → LValsTy p st₁ ts → LValsTy p st₂ ts →
      BRelRX p L (StRel (p := p) (L := L) ts) (ExitRel p L ts) vf (body₁ w₁ v₁ st₁) (body₂ w₂ v₂ st₂))
    (hu₁ : ∀ w v st, LValTy p v a → LValsTy p st ts → UOk A₁ (TupTy p ts) F₁ w (body₁ w v st))
    (hu₂ : ∀ w v st, LValTy p v a → LValsTy p st ts → UOk A₂ (TupTy p ts) F₂ w (body₂ w v st))
    (hex₁ : ∀ b vs, F₁.exit b vs → LValsTy p vs ts) (hex₂ : ∀ b vs, F₂.exit b vs → LValsTy p vs ts)
    (hr₁ : ¬ vf → ∀ w st xs, LValsTy p st ts → (∀ x ∈ xs, LValTy p x a) →
      proj p L (Run.endLoop ts.length (walkLoop body₁ w st xs)).1 = [] ∧
        ((Run.endLoop ts.length (walkLoop body₁ w st xs)).2.1.IsFail → ¬ vf) ∧
        WorldRel p L w (Run.endLoop ts.length (walkLoop body₁ w st xs)).2.2)
    (hr₂ : ¬ vf → ∀ w st xs, LValsTy p st ts → (∀ x ∈ xs, LValTy p x a) →
      proj p L (Run.endLoop ts.length (walkLoop body₂ w st xs)).1 = [] ∧
        ((Run.endLoop ts.length (walkLoop body₂ w st xs)).2.1.IsFail → ¬ vf) ∧
        WorldRel p L w (Run.endLoop ts.length (walkLoop body₂ w st xs)).2.2) :
    ∀ {xs ys : List Val} {st₁ st₂ : List Val} {w₁ w₂ : World H}, WorldRel p L w₁ w₂ →
      VList p L ts st₁ st₂ → LValsTy p st₁ ts → LValsTy p st₂ ts → VAll p L a xs ys →
      LValsAll p xs a → LValsAll p ys a →
      BRelR p L (StRel (p := p) (L := L) ts) vf (Run.endLoop ts.length (walkLoop body₁ w₁ st₁ xs))
        (Run.endLoop ts.length (walkLoop body₂ w₂ st₂ ys))
  | [], [], _, _, _, _, hw, hst, _, _, .nil, _, _ => .of_brel (brel_done hw ⟨_, _, rfl, rfl, hst⟩)
  | _ :: xs, _ :: ys, st₁, st₂, w₁, w₂, hw, hst, hs₁, hs₂, .cons h hs, .cons hx hxs, .cons hy hys => by
    simp only [walkLoop]
    have f₁ := iter_facts (hu₁ w₁ _ _ hx hs₁) hex₁
    have f₂ := iter_facts (hu₂ w₂ _ _ hy hs₂) hex₂
    refine iterThen_brelR rfl (iter_brelRX (hbody _ _ _ _ _ _ hw h hx hy hst hs₁ hs₂))
      (fun st₁' st₂' h₁ h₂ hss hw' => ?_) (fun hn v hv => ?_) (fun hn v hv => ?_) f₁.2 f₂.2
    · obtain ⟨_, he₁, ht₁⟩ := f₁.1 _ h₁
      obtain ⟨_, he₂, ht₂⟩ := f₂.1 _ h₂
      cases he₁
      cases he₂
      simp only
      rw [ite_eq_left (by rw [LValsTy.length' ht₁, LValsTy.length' hs₁]),
        ite_eq_left (by rw [LValsTy.length' ht₂, LValsTy.length' hs₂])]
      exact walkLoop_brelRX hbody hu₁ hu₂ hex₁ hex₂ hr₁ hr₂ hw' hss ht₁ ht₂ hs hxs hys
    · obtain ⟨s, rfl, hts⟩ := f₁.1 v hv
      simp only
      rw [ite_eq_left (by rw [LValsTy.length' hts, LValsTy.length' hs₁])]
      exact hr₁ hn _ _ _ hts (LValsAll.mem hxs)
    · obtain ⟨s, rfl, hts⟩ := f₂.1 v hv
      simp only
      rw [ite_eq_left (by rw [LValsTy.length' hts, LValsTy.length' hs₂])]
      exact hr₂ hn _ _ _ hts (LValsAll.mem hys)

/-- The end of a loop after a first part that does not exit is the first part, then the end. -/
theorem endLoop_andThen_noExit (n : Nat) {f : Run H} (k : World H → Val → Run H)
    (h : ∀ b vs, f.2.1 ≠ .exit b vs) :
    Run.endLoop n (andThen f k) = andThen f (fun w v => Run.endLoop n (k w v)) := by
  obtain ⟨t, r, w⟩ := f
  cases r with
  | ok v =>
    rw [endLoop_andThen_ok]
    simp only [andThen]
  | exit b vs => exact absurd rfl (h b vs)
  | _ => rfl

/-- **Two `for until` loops from related states, whose conditions the observer sees, are related
up to their first different release, once their `stop` ends them** (ADR 0116 rule 7): the
conditions do not fail, and give the same Boolean in both runs (`hcond`), and the iterations are
related as in `walkLoop_brelRX`. -/
theorem walkUntil_brelRX {cond₁ cond₂ body₁ body₂ : World H → List Val → Run H} {ts : List LTy}
    {A₁ A₂ : Event → Prop} {F₁ F₂ : FailCond} {vf : Prop}
    (hcond : ∀ w₁ w₂ st₁ st₂, WorldRel p L w₁ w₂ → VList p L ts st₁ st₂ → LValsTy p st₁ ts →
      LValsTy p st₂ ts →
      BRelR p L (fun a b => ∃ c : Bool, a = .bool c ∧ b = .bool c) True (cond₁ w₁ st₁) (cond₂ w₂ st₂))
    (hcu₁ : ∀ w st, LValsTy p st ts → ∀ b vs, (cond₁ w st).2.1 ≠ .exit b vs)
    (hcu₂ : ∀ w st, LValsTy p st ts → ∀ b vs, (cond₂ w st).2.1 ≠ .exit b vs)
    (hbody : ∀ w₁ w₂ st₁ st₂, WorldRel p L w₁ w₂ → VList p L ts st₁ st₂ → LValsTy p st₁ ts →
      LValsTy p st₂ ts →
      BRelRX p L (StRel (p := p) (L := L) ts) (ExitRel p L ts) vf (body₁ w₁ st₁) (body₂ w₂ st₂))
    (hu₁ : ∀ w st, LValsTy p st ts → UOk A₁ (TupTy p ts) F₁ w (body₁ w st))
    (hu₂ : ∀ w st, LValsTy p st ts → UOk A₂ (TupTy p ts) F₂ w (body₂ w st))
    (hex₁ : ∀ b vs, F₁.exit b vs → LValsTy p vs ts) (hex₂ : ∀ b vs, F₂.exit b vs → LValsTy p vs ts)
    (hb₁ : ¬ vf → ∀ k w st, LValsTy p st ts →
      proj p L (Run.endLoop ts.length (walkUntil cond₁ body₁ k w st)).1 = [] ∧
        ((Run.endLoop ts.length (walkUntil cond₁ body₁ k w st)).2.1.IsFail → ¬ vf) ∧
        WorldRel p L w (Run.endLoop ts.length (walkUntil cond₁ body₁ k w st)).2.2)
    (hb₂ : ¬ vf → ∀ k w st, LValsTy p st ts →
      proj p L (Run.endLoop ts.length (walkUntil cond₂ body₂ k w st)).1 = [] ∧
        ((Run.endLoop ts.length (walkUntil cond₂ body₂ k w st)).2.1.IsFail → ¬ vf) ∧
        WorldRel p L w (Run.endLoop ts.length (walkUntil cond₂ body₂ k w st)).2.2) :
    ∀ (k : Nat) {st₁ st₂ : List Val} {w₁ w₂ : World H}, WorldRel p L w₁ w₂ →
      VList p L ts st₁ st₂ → LValsTy p st₁ ts → LValsTy p st₂ ts →
      BRelR p L (StRel (p := p) (L := L) ts) vf (Run.endLoop ts.length (walkUntil cond₁ body₁ k w₁ st₁))
        (Run.endLoop ts.length (walkUntil cond₂ body₂ k w₂ st₂))
  | 0, _, _, _, _, _, _, _, _ => fun h => by simp [walkUntil, Run.endLoop, Term] at h
  | k + 1, st₁, st₂, w₁, w₂, hw, hst, hs₁, hs₂ => by
    simp only [walkUntil]
    rw [endLoop_andThen_noExit _ _ (hcu₁ w₁ st₁ hs₁), endLoop_andThen_noExit _ _ (hcu₂ w₂ st₂ hs₂)]
    refine BRelR.mono (fun _ _ h => h) id (andThen_brelR (hcond w₁ w₂ st₁ st₂ hw hst hs₁ hs₂)
      (fun _ => trivial) (fun c₁ c₂ h₁ h₂ hc hw' => ?_) (fun hn => absurd trivial hn)
      (fun hn => absurd trivial hn))
    obtain ⟨c, rfl, rfl⟩ := hc
    cases c
    · simp only
      have f₁ := iter_facts (hu₁ (cond₁ w₁ st₁).2.2 _ hs₁) hex₁
      have f₂ := iter_facts (hu₂ (cond₂ w₂ st₂).2.2 _ hs₂) hex₂
      refine iterThen_brelR rfl (iter_brelRX (hbody _ _ _ _ hw' hst hs₁ hs₂))
        (fun st₁' st₂' e₁ e₂ hss hw'' => ?_) (fun hn v hv => ?_) (fun hn v hv => ?_) f₁.2 f₂.2
      · obtain ⟨_, he₁, ht₁⟩ := f₁.1 _ e₁
        obtain ⟨_, he₂, ht₂⟩ := f₂.1 _ e₂
        cases he₁
        cases he₂
        simp only
        rw [ite_eq_left (by rw [LValsTy.length' ht₁, LValsTy.length' hs₁]),
          ite_eq_left (by rw [LValsTy.length' ht₂, LValsTy.length' hs₂])]
        exact walkUntil_brelRX hcond hcu₁ hcu₂ hbody hu₁ hu₂ hex₁ hex₂ hb₁ hb₂ k hw'' hss ht₁ ht₂
      · obtain ⟨s, rfl, hts⟩ := f₁.1 v hv
        simp only
        rw [ite_eq_left (by rw [LValsTy.length' hts, LValsTy.length' hs₁])]
        exact hb₁ hn _ _ _ hts
      · obtain ⟨s, rfl, hts⟩ := f₂.1 v hv
        simp only
        rw [ite_eq_left (by rw [LValsTy.length' hts, LValsTy.length' hs₂])]
        exact hb₂ hn _ _ _ hts
    · exact .of_brel (brel_done hw' ⟨_, _, rfl, rfl, hst⟩)

/-- **Two `for ever` loops from related states, under one fixed stop schedule, are related up to
their first different release, once their `stop` ends them** (ADR 0116 rules 10, 13 and 14): the
host stops both at the same iteration start, since its schedule is the same input to both runs,
and the iterations are related as in `walkLoop_brelRX`. -/
theorem walkEver_brelRX {stops : Nat → Bool} {body₁ body₂ : World H → List Val → Run H}
    {ts : List LTy} {A₁ A₂ : Event → Prop} {F₁ F₂ : FailCond} {vf : Prop}
    (hbody : ∀ w₁ w₂ st₁ st₂, WorldRel p L w₁ w₂ → VList p L ts st₁ st₂ → LValsTy p st₁ ts →
      LValsTy p st₂ ts →
      BRelRX p L (StRel (p := p) (L := L) ts) (ExitRel p L ts) vf (body₁ w₁ st₁) (body₂ w₂ st₂))
    (hu₁ : ∀ w st, LValsTy p st ts → UOk A₁ (TupTy p ts) F₁ w (body₁ w st))
    (hu₂ : ∀ w st, LValsTy p st ts → UOk A₂ (TupTy p ts) F₂ w (body₂ w st))
    (hex₁ : ∀ b vs, F₁.exit b vs → LValsTy p vs ts) (hex₂ : ∀ b vs, F₂.exit b vs → LValsTy p vs ts)
    (hb₁ : ¬ vf → ∀ k i w st, LValsTy p st ts →
      proj p L (Run.endLoop ts.length (walkEver stops body₁ k i w st)).1 = [] ∧
        ((Run.endLoop ts.length (walkEver stops body₁ k i w st)).2.1.IsFail → ¬ vf) ∧
        WorldRel p L w (Run.endLoop ts.length (walkEver stops body₁ k i w st)).2.2)
    (hb₂ : ¬ vf → ∀ k i w st, LValsTy p st ts →
      proj p L (Run.endLoop ts.length (walkEver stops body₂ k i w st)).1 = [] ∧
        ((Run.endLoop ts.length (walkEver stops body₂ k i w st)).2.1.IsFail → ¬ vf) ∧
        WorldRel p L w (Run.endLoop ts.length (walkEver stops body₂ k i w st)).2.2) :
    ∀ (k i : Nat) {st₁ st₂ : List Val} {w₁ w₂ : World H}, WorldRel p L w₁ w₂ →
      VList p L ts st₁ st₂ → LValsTy p st₁ ts → LValsTy p st₂ ts →
      BRelR p L (StRel (p := p) (L := L) ts) vf
        (Run.endLoop ts.length (walkEver stops body₁ k i w₁ st₁))
        (Run.endLoop ts.length (walkEver stops body₂ k i w₂ st₂))
  | 0, _, _, _, _, _, _, _, _, _ => fun h => by simp [walkEver, Run.endLoop, Term] at h
  | k + 1, i, st₁, st₂, w₁, w₂, hw, hst, hs₁, hs₂ => by
    simp only [walkEver]
    by_cases hs : stops i = true
    · rw [if_pos hs, if_pos hs]
      exact .of_brel (brel_done hw ⟨_, _, rfl, rfl, hst⟩)
    · rw [if_neg hs, if_neg hs]
      have f₁ := iter_facts (hu₁ w₁ _ hs₁) hex₁
      have f₂ := iter_facts (hu₂ w₂ _ hs₂) hex₂
      refine iterThen_brelR rfl (iter_brelRX (hbody _ _ _ _ hw hst hs₁ hs₂))
        (fun st₁' st₂' e₁ e₂ hss hw' => ?_) (fun hn v hv => ?_) (fun hn v hv => ?_) f₁.2 f₂.2
      · obtain ⟨_, he₁, ht₁⟩ := f₁.1 _ e₁
        obtain ⟨_, he₂, ht₂⟩ := f₂.1 _ e₂
        cases he₁
        cases he₂
        simp only
        rw [ite_eq_left (by rw [LValsTy.length' ht₁, LValsTy.length' hs₁]),
          ite_eq_left (by rw [LValsTy.length' ht₂, LValsTy.length' hs₂])]
        exact walkEver_brelRX hbody hu₁ hu₂ hex₁ hex₂ hb₁ hb₂ k (i + 1) hw' hss ht₁ ht₂
      · obtain ⟨s, rfl, hts⟩ := f₁.1 v hv
        simp only
        rw [ite_eq_left (by rw [LValsTy.length' hts, LValsTy.length' hs₁])]
        exact hb₁ hn _ _ _ _ hts
      · obtain ⟨s, rfl, hts⟩ := f₂.1 v hv
        simp only
        rw [ite_eq_left (by rw [LValsTy.length' hts, LValsTy.length' hs₂])]
        exact hb₂ hn _ _ _ _ hts

/-- A run of `on stop` after a failure ends when the whole ends. -/
theorem termk_of {k : World H → List Val → Run H} {st₀ : List Val} {tr : Trace} {o : Res}
    {w : World H} (h : Term (finishEver k st₀ (tr, o, w)).2.1) (hf : o.IsFail) :
    Term (k w st₀).2.1 := by
  cases o with
  | failed c =>
    simp only [finishEver] at h
    revert h
    rcases k w st₀ with ⟨t', r', w'⟩
    cases r' <;> simp [Term]
  | raised kd c ps =>
    simp only [finishEver] at h
    revert h
    rcases k w st₀ with ⟨t', r', w'⟩
    cases r' <;> simp [Term]
  | _ => simp [Res.IsFail] at hf

/-- `on stop` adds to a run's trace. -/
theorem finishEver_fst (k : World H → List Val → Run H) (st₀ : List Val) (r : Run H) :
    ∃ u, (finishEver k st₀ r).1 = r.1 ++ u := by
  obtain ⟨tr, o, w⟩ := r
  cases o with
  | ok v =>
    cases v with
    | tuple st =>
      simp only [finishEver]
      rcases k w st with ⟨t', r', w'⟩
      exact ⟨t', rfl⟩
    | _ => exact ⟨[], by simp [finishEver]⟩
  | failed c =>
    simp only [finishEver]
    rcases k w st₀ with ⟨t', r', w'⟩
    cases r' <;> exact ⟨t', rfl⟩
  | raised kd c ps =>
    simp only [finishEver]
    rcases k w st₀ with ⟨t', r', w'⟩
    cases r' <;> exact ⟨t', rfl⟩
  | _ => exact ⟨[], by simp [finishEver]⟩

/-- **`on stop` after two related `for ever` loops** (ADR 0116 rule 10): the block runs on the
loops' related final states, or, after a failure of both, on their related initial states, and
gives related values; it does not fail (U3029). When the observer does not see what decides the
loops' failures, the blocks show it nothing (`hk₁`, `hk₂`). -/
theorem finishEver_brelR {k₁ k₂ : World H → List Val → Run H} {ts : List LTy}
    {R : Val → Val → Prop} {vf vf' : Prop} {st₀₁ st₀₂ : List Val} {r₁ r₂ : Run H}
    (hvf : vf' → vf) (hr : BRelR p L (StRel (p := p) (L := L) ts) vf r₁ r₂)
    (ht₁ : ∀ v, r₁.2.1 = .ok v → ∃ st, v = .tuple st ∧ LValsTy p st ts)
    (ht₂ : ∀ v, r₂.2.1 = .ok v → ∃ st, v = .tuple st ∧ LValsTy p st ts)
    (hs₀ : VList p L ts st₀₁ st₀₂) (hs₀₁ : LValsTy p st₀₁ ts) (hs₀₂ : LValsTy p st₀₂ ts)
    (hk : ∀ w₁ w₂ st₁ st₂, WorldRel p L w₁ w₂ → VList p L ts st₁ st₂ → LValsTy p st₁ ts →
      LValsTy p st₂ ts → BRelR p L R vf' (k₁ w₁ st₁) (k₂ w₂ st₂))
    (hkf₁ : ∀ w st, LValsTy p st ts → ¬ (k₁ w st).2.1.IsFail ∧ ∀ b vs, (k₁ w st).2.1 ≠ .exit b vs)
    (hkf₂ : ∀ w st, LValsTy p st ts → ¬ (k₂ w st).2.1.IsFail ∧ ∀ b vs, (k₂ w st).2.1 ≠ .exit b vs)
    (hc₁ : ¬ vf → ∀ w st, LValsTy p st ts →
      proj p L (k₁ w st).1 = [] ∧ WorldRel p L w (k₁ w st).2.2)
    (hc₂ : ¬ vf → ∀ w st, LValsTy p st ts →
      proj p L (k₂ w st).1 = [] ∧ WorldRel p L w (k₂ w st).2.2) :
    BRelR p L R vf' (finishEver k₁ st₀₁ r₁) (finishEver k₂ st₀₂ r₂) := by
  obtain ⟨tr₁, o₁, w₁⟩ := r₁
  obtain ⟨tr₂, o₂, w₂⟩ := r₂
  simp only at ht₁ ht₂
  -- The end of the whole is the end of the loop.
  have hT : ∀ {tr : Trace} {o : Res} {w : World H} {k : World H → List Val → Run H} {st₀ : List Val},
      Term (finishEver k st₀ (tr, o, w)).2.1 → Term o := by
    intro tr o w k st₀ h
    cases o with
    | ok v => trivial
    | failed _ | raised _ _ _ => trivial
    | exit _ _ => simp [finishEver, Term] at h
    | _ => simp [finishEver, Term] at h
  -- Running the block after a run: the block's trace, outcome and world.
  have hok : ∀ {tr : Trace} {st : List Val} {w : World H} {k : World H → List Val → Run H} {st₀ : List Val},
      finishEver k st₀ (tr, .ok (.tuple st), w) = (tr ++ (k w st).1, (k w st).2.1, (k w st).2.2) := by
    intro tr st w k st₀; simp only [finishEver]
  have hfl : ∀ {tr : Trace} {o : Res} {w : World H} {k : World H → List Val → Run H} {st₀ : List Val},
      o.IsFail → (∀ b vs, (k w st₀).2.1 ≠ .exit b vs) → ¬ (k w st₀).2.1.IsFail →
      Term (finishEver k st₀ (tr, o, w)).2.1 →
      finishEver k st₀ (tr, o, w) = (tr ++ (k w st₀).1, o, (k w st₀).2.2) := by
    intro tr o w k st₀ hf hx hnf ht
    cases o with
    | failed c =>
      simp only [finishEver] at ht ⊢
      revert ht hx hnf
      rcases k w st₀ with ⟨t', r', w'⟩
      intro hx hnf ht
      cases r' <;> simp_all [Term, Res.IsFail]
    | raised kd c ps =>
      simp only [finishEver] at ht ⊢
      revert ht hx hnf
      rcases k w st₀ with ⟨t', r', w'⟩
      intro hx hnf ht
      cases r' <;> simp_all [Term, Res.IsFail]
    | _ => simp [Res.IsFail] at hf
  intro t₁ t₂
  rcases hr (hT t₁) (hT t₂) with ⟨hp, ho, hw⟩ | hd
  · simp only at hp ho hw
    cases o₁ with
    | ok v₁ =>
      obtain ⟨st₁, rfl, hs₁⟩ := ht₁ _ rfl
      cases o₂ with
      | ok v₂ =>
        obtain ⟨st₂, rfl, hs₂⟩ := ht₂ _ rfl
        obtain ⟨_, _, h₁, h₂, hss⟩ := ho
        cases h₁; cases h₂
        rw [hok] at t₁ ⊢
        rw [hok] at t₂ ⊢
        rcases hk w₁ w₂ _ _ hw hss hs₁ hs₂ t₁ t₂ with ⟨hp', ho', hw'⟩ | hd'
        · exact .inl ⟨by simp only [proj_append]; rw [hp, hp'], ho', hw'⟩
        · exact .inr (Diverged.prepend hp hd')
      | failed _ | raised _ _ _ =>
        have hnv : ¬ vf := by simpa [OutRel] using ho
        obtain ⟨hp₁, hw₁⟩ := hc₁ hnv w₁ st₁ hs₁
        obtain ⟨hp₂, hw₂⟩ := hc₂ hnv w₂ st₀₂ hs₀₂
        rw [hok] at t₁ ⊢
        rw [hfl (by simp [Res.IsFail]) (hkf₂ _ _ hs₀₂).2 (hkf₂ _ _ hs₀₂).1 t₂]
        refine .inl ⟨by simp only [proj_append]; rw [hp₁, hp₂, hp], ?_,
          hw₁.symm.trans (hw.trans hw₂)⟩
        have hnf := (hkf₁ w₁ st₁ hs₁).1
        revert t₁ hnf
        generalize (k₁ w₁ st₁).2.1 = rk
        intro t₁ hnf
        cases rk <;> simp_all [OutRel, Term, Res.IsFail] <;> exact fun h => hnv (hvf h)
      | _ => simp [Term, finishEver] at t₂
    | failed _ | raised _ _ _ =>
      rw [hfl (by simp [Res.IsFail]) (hkf₁ _ _ hs₀₁).2 (hkf₁ _ _ hs₀₁).1 t₁]
      cases o₂ with
      | ok v₂ =>
        obtain ⟨st₂, rfl, hs₂⟩ := ht₂ _ rfl
        have hnv : ¬ vf := by simpa [OutRel] using ho
        obtain ⟨hp₁, hw₁⟩ := hc₁ hnv w₁ st₀₁ hs₀₁
        obtain ⟨hp₂, hw₂⟩ := hc₂ hnv w₂ st₂ hs₂
        rw [hok] at t₂ ⊢
        refine .inl ⟨by simp only [proj_append]; rw [hp₁, hp₂, hp], ?_,
          hw₁.symm.trans (hw.trans hw₂)⟩
        have hnf := (hkf₂ w₂ st₂ hs₂).1
        revert t₂ hnf
        generalize (k₂ w₂ st₂).2.1 = rk
        intro t₂ hnf
        clear hok hfl hT hk hkf₁ hkf₂ hc₁ hc₂
        cases rk <;> simp_all [OutRel, Term, Res.IsFail] <;> exact fun h => hnv (hvf h)
      | failed _ | raised _ _ _ =>
        have hkt₁ := termk_of t₁ (by simp [Res.IsFail])
        have hkt₂ := termk_of t₂ (by simp [Res.IsFail])
        rw [hfl (by simp [Res.IsFail]) (hkf₂ _ _ hs₀₂).2 (hkf₂ _ _ hs₀₂).1 t₂]
        rcases hk w₁ w₂ _ _ hw hs₀ hs₀₁ hs₀₂ hkt₁ hkt₂ with ⟨hp', _, hw'⟩ | hd'
        · exact .inl ⟨by simp only [proj_append]; rw [hp, hp'], OutRel.mono (fun _ _ h => h) hvf ho, hw'⟩
        · exact .inr (Diverged.prepend hp hd')
      | _ => simp [Term, finishEver] at t₂
    | _ => simp [Term, finishEver] at t₁
  · obtain ⟨u₁, hu₁⟩ := finishEver_fst k₁ st₀₁ (tr₁, o₁, w₁)
    obtain ⟨u₂, hu₂⟩ := finishEver_fst k₂ st₀₂ (tr₂, o₂, w₂)
    rw [hu₁, hu₂]
    exact .inr (hd.append u₁ u₂)

/-- `on stop` after a loop that does not exit, with a block that does not exit on the states it
runs on, does not exit. -/
theorem finishEver_noExit {k : World H → List Val → Run H} {st₀ : List Val} {r : Run H}
    (P : List Val → Prop) (hr : ∀ b vs, r.2.1 ≠ .exit b vs)
    (hrt : ∀ st, r.2.1 = .ok (.tuple st) → P st) (h₀ : P st₀)
    (hk : ∀ w st, P st → ∀ b vs, (k w st).2.1 ≠ .exit b vs) :
    ∀ b vs, (finishEver k st₀ r).2.1 ≠ .exit b vs := by
  obtain ⟨tr, o, w⟩ := r
  intro b vs
  simp only at hr hrt
  cases o with
  | ok v =>
    cases v with
    | tuple st =>
      simp only [finishEver]
      have := hk w st (hrt st rfl) b vs
      revert this
      rcases k w st with ⟨_, _, _⟩
      exact id
    | _ => simp [finishEver]
  | failed c =>
    simp only [finishEver]
    have := hk w st₀ h₀ b vs
    revert this
    rcases k w st₀ with ⟨_, r', _⟩
    cases r' <;> simp
  | raised kd c ps =>
    simp only [finishEver]
    have := hk w st₀ h₀ b vs
    revert this
    rcases k w st₀ with ⟨_, r', _⟩
    cases r' <;> simp
  | exit b' vs' => exact absurd rfl (hr b' vs')
  | _ => simp [finishEver]

end Una.F1
