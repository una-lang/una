/-
M3's first item (the owner's condition 3, 2026-10-09): instantiation commutes with the semantic
reading of labels. Reading an instantiated label under `ρ` is reading the label itself under
`ρ` extended by the instantiation, each covered `T` read as `ℓ*` of its type argument. So the
checker's instantiation at a call (`LExpr.inst`, `LTy.lsubst`) is the reading's, and a proof
about a generic body under the reading holds at every call.

Labels hold categories as lists, so two readings are compared up to the order (`Label.Equiv`).
-/
import UnaFormal.F1.Spike

namespace Una.F1

namespace Label

/-- Two labels below each other: the same level and the same set of categories. -/
def Equiv (a b : Label) : Prop := a.le b ∧ b.le a

theorem bot_le (c : Label) : Label.bot.le c := ⟨Level.public_le _, fun _ h => nomatch h⟩

theorem join_le_iff {a b c : Label} : (a.join b).le c ↔ a.le c ∧ b.le c :=
  ⟨fun h => ⟨le_of_join_left h, le_of_join_right h⟩, fun ⟨ha, hb⟩ => join_le ha hb⟩

/-- Two labels with the same upper bounds are equivalent. -/
theorem equiv_of_le_iff {a b : Label} (h : ∀ c, a.le c ↔ b.le c) : a.Equiv b :=
  ⟨(h b).mpr (le_refl b), (h a).mp (le_refl a)⟩

end Label

namespace LExpr

/-- The reading of a label is its least upper bound: below `c` exactly when its concrete part and
the reading of each of its symbolic labels are. -/
theorem interp_le_iff {ρ : String → Label} {e : LExpr} {c : Label} :
    (e.interp ρ).le c ↔ e.base.le c ∧ ∀ x ∈ e.plain ++ e.star, (ρ x).le c :=
  ⟨fun h => ⟨Label.le_trans (interp_base_le ρ e) h,
      fun _ hx => Label.le_trans (interp_sym_le ρ e hx) h⟩,
    fun ⟨hb, hs⟩ => interp_le_of hb hs⟩

theorem interp_join_le_iff {ρ : String → Label} {a b : LExpr} {c : Label} :
    ((a.join b).interp ρ).le c ↔ (a.interp ρ).le c ∧ (b.interp ρ).le c := by
  simp only [interp_le_iff, join, Label.join_le_iff, List.mem_append]
  constructor
  · rintro ⟨⟨ha, hb⟩, hs⟩
    exact ⟨⟨ha, fun x hx => hs x (by rcases hx with hx | hx <;> simp [hx])⟩,
      ⟨hb, fun x hx => hs x (by rcases hx with hx | hx <;> simp [hx])⟩⟩
  · rintro ⟨⟨ha, hsa⟩, ⟨hb, hsb⟩⟩
    refine ⟨⟨ha, hb⟩, fun x hx => ?_⟩
    rcases hx with (hx | hx) | (hx | hx)
    · exact hsa x (.inl hx)
    · exact hsb x (.inl hx)
    · exact hsa x (.inr hx)
    · exact hsb x (.inr hx)

theorem interp_joinAll_le_iff {ρ : String → Label} {c : Label} :
    ∀ {es : List LExpr}, ((joinAll es).interp ρ).le c ↔ ∀ e ∈ es, (e.interp ρ).le c
  | [] => by
    simp only [joinAll, List.not_mem_nil, false_implies, implies_true, iff_true]
    exact interp_le_of (Label.bot_le c) (fun _ h => by simp [bot, ofLabel] at h)
  | e :: es => by
    simp only [joinAll, interp_join_le_iff, interp_joinAll_le_iff, List.mem_cons, forall_eq_or_imp]

/-- `ρ` extended by an instantiation: each `T` it covers reads as the reading of its label. -/
def extend (ρ : String → Label) (σ : List (String × LExpr)) (x : String) : Label :=
  match List.lookup x σ with
  | some e => e.interp ρ
  | none => ρ x

theorem inst_le_iff {ρ : String → Label} {σ : List (String × LExpr)} {e : LExpr} {c : Label} :
    ((e.inst σ).interp ρ).le c ↔ (e.interp (extend ρ σ)).le c := by
  unfold inst
  rw [interp_join_le_iff, interp_joinAll_le_iff, interp_le_iff, interp_le_iff]
  simp only [List.mem_filterMap, List.mem_append, List.mem_filter, Bool.not_eq_true',
    Option.isSome_eq_false_iff, Option.isNone_iff_eq_none]
  constructor
  · rintro ⟨hhit, hb, hrest⟩
    refine ⟨hb, fun x hx => ?_⟩
    unfold extend
    cases hl : List.lookup x σ with
    | some h => exact hhit h ⟨x, hx, hl⟩
    | none =>
      apply hrest x
      rcases hx with hx | hx
      · exact .inl ⟨hx, by simp [hl]⟩
      · exact .inr ⟨hx, by simp [hl]⟩
  · rintro ⟨hb, hs⟩
    refine ⟨fun h ⟨x, hx, hl⟩ => ?_, hb, fun x hx => ?_⟩
    · have := hs x hx
      simp only [extend, hl] at this
      exact this
    · have hx' : x ∈ e.plain ∨ x ∈ e.star := by
        rcases hx with ⟨hx, _⟩ | ⟨hx, _⟩
        · exact .inl hx
        · exact .inr hx
      have hnone : List.lookup x σ = none := by
        rcases hx with ⟨_, h⟩ | ⟨_, h⟩ <;> simpa using h
      have := hs x hx'
      simp only [extend, hnone] at this
      exact this

/-- **Instantiation commutes with the reading.** Reading `e` instantiated by `σ` under `ρ` is
reading `e` under `ρ` extended by `σ`. -/
theorem interp_inst (ρ : String → Label) (σ : List (String × LExpr)) (e : LExpr) :
    ((e.inst σ).interp ρ).Equiv (e.interp (extend ρ σ)) :=
  Label.equiv_of_le_iff (fun _ => inst_le_iff)

end LExpr



/-- At a position that is not a type parameter, labeled instantiation instantiates the label. -/
theorem LTy.top_lsubst (rd : String → Label) (σ : List (String × LTy)) :
    ∀ t : LTy, (∀ a l, t ≠ .var a l) →
      (t.lsubst rd σ).top = t.top.inst (σ.map (fun q => (q.1, q.2.deep rd)))
  | .int _, _ | .bool _, _ | .text _, _ | .unit _, _ | .list _ _, _ | .option _ _, _
  | .result _ _, _ | .record _ _, _ | .fn _ _ _ _ _ _, _ => by simp [LTy.lsubst, LTy.top]
  | .var a l, h => absurd rfl (h a l)

/-- At a position of type `T` that `σ` covers, labeled instantiation puts `T`'s type argument,
with its own labels in place, raised by the position's other labels instantiated. -/
theorem LTy.var_lsubst (rd : String → Label) (σ : List (String × LTy)) {a : String} {u : LTy}
    (l : LExpr) (h : List.lookup a σ = some u) :
    (LTy.var a l).lsubst rd σ = u.raise ((l.dropPlain a).inst (σ.map (fun q => (q.1, q.2.deep rd)))) := by
  simp [LTy.lsubst, h]

/-- So the reading of an instantiated label takes each covered `T` as `ℓ*` of its type
argument: the extension of `ρ` by the instantiation of a call reads `T` as the reading of
`deep (σ T)`. -/
theorem extend_deep (rd : String → Label) (ρ : String → Label) (σ : List (String × LTy))
    {a : String} {u : LTy} (h : List.lookup a σ = some u) :
    LExpr.extend ρ (σ.map (fun q => (q.1, q.2.deep rd))) a = (u.deep rd).interp ρ := by
  simp [LExpr.extend, lookup_map, h]

end Una.F1
