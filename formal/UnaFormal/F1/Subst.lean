/-
M4, generics: rule 8's static order of labels is stable under instantiation (`LExpr.le_inst`):
when the checker finds `a ⊑ b` in a generic body, `a ⊑ b` holds of their instances at every call;
so are flows (`Flow.lsubst`), bounds and lookups.
-/
import UnaFormal.F1.Order
import UnaFormal.F1.Inst

namespace Una.F1

namespace LExpr

variable {top : Label}

theorem le_joinAll {e : LExpr} : ∀ {es : List LExpr}, e ∈ es → LExpr.le top e (joinAll es)
  | _ :: _, .head _ => le_join_left _ _
  | _ :: _, .tail _ h => le_trans (le_joinAll h) (le_join_right _ _)

/-- **Rule 8's order is stable under instantiation.** -/
theorem le_inst (σ : List (String × LExpr)) {a b : LExpr} (h : LExpr.le top a b) :
    LExpr.le top (a.inst σ) (b.inst σ) := by
  -- The instance of `b` is above `b`'s concrete part and each label `σ` gives its symbols.
  have hbase : b.base.le (b.inst σ).base := Label.le_join_right _ _
  have hhit : ∀ x ∈ b.plain ++ b.star, ∀ l, List.lookup x σ = some l → LExpr.le top l (b.inst σ) :=
    fun x hx l hl => le_trans (le_joinAll (List.mem_filterMap.mpr ⟨x, hx, hl⟩)) (le_join_left _ _)
  rcases h with htop | ⟨hb, hp, hs⟩
  · exact .inl (Label.le_trans htop hbase)
  · unfold inst
    refine join_le ?_ ?_
    · -- what `σ` gives `a`'s symbols, `b` gets too
      suffices ∀ es : List LExpr, (∀ e ∈ es, LExpr.le top e (b.inst σ)) →
          LExpr.le top (joinAll es) (b.inst σ) by
        apply this
        intro e he
        obtain ⟨x, hx, hl⟩ := List.mem_filterMap.mp he
        apply hhit x _ e hl
        rcases List.mem_append.mp hx with hx | hx
        · rcases hp x hx with h | h
          · exact List.mem_append_left _ h
          · exact List.mem_append_right _ h
        · exact List.mem_append_right _ (hs x hx)
      intro es hes
      induction es with
      | nil => exact bot_le _
      | cons e es ih =>
        exact join_le (hes e List.mem_cons_self) (ih (fun e' h => hes e' (List.mem_cons_of_mem _ h)))
    · -- the symbols `σ` leaves are still `b`'s
      refine .inr ⟨Label.le_trans hb hbase, fun x hx => ?_, fun x hx => ?_⟩
      · obtain ⟨hx, hnc⟩ := List.mem_filter.mp hx
        rcases hp x hx with h | h
        · exact .inl (List.mem_append_right _ (List.mem_filter.mpr ⟨h, hnc⟩))
        · exact .inr (List.mem_append_right _ (List.mem_filter.mpr ⟨h, hnc⟩))
      · obtain ⟨hx, hnc⟩ := List.mem_filter.mp hx
        exact List.mem_append_right _ (List.mem_filter.mpr ⟨hs x hx, hnc⟩)

end LExpr

namespace LExpr

variable {top : Label}

theorem le_dropPlain (x : String) {a b : LExpr} (h : LExpr.le top a b) :
    LExpr.le top (a.dropPlain x) (b.dropPlain x) := by
  rcases h with htop | ⟨hb, hp, hs⟩
  · exact .inl htop
  · refine .inr ⟨hb, fun y hy => ?_, hs⟩
    obtain ⟨hy, hne⟩ := List.mem_filter.mp hy
    rcases hp y hy with h | h
    · exact .inl (List.mem_filter.mpr ⟨h, hne⟩)
    · exact .inr h

/-- A label without symbols is its own instance, joined with `⊥`. -/
theorem inst_symFree (σ : List (String × LExpr)) {e : LExpr} (hp : e.plain = [])
    (hs : e.star = []) : e.inst σ = LExpr.bot.join e := by
  obtain ⟨base, plain, star⟩ := e
  simp only at hp hs
  subst hp hs
  rfl

theorem inst_ofLabel (σ : List (String × LExpr)) (l : Label) :
    (LExpr.ofLabel l).inst σ = LExpr.bot.join (.ofLabel l) := inst_symFree σ rfl rfl

theorem le_of_le_inst_ofLabel {σ : List (String × LExpr)} {x : LExpr} {l : Label}
    (h : LExpr.le top x ((LExpr.ofLabel l).inst σ)) : LExpr.le top x (.ofLabel l) := by
  rw [inst_ofLabel] at h
  exact le_trans h (join_le (bot_le _) (le_refl _))

theorem le_bot_of_le_inst_bot {σ : List (String × LExpr)} {x : LExpr}
    (h : LExpr.le top x (LExpr.bot.inst σ)) : LExpr.le top x .bot :=
  le_of_le_inst_ofLabel h

theorem symFree_inst (σ : List (String × LExpr)) {e : LExpr} (h : e.SymFree) :
    (e.inst σ).SymFree := by
  rw [inst_symFree σ h.1 h.2]
  simp [SymFree, join, bot, ofLabel, h.1, h.2]

end LExpr



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

end Una.F1
