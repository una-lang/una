/-
M4, generics: rule 8's static order of labels is stable under instantiation (`LExpr.le_inst`):
when the checker finds `a ⊑ b` in a generic body, `a ⊑ b` holds of their instances at every call;
so are flows (`Flow.lsubst`), bounds and lookups.
-/
import UnaFormal.F1H.Order
import UnaFormal.F1H.Inst
import UnaFormal.F1.Subst

namespace Una.F1H

open Una.F1 (Label LExpr)



/-- The bound of a function's effects holds of the instance of the program counter. -/
theorem Program.Bound.inst {p : Program} {pc : LExpr} {o : Bool} {g : HostFx} {f : Bool}
    (σ : List (String × LExpr)) (h : p.Bound pc o g f) : p.Bound (pc.inst σ) o g f := by
  have lift : ∀ {l : Label}, p.le pc (.ofLabel l) → p.le (pc.inst σ) (.ofLabel l) :=
    fun hl => LExpr.le_of_le_inst_ofLabel (LExpr.le_inst σ hl)
  obtain ⟨ho, hr, hw, hq, hf⟩ := h
  refine ⟨fun h' => ⟨lift (ho h').1, LExpr.symFree_inst σ (ho h').2⟩,
    fun h' => ⟨lift (hr h').1, LExpr.symFree_inst σ (hr h').2⟩,
    fun h' => ⟨lift (hw h').1, fun hold => lift ((hw h').2.1 hold), LExpr.symFree_inst σ (hw h').2.2⟩,
    fun h' => ⟨lift (hq h').1, LExpr.symFree_inst σ (hq h').2⟩,
    fun h' => ⟨LExpr.le_bot_of_le_inst_bot (LExpr.le_inst σ (hf h').1), LExpr.symFree_inst σ (hf h').2⟩⟩

/-- Raising one type by two ordered labels gives two types that flow. -/
theorem Flow.raise_mono {top : Label} {e₁ e₂ : LExpr} (h : LExpr.le top e₁ e₂) :
    ∀ u : LTy, Flow top (u.raise e₁) (u.raise e₂)
  | .int l => .int (LExpr.join_mono (LExpr.le_refl l) h)
  | .bool l => .bool (LExpr.join_mono (LExpr.le_refl l) h)
  | .text l => .text (LExpr.join_mono (LExpr.le_refl l) h)
  | .unit _ => .unit
  | .list t l => .list (Flow.refl t) (LExpr.join_mono (LExpr.le_refl l) h)
  | .option t l => .option (Flow.refl t) (LExpr.join_mono (LExpr.le_refl l) h)
  | .result t l => .result (Flow.refl t) (LExpr.join_mono (LExpr.le_refl l) h)
  | .record _ l => .record (LExpr.join_mono (LExpr.le_refl l) h)
  | .fn ps r _ _ _ l => .fn (Flows.refl ps) (Flow.refl r) id (HostFx.Le.refl _) id (LExpr.join_mono (LExpr.le_refl l) h)
  | .var _ l => .var (LExpr.join_mono (LExpr.le_refl l) h)

mutual
  /-- **Flows are stable under labeled instantiation.** A flow the checker finds between two
  types of a generic body holds between their instances at every call. -/
  theorem Flow.lsubst {top : Label} (rd : String → Label) (σ : List (String × LTy)) :
      ∀ {t₁ t₂ : LTy}, Flow top t₁ t₂ → Flow top (t₁.lsubst rd σ) (t₂.lsubst rd σ)
    | _, _, .int h => .int (LExpr.le_inst _ h)
    | _, _, .bool h => .bool (LExpr.le_inst _ h)
    | _, _, .text h => .text (LExpr.le_inst _ h)
    | _, _, .unit => .unit
    | _, _, .list f h => .list (Flow.lsubst rd σ f) (LExpr.le_inst _ h)
    | _, _, .option f h => .option (Flow.lsubst rd σ f) (LExpr.le_inst _ h)
    | _, _, .result f h => .result (Flow.lsubst rd σ f) (LExpr.le_inst _ h)
    | _, _, .record h => .record (LExpr.le_inst _ h)
    | _, _, .fn fps fr fo fg ff h =>
      .fn (Flows.lsubst rd σ fps) (Flow.lsubst rd σ fr) fo fg ff (LExpr.le_inst _ h)
    | _, _, .var (a := a) h => by
      simp only [LTy.lsubst]
      cases List.lookup a σ with
      | some u => exact Flow.raise_mono (LExpr.le_inst _ (LExpr.le_dropPlain a h)) u
      | none => exact .var (LExpr.le_inst _ h)

  theorem Flows.lsubst {top : Label} (rd : String → Label) (σ : List (String × LTy)) :
      ∀ {ts₁ ts₂ : List LTy}, Flows top ts₁ ts₂ →
        Flows top (LTy.lsubsts rd σ ts₁) (LTy.lsubsts rd σ ts₂)
    | _, _, .nil => .nil
    | _, _, .cons f fs => .cons (Flow.lsubst rd σ f) (Flows.lsubst rd σ fs)
end

/-- A labeled context instantiated. -/
def LCtx.lsub (rd : String → Label) (σ : List (String × LTy)) (Γ : LCtx) : LCtx :=
  Γ.map (fun q => (q.1, q.2.lsubst rd σ))

theorem lctxLookup_lsub (rd : String → Label) (σ : List (String × LTy)) (x : String) :
    ∀ Γ : LCtx, lctxLookup (Γ.lsub rd σ) x = (lctxLookup Γ x).map (LTy.lsubst rd σ)
  | [] => rfl
  | (y, t) :: Γ => by
    simp only [LCtx.lsub, List.map_cons, lctxLookup]
    split
    · rfl
    · exact lctxLookup_lsub rd σ x Γ

/-- A non-vacuity instance: `ℓ_T ⊑ ℓ*_T` holds, and so does its instance at `T := secret`. -/
example : LExpr.le (top := ⟨.restricted, []⟩) (LExpr.inst [("T", .ofLabel ⟨.secret, []⟩)] ⟨Label.bot, ["T"], []⟩)
    (LExpr.inst [("T", .ofLabel ⟨.secret, []⟩)] ⟨Label.bot, [], ["T"]⟩) :=
  LExpr.le_inst _ (.inr ⟨Label.le_refl _, fun _ h => .inr (by simpa using h), fun _ h => nomatch h⟩)

end Una.F1H
