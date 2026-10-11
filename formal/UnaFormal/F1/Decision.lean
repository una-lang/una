/-
The decision label (#524, the coordinator's ruling D, `docs/plan/formal-track-handlers.md` §4a).

An effect's failure has two labels: `failLabel`, where it happens (the program counter and the
data at the failing site), and `dl`, what decides whether a step both runs reach fails, and how
(the data only). This file proves, of every typing:

- **a failure happens at or above the program counter** (`fails = some l → pc ⊑ l`), and
- **`dl ⊑ failLabel`**,

so a relation of outcomes stated at `dl`'s visibility gives the one at `failLabel`'s.
-/
import UnaFormal.F1.LTyping
import UnaFormal.F1.Order

namespace Una.F1

variable {p : Program}

/-- What the typing guarantees of an effect at `pc` (#524): a failure at or above `pc`, and the
decision label at or below the failure label. -/
def Eff.DlOk (p : Program) (pc : LExpr) (ε : Eff) : Prop :=
  (∀ l, ε.fails = some l → p.le pc l) ∧ p.le ε.dl ε.failLabel

namespace Eff

theorem dlOk_none {pc : LExpr} : Eff.none.DlOk p pc :=
  ⟨fun _ h => by simp [Eff.none] at h, LExpr.bot_le _⟩

theorem DlOk.mono {pc pc' : LExpr} {ε : Eff} (h : ε.DlOk p pc') (hpc : p.le pc pc') : ε.DlOk p pc :=
  ⟨fun l hl => LExpr.le_trans hpc (h.1 l hl), h.2⟩

/-- A failure labeled `⊥` and decided at `⊥`, which `pc ⊑ ⊥` allows. -/
theorem dlOk_ofFn {pc : LExpr} {o : Bool} {g : HostFx} {f : Bool} (hf : f = true → p.le pc .bot) :
    (Eff.ofFn o g f).DlOk p pc := by
  refine ⟨fun l hl => ?_, LExpr.bot_le _⟩
  cases f <;> simp [Eff.ofFn] at hl
  subst hl; exact hf rfl

theorem dlOk_ofCall {pc : LExpr} {fn : Fun} (hf : fn.fails = true → p.le pc .bot) :
    (Eff.ofCall fn).DlOk p pc := by
  have h := dlOk_ofFn (o := fn.op) (g := fn.host) hf
  exact ⟨fun l hl => h.1 l (by simpa [Eff.ofCall, Eff.ofFnR] using hl), LExpr.bot_le _⟩

theorem dlOk_ofInvoke {pc : LExpr} {fn : Fun} : (Eff.ofInvoke pc fn).DlOk p pc := by
  refine ⟨fun l hl => ?_, LExpr.bot_le _⟩
  cases hf : fn.fails <;> simp [Eff.ofInvoke, hf] at hl
  subst hl; exact LExpr.le_refl _

theorem dlOk_ofPolicy {pc dst : LExpr} {o : Bool} {g : HostFx} {f : Bool} {pol : String} :
    (Eff.ofPolicy o g f pc dst pol).DlOk p pc := by
  cases f
  · exact ⟨fun l hl => by simp [Eff.ofPolicy] at hl, by simp [Eff.ofPolicy, Eff.failLabel]; exact LExpr.bot_le _⟩
  · refine ⟨fun l hl => ?_, ?_⟩
    · simp [Eff.ofPolicy] at hl; subst hl; exact LExpr.le_join_left _ _
    · simp only [Eff.ofPolicy, Eff.failLabel, if_true, Option.getD_some]
      exact LExpr.le_join_right _ _

/-- A failure labeled `l`, with its decision label `d ⊑ l`, at `pc ⊑ l`. -/
theorem dlOk_leaf {pc l d : LExpr} {o : Bool} {g : HostFx} {rs : List String} {c : Bool}
    {ks : List String} (hpc : p.le pc l) (hd : p.le d l) : (⟨o, g, some l, rs, c, ks, d⟩ : Eff).DlOk p pc :=
  ⟨fun l' hl => by simp at hl; subst hl; exact hpc, by simpa [Eff.failLabel] using hd⟩

theorem failLabel_le_union_left (a b : Eff) : p.le a.failLabel (a.union b).failLabel := by
  cases ha : a.fails <;> cases hb : b.fails <;>
    simp [Eff.failLabel, Eff.union, ha, hb] <;>
    first | exact LExpr.le_refl _ | exact LExpr.bot_le _ | exact LExpr.le_join_left _ _

theorem failLabel_le_union_right (a b : Eff) : p.le b.failLabel (a.union b).failLabel := by
  cases ha : a.fails <;> cases hb : b.fails <;>
    simp [Eff.failLabel, Eff.union, ha, hb] <;>
    first | exact LExpr.le_refl _ | exact LExpr.bot_le _ | exact LExpr.le_join_right _ _

/-- A sequence, the second part at a program counter above the first's. -/
theorem DlOk.union {pc pc' : LExpr} {a b : Eff} (ha : a.DlOk p pc) (hb : b.DlOk p pc')
    (hpc : p.le pc pc') : (a.union b).DlOk p pc := by
  refine ⟨fun l hl => ?_, ?_⟩
  · cases h₁ : a.fails <;> cases h₂ : b.fails <;> simp [Eff.union, h₁, h₂] at hl <;> subst hl
    · exact LExpr.le_trans hpc (hb.1 _ h₂)
    · exact ha.1 _ h₁
    · exact LExpr.le_trans (ha.1 _ h₁) (LExpr.le_join_left _ _)
  · exact LExpr.join_le (LExpr.le_trans ha.2 (failLabel_le_union_left a b))
      (LExpr.le_trans hb.2 (failLabel_le_union_right a b))

/-- A choice decided at `c`, below the program counter its branches run under. -/
theorem DlOk.decide {pc c : LExpr} {ε : Eff} (h : ε.DlOk p pc) (hc : p.le c pc) :
    (ε.decide c).DlOk p pc := by
  unfold Eff.decide
  cases hf : ε.fails with
  | none => simpa [hf] using h
  | some l =>
    refine ⟨fun l' hl => h.1 l' (by simpa [hf] using hl), ?_⟩
    simp only [hf, Option.isSome_some, if_true, Eff.failLabel, Option.getD_some]
    have h2 : p.le ε.dl l := by simpa [Eff.failLabel, hf] using h.2
    exact LExpr.join_le h2 (LExpr.le_trans hc (h.1 l hf))

theorem dlOk_attemptOf {pc : LExpr} (ε : Eff) : (Eff.attemptOf ε).DlOk p pc :=
  ⟨fun _ h => by simp [Eff.attemptOf] at h, LExpr.bot_le _⟩

/-- A handled expression: the handler runs under `pc ⊔ failLabel` of the body. -/
theorem DlOk.handleOf {pc : LExpr} {ε₁ ε₂ : Eff} (h₁ : ε₁.DlOk p pc)
    (h₂ : ε₂.DlOk p (pc.join ε₁.failLabel)) : (Eff.handleOf ε₁ ε₂).DlOk p pc := by
  have hi : (⟨ε₁.op || ε₂.op, ε₁.host.or ε₂.host, ε₂.fails, ε₁.rels ++ ε₂.rels, ε₂.codes, ε₂.raises,
      ε₂.dl⟩ : Eff).DlOk p (pc.join ε₁.failLabel) := ⟨h₂.1, h₂.2⟩
  exact (DlOk.decide hi (LExpr.le_trans h₁.2 (LExpr.le_join_right _ _))).mono (LExpr.le_join_left _ _)

end Eff

theorem Program.fsFail_mono {a b : LExpr} (h : p.le a b) : p.le (p.fsFail a) (p.fsFail b) := by
  unfold Program.fsFail
  split
  · exact h
  · exact LExpr.join_le (LExpr.le_trans h (LExpr.le_join_left _ _)) (LExpr.le_join_right _ _)

theorem Program.le_fsFail (a : LExpr) : p.le a (p.fsFail a) := by
  unfold Program.fsFail
  split
  · exact LExpr.le_refl _
  · exact LExpr.le_join_left _ _

theorem Program.reqFail_mono {a b : LExpr} (ts : List LTy) (h : p.le a b) :
    p.le (p.reqFail a ts) (p.reqFail b ts) := by
  unfold Program.reqFail
  split
  · exact LExpr.join_le (LExpr.le_trans h (LExpr.le_join_left _ _)) (LExpr.le_join_right _ _)
  · refine LExpr.join_le (LExpr.join_le ?_ ?_) (LExpr.le_join_right _ _)
    · exact LExpr.le_trans h (LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_left _ _))
    · exact LExpr.le_trans (LExpr.le_join_right _ _) (LExpr.le_join_left _ _)

theorem Program.le_reqFail (pc : LExpr) (ts : List LTy) : p.le pc (p.reqFail pc ts) := by
  unfold Program.reqFail
  split
  · exact LExpr.le_join_left _ _
  · exact LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_left _ _)

theorem Program.Bound.fails_bot {pc : LExpr} {o : Bool} {g : HostFx} {f : Bool} (h : p.Bound pc o g f) :
    f = true → p.le pc .bot := fun hf => (h.2.2.2.2 hf).1

mutual
  /-- **Every typing's effects satisfy `Eff.DlOk`** (#524). -/
  theorem LRhsTy.dlOk :
      ∀ {E : List String} {Γ : LCtx} {pc : LExpr} {r : Rhs} {t : LTy} {ε : Eff},
        LRhsTy p E Γ pc r t ε → ε.DlOk p pc
    | _, _, _, _, _, _, .atom _ => Eff.dlOk_none
    | _, _, _, _, _, _, .prim _ _ => Eff.dlOk_none
    | _, _, _, _, _, _, .call _ _ _ _ _ _ _ _ hb _ => Eff.dlOk_ofCall hb.fails_bot
    | _, _, _, _, _, _, .invoke _ _ _ _ _ _ _ => Eff.dlOk_ofInvoke
    | _, _, _, _, _, _, .apply _ _ _ hb =>
      Eff.dlOk_ofFn fun hf => LExpr.le_trans (LExpr.le_join_left _ _) (hb.fails_bot hf)
    | _, _, _, _, _, _, .op _ _ _ _ => ⟨fun _ h => by simp at h, LExpr.bot_le _⟩
    | _, _, _, _, _, _, .read _ _ _ _ =>
      Eff.dlOk_leaf (LExpr.le_trans (LExpr.le_join_left _ _) (Program.le_fsFail _))
        (Program.fsFail_mono (LExpr.le_join_right _ _))
    | _, _, _, _, _, _, .write _ _ _ _ _ _ _ =>
      Eff.dlOk_leaf (LExpr.le_trans (LExpr.le_join_left _ _) (Program.le_fsFail _))
        (Program.fsFail_mono (LExpr.le_join_right _ _))
    | _, _, _, _, _, _, .req _ _ _ _ _ =>
      Eff.dlOk_leaf (Program.le_reqFail _ _) (Program.reqFail_mono _ (LExpr.bot_le _))
    | _, _, _, _, _, _, .construct _ _ _ => Eff.dlOk_none
    | _, _, _, _, _, _, .variant _ _ _ _ => Eff.dlOk_none
    | _, _, _, _, _, _, .field _ _ _ => Eff.dlOk_none
    | _, _, _, _, _, _, .list _ _ _ => Eff.dlOk_none
    | _, _, _, _, _, _, .none _ => Eff.dlOk_none
    | _, _, _, _, _, _, .some _ => Eff.dlOk_none
    | _, _, _, _, _, _, .first _ => Eff.dlOk_none
    | _, _, _, _, _, _, .mkOk _ _ => Eff.dlOk_none
    | _, _, _, _, _, _, .mkErr _ _ => Eff.dlOk_none
    | _, _, _, _, _, _, .lambda _ _ _ _ _ => Eff.dlOk_none
    | _, _, _, _, _, _, .funRef _ _ _ _ _ _ _ _ => Eff.dlOk_none
    | _, _, _, _, _, _, .declassify _ _ _ _ _ _ => Eff.dlOk_ofPolicy
    | _, _, _, _, _, _, .map _ _ _ hb =>
      Eff.dlOk_ofFn fun hf => LExpr.le_trans (LExpr.le_trans (LExpr.le_join_left _ _)
        (LExpr.le_join_left _ _)) (hb.fails_bot hf)
    | _, _, _, _, _, _, .filter _ _ _ hb =>
      Eff.dlOk_ofFn fun hf => LExpr.le_trans (LExpr.le_trans (LExpr.le_join_left _ _)
        (LExpr.le_join_left _ _)) (hb.fails_bot hf)
    | _, _, _, _, _, _, .firstWhere _ _ _ hb =>
      Eff.dlOk_ofFn fun hf => LExpr.le_trans (LExpr.le_trans (LExpr.le_join_left _ _)
        (LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_trans (LExpr.le_join_left _ _)
          (LExpr.le_join_left _ _)))) (hb.fails_bot hf)
    | _, _, _, _, _, _, .fold _ _ _ _ _ _ hb =>
      Eff.dlOk_ofFn fun hf => LExpr.le_trans (LExpr.le_trans (LExpr.le_join_left _ _)
        (LExpr.le_join_left _ _)) (hb.fails_bot hf)
    | _, _, _, _, _, _, .parse _ _ => Eff.dlOk_leaf (LExpr.le_join_left _ _) (LExpr.le_join_right _ _)
    | _, _, _, _, _, _, .checked _ _ _ _ => Eff.dlOk_leaf (LExpr.le_join_left _ _) (LExpr.le_join_right _ _)
    | _, _, _, _, _, _, .eq _ _ _ _ _ => Eff.dlOk_none
    | _, _, _, _, _, _, .toText _ _ => Eff.dlOk_none
    | _, _, _, _, _, _, .envGet _ _ _ => Eff.dlOk_none
    | _, _, _, _, _, _, .tuple _ => Eff.dlOk_none
    | _, _, _, _, _, _, .append _ _ _ => Eff.dlOk_none
    | _, _, _, _, _, _, .set _ _ _ _ _ => Eff.dlOk_none

  theorem LExprTy.dlOk :
      ∀ {E : List String} {Γ : LCtx} {pc : LExpr} {e : Expr} {t : LTy} {ε : Eff},
        LExprTy p E Γ pc e t ε → ε.DlOk p pc
    | _, _, _, _, _, _, .atom _ => Eff.dlOk_none
    | _, _, _, _, _, _, .letR hr _ hb _ =>
      Eff.DlOk.union (LRhsTy.dlOk hr) (LExprTy.dlOk hb) (LExpr.le_join_left _ _)
    | _, _, _, _, _, _, .letE hv _ hb _ =>
      Eff.DlOk.union (LExprTy.dlOk hv) (LExprTy.dlOk hb) (LExpr.le_join_left _ _)
    | _, _, _, _, _, _, .ite _ ht he _ _ _ =>
      (Eff.DlOk.decide (Eff.DlOk.union (LExprTy.dlOk ht) (LExprTy.dlOk he) (LExpr.le_refl _))
        (LExpr.le_join_right _ _)).mono (LExpr.le_join_left _ _)
    | _, _, _, _, _, _, .iteT _ ht he _ _ _ _ =>
      (Eff.DlOk.decide (Eff.DlOk.union (LExprTy.dlOk ht) (LExprTy.dlOk he) (LExpr.le_refl _))
        (LExpr.le_join_right _ _)).mono (LExpr.le_join_left _ _)
    | _, _, _, _, _, _, .caseOption _ hn hs _ _ _ =>
      (Eff.DlOk.decide (Eff.DlOk.union (LExprTy.dlOk hn) (LExprTy.dlOk hs) (LExpr.le_refl _))
        (LExpr.le_join_right _ _)).mono (LExpr.le_join_left _ _)
    | _, _, _, _, _, _, .caseOptionT _ hn hs _ _ _ _ =>
      (Eff.DlOk.decide (Eff.DlOk.union (LExprTy.dlOk hn) (LExprTy.dlOk hs) (LExpr.le_refl _))
        (LExpr.le_join_right _ _)).mono (LExpr.le_join_left _ _)
    | _, _, _, _, _, _, .caseResult _ ho he _ _ _ =>
      (Eff.DlOk.decide (Eff.DlOk.union (LExprTy.dlOk ho) (LExprTy.dlOk he) (LExpr.le_refl _))
        (LExpr.le_join_right _ _)).mono (LExpr.le_join_left _ _)
    | _, _, _, _, _, _, .caseResultT _ ho he _ _ _ _ =>
      (Eff.DlOk.decide (Eff.DlOk.union (LExprTy.dlOk ho) (LExprTy.dlOk he) (LExpr.le_refl _))
        (LExpr.le_join_right _ _)).mono (LExpr.le_join_left _ _)
    | _, _, _, _, _, _, .fail hle _ _ => Eff.dlOk_leaf hle (LExpr.bot_le _)
    | _, _, _, _, _, _, .raise _ _ _ _ hle _ _ => Eff.dlOk_leaf hle (LExpr.bot_le _)
    | _, _, _, _, _, _, .unreachable _ => Eff.dlOk_none
    | _, _, _, _, _, _, .caseSum _ _ _ _ ht he _ _ _ =>
      (Eff.DlOk.decide (Eff.DlOk.union (LExprTy.dlOk ht) (LExprTy.dlOk he) (LExpr.le_refl _))
        (LExpr.le_join_right _ _)).mono (LExpr.le_join_left _ _)
    | _, _, _, _, _, _, .caseSumT _ _ _ _ ht he _ _ _ _ =>
      (Eff.DlOk.decide (Eff.DlOk.union (LExprTy.dlOk ht) (LExprTy.dlOk he) (LExpr.le_refl _))
        (LExpr.le_join_right _ _)).mono (LExpr.le_join_left _ _)
    | _, _, _, _, _, _, .attempt _ _ => Eff.dlOk_attemptOf _
    | _, _, _, _, _, _, .attemptK _ _ _ => Eff.dlOk_attemptOf _
    | _, _, _, _, _, _, .handle hb _ hh _ _ _ => Eff.DlOk.handleOf (LExprTy.dlOk hb) (LExprTy.dlOk hh)
    | _, _, _, _, _, _, .handleK hb _ _ hh _ _ _ => Eff.DlOk.handleOf (LExprTy.dlOk hb) (LExprTy.dlOk hh)
    | _, _, _, _, _, _, .handleT hb _ hh _ _ _ _ => Eff.DlOk.handleOf (LExprTy.dlOk hb) (LExprTy.dlOk hh)
    | _, _, _, _, _, _, .handleKT hb _ _ hh _ _ _ _ => Eff.DlOk.handleOf (LExprTy.dlOk hb) (LExprTy.dlOk hh)
    | _, _, _, _, _, _, .letTuple _ _ _ hb => LExprTy.dlOk hb
    | _, _, _, _, _, _, .loopList _ _ _ _ _ _ hb _ _ _ =>
      (Eff.DlOk.decide (LExprTy.dlOk hb) (LExpr.le_join_right _ _)).mono (LExpr.le_join_left _ _)
    | _, _, _, _, _, _, .loopRange _ _ _ _ _ _ _ hb _ _ _ =>
      (Eff.DlOk.decide (LExprTy.dlOk hb) (LExpr.le_join_right _ _)).mono (LExpr.le_join_left _ _)
end

/-- **`dl ⊑ failLabel`** for every typing (#524). -/
theorem LExprTy.dl_le_failLabel {E : List String} {Γ : LCtx} {pc : LExpr} {e : Expr} {t : LTy}
    {ε : Eff} (h : LExprTy p E Γ pc e t ε) : p.le ε.dl ε.failLabel := (LExprTy.dlOk h).2

theorem LRhsTy.dl_le_failLabel {E : List String} {Γ : LCtx} {pc : LExpr} {r : Rhs} {t : LTy}
    {ε : Eff} (h : LRhsTy p E Γ pc r t ε) : p.le ε.dl ε.failLabel := (LRhsTy.dlOk h).2

end Una.F1
