/-
M4 completion, part A3: **a program checked at its instances**. The re-check replaces each
generic function it checks at its instances by those instances, renames the calls of it to the
instance they reach, and certifies the program it gets, `P₁`, with no hypothesis left
(`Check.progOk_sound_nil`). `progSim` tests, as a Boolean, that `P₁` agrees with the document's
program `P₂` up to annotations, an instance's name standing for its generic function's
(`progSim_sound`), and `t6_release_erasure` carries T6 from `P₁` to the runs of `P₂`: they are,
up to the annotations inside closures, runs `t6_release` relates.
-/
import UnaFormal.F1.Erasure
import UnaFormal.F1.Release
import UnaFormal.F1.CheckProgramSound

namespace Una.F1

deriving instance DecidableEq for Atom
deriving instance DecidableEq for Prim

mutual
  /-- Two right-hand sides agree up to annotations, the functions of the first renamed by `ρ`. -/
  def rhsSim (ρ : String → String) : Rhs → Rhs → Bool
    | .call f _ a, .call g _ b => ρ f == g && a == b
    | .funRef f _, .funRef g _ => ρ f == g
    | .lambda ps b, .lambda qs c => ps.map Prod.fst == qs.map Prod.fst && exprSim ρ b c
    | .atom a, .atom b => a == b
    | .prim p a, .prim q b => p == q && a == b
    | .apply f a, .apply g b => f == g && a == b
    | .op a, .op b => a == b
    | .get a, .get b => a == b
    | .construct r a, .construct s b => r == s && a == b
    | .field v i, .field u j => v == u && i == j
    | .list a, .list b => a == b
    | .none, .none => true
    | .some a, .some b => a == b
    | .declassify p a, .declassify q b => p == q && a == b
    | .map a f, .map b g => a == b && f == g
    | .filter a f, .filter b g => a == b && f == g
    | .firstWhere a f, .firstWhere b g => a == b && f == g
    | .fold a i f, .fold b j g => a == b && i == j && f == g
    | .parse a, .parse b => a == b
    | .eq a b, .eq c d => a == c && b == d
    | _, _ => false

  /-- Two bodies agree up to annotations, the functions of the first renamed by `ρ`. -/
  def exprSim (ρ : String → String) : Expr → Expr → Bool
    | .atom a, .atom b => a == b
    | .letR x _ v b, .letR y _ u c => x == y && rhsSim ρ v u && exprSim ρ b c
    | .letE x _ v b, .letE y _ u c => x == y && exprSim ρ v u && exprSim ρ b c
    | .ite a t e, .ite b u f => a == b && exprSim ρ t u && exprSim ρ e f
    | .caseOption a t x e, .caseOption b u y f =>
      a == b && x == y && exprSim ρ t u && exprSim ρ e f
    | .caseResult a x t y e, .caseResult b x' u y' f =>
      a == b && x == x' && y == y' && exprSim ρ t u && exprSim ρ e f
    | .fail c, .fail d => c == d
    | .attempt b, .attempt c => exprSim ρ b c
    | .handle b x h, .handle c y k => x == y && exprSim ρ b c && exprSim ρ h k
    | _, _ => false
end

mutual
  theorem rhsSim_sound {ρ : String → String} :
      ∀ (r s : Rhs), rhsSim ρ r s = true → r.anon ρ = s.anon id
    | .lambda ps b, s, h => by
      cases s with
      | lambda qs c =>
        simp only [rhsSim, Bool.and_eq_true, beq_iff_eq] at h
        simp only [Rhs.anon]
        rw [exprSim_sound b c h.2]
        congr 1
        have h1 := h.1
        clear h
        induction ps generalizing qs with
        | nil => cases qs <;> simp_all
        | cons p ps ih =>
          cases qs with
          | nil => simp at h1
          | cons q qs =>
            simp only [List.map_cons, List.cons.injEq] at h1 ⊢
            exact ⟨by rw [h1.1], ih qs h1.2⟩
      | _ => simp [rhsSim] at h
    | .call _ _ _, s, h | .funRef _ _, s, h | .atom _, s, h | .prim _ _, s, h
    | .apply _ _, s, h | .op _, s, h | .get _, s, h | .construct _ _, s, h | .field _ _, s, h
    | .list _, s, h | .none, s, h | .some _, s, h | .declassify _ _, s, h | .map _ _, s, h
    | .filter _ _, s, h | .firstWhere _ _, s, h | .fold _ _ _, s, h | .parse _, s, h
    | .eq _ _, s, h => by
      cases s <;> simp_all [rhsSim, Rhs.anon]

  theorem exprSim_sound {ρ : String → String} :
      ∀ (e f : Expr), exprSim ρ e f = true → e.anon ρ = f.anon id
    | .atom a, f, h => by cases f <;> simp_all [exprSim, Expr.anon]
    | .fail c, f, h => by cases f <;> simp_all [exprSim, Expr.anon]
    | .letR x _ v b, f, h => by
      cases f with
      | letR y _ u c =>
        simp only [exprSim, Bool.and_eq_true, beq_iff_eq] at h
        simp only [Expr.anon, h.1.1, rhsSim_sound v u h.1.2, exprSim_sound b c h.2]
      | _ => simp [exprSim] at h
    | .letE x _ v b, f, h => by
      cases f with
      | letE y _ u c =>
        simp only [exprSim, Bool.and_eq_true, beq_iff_eq] at h
        simp only [Expr.anon, h.1.1, exprSim_sound v u h.1.2, exprSim_sound b c h.2]
      | _ => simp [exprSim] at h
    | .ite a t e, f, h => by
      cases f with
      | ite b u g =>
        simp only [exprSim, Bool.and_eq_true, beq_iff_eq] at h
        simp only [Expr.anon, h.1.1, exprSim_sound t u h.1.2, exprSim_sound e g h.2]
      | _ => simp [exprSim] at h
    | .caseOption a t x e, f, h => by
      cases f with
      | caseOption b u y g =>
        simp only [exprSim, Bool.and_eq_true, beq_iff_eq] at h
        simp only [Expr.anon, h.1.1.1, h.1.1.2, exprSim_sound t u h.1.2, exprSim_sound e g h.2]
      | _ => simp [exprSim] at h
    | .caseResult a x t y e, f, h => by
      cases f with
      | caseResult b x' u y' g =>
        simp only [exprSim, Bool.and_eq_true, beq_iff_eq] at h
        simp only [Expr.anon, h.1.1.1.1, h.1.1.1.2, h.1.1.2, exprSim_sound t u h.1.2,
          exprSim_sound e g h.2]
      | _ => simp [exprSim] at h
    | .attempt b, f, h => by
      cases f with
      | attempt c =>
        simp only [exprSim] at h
        simp only [Expr.anon, exprSim_sound b c h]
      | _ => simp [exprSim] at h
    | .handle b x k, f, h => by
      cases f with
      | handle c y k' =>
        simp only [exprSim, Bool.and_eq_true, beq_iff_eq] at h
        simp only [Expr.anon, h.1.1, exprSim_sound b c h.1.2, exprSim_sound k k' h.2]
      | _ => simp [exprSim] at h
end

/-- The renaming of a re-check: an instance's name to its generic function's, a generic function
checked at its instances to a name no function has, and every other name to itself. -/
def renOf (ren : List (String × String)) (f : String) : String := (ren.lookup f).getD f

/-- Two functions agree up to annotations, the first renamed by `ρ`. -/
def funSim (ρ : String → String) (fn₁ fn₂ : Fun) : Bool :=
  ρ fn₁.name == fn₂.name && fn₁.params.map Prod.fst == fn₂.params.map Prod.fst &&
    exprSim ρ fn₁.body fn₂.body

theorem funSim_sound {ρ : String → String} {fn₁ fn₂ : Fun} (h : funSim ρ fn₁ fn₂ = true) :
    Fun.anon ρ fn₁ = Fun.anon id fn₂ := by
  simp only [funSim, Bool.and_eq_true, beq_iff_eq] at h
  obtain ⟨⟨hn, hp⟩, hb⟩ := h
  simp only [Fun.anon, hn, exprSim_sound _ _ hb, anonParams, id]
  congr 1
  have : (fn₁.params.map Prod.fst).map (fun x => (x, anonTy)) =
      (fn₂.params.map Prod.fst).map (fun x => (x, anonTy)) := by rw [hp]
  simpa [List.map_map, Function.comp_def] using this

theorem lookup_mem {ren : List (String × String)} {f v : String} :
    ren.lookup f = some v → (f, v) ∈ ren := by
  induction ren with
  | nil => simp
  | cons kv ren ih =>
    obtain ⟨k, w⟩ := kv
    simp only [List.lookup]
    split
    · rename_i hk
      intro h
      simp only [beq_iff_eq] at hk
      cases h
      simp [hk]
    · intro h
      exact List.mem_cons_of_mem _ (ih h)

theorem lookup_isSome_of_mem {ren : List (String × String)} {k v : String} :
    (k, v) ∈ ren → (ren.lookup k).isSome := by
  induction ren with
  | nil => simp
  | cons kv ren ih =>
    obtain ⟨k', w⟩ := kv
    intro h
    simp only [List.lookup]
    split
    · rfl
    · rename_i hk
      rcases List.mem_cons.mp h with h | h
      · simp only [Prod.mk.injEq] at h
        simp [h.1] at hk
      · exact ih h

/-- **`P₁` agrees with `P₂` up to annotations**, as a Boolean: every function of `P₁` agrees
with the function of `P₂` its renamed name finds; a name `P₁` lacks finds nothing in `P₂` once
renamed; and the renaming keeps the `via` functions. -/
def progSim (ren : List (String × String)) (P₁ P₂ : Program) : Bool :=
  P₁.funs.all (fun fn₁ => match P₂.find (renOf ren fn₁.name) with
    | some fn₂ => funSim (renOf ren) fn₁ fn₂
    | none => false) &&
  ren.all (fun kv => P₁.funs.any (·.name == kv.1) || !P₂.funs.any (·.name == kv.2)) &&
  P₂.funs.all (fun fn₂ => P₁.funs.any (·.name == fn₂.name) || ren.any (·.1 == fn₂.name)) &&
  P₂.policies.all (fun po => renOf ren po.via == po.via)

theorem find_none_of {p : Program} {f : String} (h : p.funs.any (·.name == f) = false) :
    p.find f = none := by
  unfold Program.find
  rw [List.find?_eq_none]
  intro fn hfn hname
  have := List.any_eq_false.mp h fn hfn
  simp_all

theorem any_of_find {p : Program} {f : String} {fn : Fun} (h : p.find f = some fn) :
    p.funs.any (·.name == f) = true := by
  obtain ⟨hmem, hname⟩ := Program.find_some h
  exact List.any_eq_true.mpr ⟨fn, hmem, by simp [hname]⟩

/-- The Boolean is sound: a program the re-check builds from the document's program `P₂`, with
its functions `fs`, agrees with it up to annotations. -/
theorem progSim_sound {ren : List (String × String)} {P₂ : Program} {fs : List Fun}
    (h : progSim ren { P₂ with funs := fs } P₂ = true) :
    Sim (renOf ren) { P₂ with funs := fs } P₂ := by
  simp only [progSim, Bool.and_eq_true, List.all_eq_true] at h
  obtain ⟨⟨⟨hfuns, hren⟩, hcover⟩, hvia⟩ := h
  refine ⟨fun f => ?_, rfl, fun po hpo => by simpa using hvia po hpo⟩
  cases h₁ : Program.find { P₂ with funs := fs } f with
  | some fn₁ =>
    obtain ⟨hmem, hname⟩ := Program.find_some h₁
    have := hfuns fn₁ hmem
    rw [hname] at this
    split at this
    · rename_i fn₂ hf₂
      simp only [Option.map_some, hf₂, funSim_sound this]
    · simp at this
  | none =>
    have hno : fs.any (·.name == f) = false := by
      cases ha : fs.any (·.name == f) with
      | false => rfl
      | true =>
        obtain ⟨fn, hfn, he⟩ := List.any_eq_true.mp ha
        have : Program.find { P₂ with funs := fs } f ≠ none := by
          unfold Program.find
          intro hn
          rw [List.find?_eq_none] at hn
          exact hn fn hfn (by simpa using he)
        exact absurd h₁ this
    simp only [Option.map_none]
    cases hl : ren.lookup f with
    | some v =>
      have hmem : (f, v) ∈ ren := lookup_mem hl
      have := hren (f, v) hmem
      simp only [hno, Bool.false_or, Bool.not_eq_true'] at this
      simp [renOf, hl, find_none_of this]
    | none =>
      have hrf : renOf ren f = f := by simp [renOf, hl]
      rw [hrf]
      cases h₂ : P₂.find f with
      | none => rfl
      | some fn₂ =>
        obtain ⟨hmem₂, hname₂⟩ := Program.find_some h₂
        have := hcover fn₂ hmem₂
        rw [hname₂] at this
        simp only [hno, Bool.false_or, List.any_eq_true, beq_iff_eq] at this
        obtain ⟨kv, hkv, hk⟩ := this
        have : (ren.lookup f).isSome := by
          rw [← hk]
          exact lookup_isSome_of_mem hkv
        simp [hl] at this

/-- **T6 for a program checked at its instances.** When `P₁`, a program the labeled typing accepts
within B1, agrees with `P₂` up to annotations, two runs of a body of `P₂` are, up to the
annotations inside closures, two runs of the corresponding body of `P₁` that `t6_release`
relates: equal to the observer, or diverging at a release. -/
theorem t6_release_erasure {ρ : String → String} {P₁ P₂ : Program} (L : Label) (hsim : Sim ρ P₁ P₂)
    (hwf : P₁.LWellFormed) (hb1 : P₁.B1WellFormed)
    (hget : P₁.le (.ofLabel P₁.acceptsGet) (.ofLabel P₁.getResult)) (w : World)
    {σ : List (String × LTy)} {e₁ e₂ : Env} {Γ : LCtx} {E : List String} {pc : LExpr} {e : Expr}
    {t : LTy} {ε : Eff} (hσ : GroundSubst σ) (hE : P₁.EqArgsOK σ E) (hΓ : Γ.InPlace)
    (hpc : pc.PlainFree) (h₁ : LEnvTy P₁ e₁ (Γ.lsub P₁.recordDeep σ))
    (h₂ : LEnvTy P₁ e₂ (Γ.lsub P₁.recordDeep σ)) (hrel : EnvRel P₁ L (Γ.lsub P₁.recordDeep σ) e₁ e₂)
    (he : LExprTy P₁ E Γ pc e t ε) {e' : Expr} {e₁' e₂' : Env} (hee : e.anon ρ = e'.anon id)
    (he₁ : Val.anonEnv ρ e₁ = Val.anonEnv id e₁') (he₂ : Val.anonEnv ρ e₂ = Val.anonEnv id e₂')
    (n : Nat) :
    ∃ r₁ r₂, BRelR P₁ L (V P₁ L (t.lsubst P₁.recordDeep σ)) (Vis P₁ L (ε.inst (P₁.sd σ)).failLabel) r₁ r₂ ∧
      runAnon ρ r₁ = runAnon id (eval n P₂ w e₁' e') ∧ runAnon ρ r₂ = runAnon id (eval n P₂ w e₂' e') :=
  ⟨_, _, t6_release P₁ L hwf hb1 hget w hσ hE hΓ hpc h₁ h₂ hrel he n,
    eval_erasure hsim w n he₁ hee, eval_erasure hsim w n he₂ hee⟩

end Una.F1
