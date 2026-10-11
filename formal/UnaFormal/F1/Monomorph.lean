/-
M4 completion, part A3: **a program checked at its instances**. The re-check replaces each
generic function it checks at its instances by those instances, renames the calls of it to the
instance they reach, and certifies the program it gets, `P₁`, with no hypothesis left
(`Check.progOk_sound_nil`). `progSim` tests, as a Boolean, that `P₁` agrees with the document's
program `P₂` up to annotations, an instance's name standing for its generic function's
(`progSim_sound`), and `t6_release_erasure` carries T6 from `P₁` to the runs of `P₂`: they are,
up to the annotations inside closures, runs `t6_release_host` relates.
-/
import UnaFormal.F1.Erasure
import UnaFormal.F1.Release
import UnaFormal.F1.CheckProgramSound

namespace Una.F1

deriving instance DecidableEq for Atom
deriving instance DecidableEq for Prim

/-- Two loop sources are the same. -/
def sourceEq : Source → Source → Bool
  | .list a, .list b => a == b
  | .range l h i, .range l' h' i' => l == l' && h == h' && i == i'
  | _, _ => false

theorem sourceEq_sound : ∀ {s t : Source}, sourceEq s t = true → s = t
  | .list a, .list b, h => by simp only [sourceEq, beq_iff_eq] at h; rw [h]
  | .range _ _ _, .range _ _ _, h => by
    simp only [sourceEq, Bool.and_eq_true, beq_iff_eq] at h
    rw [h.1.1, h.1.2, h.2]
  | .list _, .range _ _ _, h | .range _ _ _, .list _, h => by simp [sourceEq] at h

/-- Binders with the same names agree once their types are forgotten. -/
theorem anonBinders_eq : ∀ {ps qs : List (String × LTy)}, ps.map Prod.fst = qs.map Prod.fst →
    ps.map (fun q => (q.1, anonTy)) = qs.map (fun q => (q.1, anonTy))
  | [], [], _ => rfl
  | p :: ps, q :: qs, h => by
    simp only [List.map_cons, List.cons.injEq] at h ⊢
    exact ⟨by rw [h.1], anonBinders_eq h.2⟩
  | [], _ :: _, h | _ :: _, [], h => by simp at h

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
    | .read a, .read b => a == b
    | .write a c, .write b d => a == b && c == d
    | .req m a, .req k b => m == k && a == b
    | .construct r a, .construct s b => r == s && a == b
    | .variant k i a, .variant k' i' b => k == k' && i == i' && a == b
    | .field v i, .field u j => v == u && i == j
    | .list a, .list b => a == b
    | .none, .none => true
    | .some a, .some b => a == b
    | .mkOk a, .mkOk b => a == b
    | .mkErr a, .mkErr b => a == b
    | .declassify p a, .declassify q b => p == q && a == b
    | .map a f, .map b g => a == b && f == g
    | .filter a f, .filter b g => a == b && f == g
    | .firstWhere a f, .firstWhere b g => a == b && f == g
    | .fold a i f, .fold b j g => a == b && i == j && f == g
    | .parse a, .parse b => a == b
    | .checked a p c, .checked b q d => a == b && p == q && c == d
    | .eq a b, .eq c d => a == c && b == d
    | .tuple a, .tuple b => a == b
    | .append a x, .append b y => a == b && x == y
    | .set a i x, .set b j y => a == b && i == j && x == y
    | .toText a, .toText b => a == b
    | .envGet a, .envGet b => a == b
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
    | .raise k c xs, .raise k' d ys => k == k' && c == d && xs == ys
    | .caseSum a i xs t e, .caseSum b j ys u f =>
      a == b && i == j && xs == ys && exprSim ρ t u && exprSim ρ e f
    | .unreachable, .unreachable => true
    | .attempt b, .attempt c => exprSim ρ b c
    | .attemptK k b, .attemptK k' c => k == k' && exprSim ρ b c
    | .handle b x h, .handle c y k => x == y && exprSim ρ b c && exprSim ρ h k
    | .handleK kd b x h, .handleK kd' c y k => kd == kd' && x == y && exprSim ρ b c && exprSim ρ h k
    | .letTuple bs a b, .letTuple cs a' c =>
      bs.map Prod.fst == cs.map Prod.fst && a == a' && exprSim ρ b c
    | .loop s x _ st i b, .loop s' y _ st' j c =>
      sourceEq s s' && x == y && st.map Prod.fst == st'.map Prod.fst && i == j && exprSim ρ b c
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
    | .apply _ _, s, h | .op _, s, h | .read _, s, h | .write _ _, s, h | .req _ _, s, h
    | .construct _ _, s, h | .field _ _, s, h
    | .list _, s, h | .none, s, h | .some _, s, h | .mkOk _, s, h | .mkErr _, s, h
    | .declassify _ _, s, h | .map _ _, s, h
    | .filter _ _, s, h | .firstWhere _ _, s, h | .fold _ _ _, s, h | .parse _, s, h
    | .checked _ _ _, s, h
    | .eq _ _, s, h | .tuple _, s, h | .append _ _, s, h | .set _ _ _, s, h | .toText _, s, h
    | .envGet _, s, h => by
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
    | .letTuple bs a b, f, h => by
      cases f with
      | letTuple cs a' c =>
        simp only [exprSim, Bool.and_eq_true, beq_iff_eq] at h
        simp only [Expr.anon, anonBinders_eq h.1.1, h.1.2, exprSim_sound b c h.2]
      | _ => simp [exprSim] at h
    | .loop s x _ st i b, f, h => by
      cases f with
      | loop s' y _ st' j c =>
        simp only [exprSim, Bool.and_eq_true, beq_iff_eq] at h
        simp only [Expr.anon, sourceEq_sound h.1.1.1.1, h.1.1.1.2, anonBinders_eq h.1.1.2, h.1.2,
          exprSim_sound b c h.2]
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
  refine ⟨fun f => ?_, rfl, fun po hpo => by simpa using hvia po hpo, rfl⟩
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
annotations inside closures, two runs of the corresponding body of `P₁` that `t6_release_host`
relates: equal to the observer, or diverging at a release. -/
theorem t6_release_erasure {ρ : String → String} {P₁ P₂ : Program} (L : Label) (hsim : Sim ρ P₁ P₂)
    (hwf : P₁.LWellFormed) (hb1 : P₁.B1WellFormed) (hh : P₁.HostWF) {H : Host} (hlaw : H.Lawful)
    (hai : H.AnonInvariant) {w₁ w₂ : World H}
    {σ : List (String × LTy)} {e₁ e₂ : Env} {Γ : LCtx} {E : List String} {pc : LExpr} {e : Expr}
    {t : LTy} {ε : Eff} (hσ : GroundSubst σ) (hE : P₁.EqArgsOK σ E) (hΓ : Γ.InPlace)
    (hpc : pc.PlainFree) (hw : WorldRel P₁ L w₁ w₂) (h₁ : LEnvTy P₁ e₁ (Γ.lsub P₁.recordDeep σ))
    (h₂ : LEnvTy P₁ e₂ (Γ.lsub P₁.recordDeep σ)) (hrel : EnvRel P₁ L (Γ.lsub P₁.recordDeep σ) e₁ e₂)
    (he : LExprTy P₁ E Γ pc e t ε) {e' : Expr} {e₁' e₂' : Env} (hee : e.anon ρ = e'.anon id)
    (he₁ : Val.anonEnv ρ e₁ = Val.anonEnv id e₁') (he₂ : Val.anonEnv ρ e₂ = Val.anonEnv id e₂')
    (n : Nat) :
    ∃ r₁ r₂, BRelR P₁ L (V P₁ L (t.lsubst P₁.recordDeep σ)) (Vis P₁ L (ε.inst (P₁.sd σ)).failLabel) r₁ r₂ ∧
      runAnon ρ r₁ = runAnon id (eval H n P₂ w₁ e₁' e') ∧
      runAnon ρ r₂ = runAnon id (eval H n P₂ w₂ e₂' e') :=
  ⟨_, _, t6_release_host P₁ L H hwf hb1 hh hlaw hσ hE hΓ hpc hw h₁ h₂ hrel he n,
    eval_erasure hsim hai hlaw.resp_typed w₁ n he₁ hee, eval_erasure hsim hai hlaw.resp_typed w₂ n he₂ hee⟩

/-- A value of a scalar type is first-order. -/
theorem Val.firstOrder_of_scalar {p : Program} {v : Val} {t : Ty} (hs : t.Scalar)
    (hv : ValTy p v t) : v.FirstOrder := by
  cases t <;> simp [Ty.Scalar] at hs <;> cases hv <;> simp [Val.FirstOrder]

/-- Texts are first-order. -/
theorem Val.firstOrderAll_of_texts : ∀ {vs : List Val}, (∀ v ∈ vs, ∃ s, v = .text s) →
    Val.FirstOrderAll vs
  | [], _ => by simp [Val.FirstOrderAll]
  | v :: vs, h => by
    obtain ⟨s, rfl⟩ := h v (by simp)
    simp only [Val.FirstOrderAll, Val.FirstOrder, true_and]
    exact Val.firstOrderAll_of_texts fun u hu => h u (by simp [hu])

/-- **The events of a typed run are first-order** (#457), in a program within B1, on any host. A
release carries the value of its `via` function, typed at the policy's `to`, which B1 makes
scalar, or its failure `Err(c)`; a request carries its arguments, which T-Request types at `Http`'s parameter types, whose
values hold no closure (M5c). -/
theorem events_firstOrder {p : Program} {H : Host} (hwf : p.LWellFormed) (hb1 : p.B1WellFormed)
    (h4 : H.RespTyped)
    {σ : List (String × LTy)} {w : World H} {env : Env} {Γ : LCtx} {E : List String} {pc : LExpr}
    {e : Expr} {t : LTy} {ε : Eff} (hσ : GroundSubst σ) (henv : LEnvTy p env (Γ.lsub p.recordDeep σ))
    (he : LExprTy p E Γ pc e t ε) (n : Nat) : ∀ ev ∈ (eval H n p w env e).1, ev.FirstOrder := by
  intro ev hev
  have ha := ((unary_soundR hwf hb1 h4 n).2 hσ henv he).1 ev hev
  cases ev with
  | release pol v =>
    obtain ⟨_, po, hpo, _, hv⟩ := ha
    have hmem : po ∈ p.policies := by
      unfold Program.policy at hpo
      exact List.mem_of_find?_eq_some hpo
    rcases hv with hv | ⟨c, rfl⟩
    · exact Val.firstOrder_of_scalar (hb1.dst po hmem).2 hv
    · -- A failing `via` releases `Err(c)` (#453 Phase B), which is first-order.
      trivial
  | req m vs =>
    simp only [Event.FirstOrder]
    exact ha.2.2
  | out _ | read _ | write _ _ => trivial

/-- A release in an erased trace comes from a release, whose value erases to the same. -/
theorem split_release {ρ : String → String} {t pre rest : Trace} {pol : String} {v : Val}
    (h : t.map (Event.anon id) = pre.map (Event.anon ρ) ++ Event.release pol (v.anon ρ) :: rest) :
    ∃ A v' C, t = A ++ Event.release pol v' :: C ∧ A.map (Event.anon id) = pre.map (Event.anon ρ) ∧
      v'.anon id = v.anon ρ := by
  obtain ⟨A, B, rfl, hA, hB⟩ := List.map_eq_append_iff.mp h
  obtain ⟨x, C, rfl, hx, _⟩ := List.map_eq_cons_iff.mp hB
  cases x with
  | release pol' v' =>
    simp only [Event.anon, Event.release.injEq] at hx
    obtain ⟨rfl, hv⟩ := hx
    exact ⟨A, v', C, rfl, hA, hv⟩
  | out _ | read _ | write _ _ | req _ _ => simp [Event.anon] at hx

/-- **What the observer sees of the document's runs.** With `P₁` as in `t6_release_erasure`, and
`proj` the same in both programs (it reads no function), two runs of a body of `P₂` that end,
are equal to the observer, or diverge at a release: what `t6_release_host` says of `P₁`, now of
`P₂`'s own runs. Their events are first-order (`events_firstOrder`, through erasure). -/
theorem t6_release_erasure_obs {ρ : String → String} {P₁ P₂ : Program} (L : Label)
    (hsim : Sim ρ P₁ P₂) (hproj : ∀ t, proj P₁ L t = proj P₂ L t)
    (hwf : P₁.LWellFormed) (hb1 : P₁.B1WellFormed) (hh : P₁.HostWF) {H : Host} (hlaw : H.Lawful)
    (hai : H.AnonInvariant) {w₁ w₂ : World H}
    {σ : List (String × LTy)} {e₁ e₂ : Env} {Γ : LCtx} {E : List String} {pc : LExpr} {e : Expr}
    {t : LTy} {ε : Eff} (hσ : GroundSubst σ) (hE : P₁.EqArgsOK σ E) (hΓ : Γ.InPlace)
    (hpc : pc.PlainFree) (hw : WorldRel P₁ L w₁ w₂) (h₁ : LEnvTy P₁ e₁ (Γ.lsub P₁.recordDeep σ))
    (h₂ : LEnvTy P₁ e₂ (Γ.lsub P₁.recordDeep σ)) (hrel : EnvRel P₁ L (Γ.lsub P₁.recordDeep σ) e₁ e₂)
    (he : LExprTy P₁ E Γ pc e t ε) {e' : Expr} {e₁' e₂' : Env} (hee : e.anon ρ = e'.anon id)
    (he₁ : Val.anonEnv ρ e₁ = Val.anonEnv id e₁') (he₂ : Val.anonEnv ρ e₂ = Val.anonEnv id e₂')
    (n : Nat) (hT₁ : Term (eval H n P₂ w₁ e₁' e').2.1) (hT₂ : Term (eval H n P₂ w₂ e₂' e').2.1) :
    proj P₂ L (eval H n P₂ w₁ e₁' e').1 = proj P₂ L (eval H n P₂ w₂ e₂' e').1 ∨
      Diverged P₂ L (eval H n P₂ w₁ e₁' e').1 (eval H n P₂ w₂ e₂' e').1 := by
  -- The document's events are first-order: they erase to the events of `P₁`'s typed run.
  have hfo : ∀ {wa : World H} {ea ea' : Env}, LEnvTy P₁ ea (Γ.lsub P₁.recordDeep σ) →
      Val.anonEnv ρ ea = Val.anonEnv id ea' → ∀ ev ∈ (eval H n P₂ wa ea' e').1, ev.FirstOrder := by
    intro wa ea ea' hty hea ev hev
    have herase := congrArg Prod.fst (eval_erasure hsim hai hlaw.resp_typed wa n hea hee)
    simp only [runAnon] at herase
    have hm : ev.anon id ∈ (eval H n P₁ wa ea e).1.map (Event.anon ρ) := by
      rw [herase]
      exact List.mem_map_of_mem hev
    obtain ⟨ev', hev', heq⟩ := List.mem_map.mp hm
    rw [← Event.firstOrder_anon (ρ := id), ← heq, Event.firstOrder_anon]
    exact events_firstOrder hwf hb1 hlaw.resp_typed hσ hty he n ev' hev'
  have hfo₁ := hfo (wa := w₁) h₁ he₁
  have hfo₂ := hfo (wa := w₂) h₂ he₂
  have hbr := t6_release_host P₁ L H hwf hb1 hh hlaw hσ hE hΓ hpc hw h₁ h₂ hrel he n
  have ha := eval_erasure hsim hai hlaw.resp_typed w₁ n he₁ hee
  have hb := eval_erasure hsim hai hlaw.resp_typed w₂ n he₂ hee
  generalize eval H n P₁ w₁ e₁ e = r₁ at hbr ha
  generalize eval H n P₁ w₂ e₂ e = r₂ at hbr hb
  generalize eval H n P₂ w₁ e₁' e' = s₁ at ha hT₁ hfo₁ ⊢
  generalize eval H n P₂ w₂ e₂' e' = s₂ at hb hT₂ hfo₂ ⊢
  obtain ⟨t₁, o₁, x₁⟩ := r₁
  obtain ⟨t₂, o₂, x₂⟩ := r₂
  obtain ⟨u₁, q₁, y₁⟩ := s₁
  obtain ⟨u₂, q₂, y₂⟩ := s₂
  simp only [runAnon, Prod.mk.injEq] at ha hb
  have hterm : ∀ {o : Res} {q : Res}, o.anon ρ = q.anon id → Term q → Term o := fun h hq => by
    rw [← Term_anon (ρ := ρ), h, Term_anon]; exact hq
  -- What the observer sees of `P₂`'s run, erased, is what it sees of `P₁`'s.
  have hview : ∀ {tt : Trace} {uu : Trace}, tt.map (Event.anon ρ) = uu.map (Event.anon id) →
      (proj P₂ L uu).map (Event.anon id) = (proj P₁ L tt).map (Event.anon ρ) := fun h => by
    rw [← proj_anon, ← h, ← hproj, proj_anon]
  have hfoP : ∀ {uu : Trace}, (∀ ev ∈ uu, ev.FirstOrder) → ∀ ev ∈ proj P₂ L uu, ev.FirstOrder :=
    fun h ev hev => h ev (List.mem_filter.mp hev).1
  rcases hbr (hterm ha.2.1 hT₁) (hterm hb.2.1 hT₂) with ⟨hp, _⟩ | ⟨pre, pol, v₁, v₂, r₁, r₂, hne, hp₁, hp₂⟩
  · left
    apply trace_anon_inj (ρ := id) (ρ' := id) (hfoP hfo₁) (hfoP hfo₂)
    simp only at hp ⊢
    rw [hview ha.1, hview hb.1, hp]
  · right
    have h₁' := hview ha.1
    have h₂' := hview hb.1
    simp only at hp₁ hp₂
    rw [hp₁, List.map_append, List.map_cons, Event.anon] at h₁'
    rw [hp₂, List.map_append, List.map_cons, Event.anon] at h₂'
    obtain ⟨A, v₁', C, hA, hpre₁, hv₁⟩ := split_release h₁'
    obtain ⟨A', v₂', C', hA', hpre₂, hv₂⟩ := split_release h₂'
    have hfoA : ∀ ev ∈ A, ev.FirstOrder := fun ev hev => hfoP hfo₁ ev (by rw [hA]; simp [hev])
    have hfoA' : ∀ ev ∈ A', ev.FirstOrder := fun ev hev => hfoP hfo₂ ev (by rw [hA']; simp [hev])
    have hAA : A = A' := trace_anon_inj (ρ := id) (ρ' := id) hfoA hfoA' (by rw [hpre₁, hpre₂])
    have hfo₁' : v₁'.FirstOrder := by
      have := hfoP hfo₁ (Event.release pol v₁') (by rw [hA]; simp)
      simpa [Event.FirstOrder] using this
    have hfo₂' : v₂'.FirstOrder := by
      have := hfoP hfo₂ (Event.release pol v₂') (by rw [hA']; simp)
      simpa [Event.FirstOrder] using this
    refine ⟨A, pol, v₁', v₂', C, C', fun heq => hne ?_, hA, hAA ▸ hA'⟩
    have hv₁fo : v₁.FirstOrder := by
      rw [← Val.firstOrder_anon (ρ := ρ), ← hv₁, Val.firstOrder_anon]; exact hfo₁'
    have hv₂fo : v₂.FirstOrder := by
      rw [← Val.firstOrder_anon (ρ := ρ), ← hv₂, Val.firstOrder_anon]; exact hfo₂'
    apply Val.anon_inj (ρ := ρ) (ρ' := ρ) v₁ v₂ hv₁fo hv₂fo
    rw [← hv₁, ← hv₂, heq]

/-- `proj` reads no function: it is the same in a program checked at instances. -/
theorem proj_funs (P₂ : Program) (fs : List Fun) (L : Label) (t : Trace) :
    proj ({ P₂ with funs := fs } : Program) L t = proj P₂ L t := rfl

end Una.F1
