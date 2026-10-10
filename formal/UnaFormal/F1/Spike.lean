/-
The M1 spike (`docs/plan/formal-track.md` §8): does `ℓ*_T` have a clean semantic reading in a
two-run relation? This file states the reading and proves the facts M3 and M4 build on.

- **The reading.** Under an instantiation `σ`, `ℓ*_T` means `ℓ*(σ(T))`, the join of every label
  of `T`'s type argument; `ℓ_T` at a position of type `T` means the type argument's own labels,
  in place, and anywhere else the model reads it as `ℓ*(σ(T))`, which is never smaller.
  `LTy.lsubst` is this reading, and `LExpr.interp` is it for one label.
- **The static order is sound for it** (`LExpr.le_sound`): when the checker finds `a ⊑ b`
  between labels with symbolic parts, `a`'s reading is below `b`'s for every instantiation.
- **The relation.** For an observer at `L`, two values are related at a labeled type when the
  type's top label is not visible to `L`, or they have the same shape and their parts are
  related at the parts' types (`Rel`, first-order here; function types are M3's).
- **Why `ℓ*_T` must be the deep join.** Values related at `σ(T)` are equal when every label of
  `σ(T)` is visible (`Rel.eq_of_deep`), so whatever a comparison, an equality or a field
  computes from two values of `T` agrees in both runs when `ℓ*(σ(T))` is visible: that is the
  label rule 8 gives such a result (`derived_agrees`). The top label alone is not enough
  (`top_label_not_enough`): a public list of secret integers has a visible top label, and two
  related values of it differ.
-/
import UnaFormal.F1.LTyping

namespace Una.F1

namespace Label

theorem le_of_join_left {a b c : Label} (h : (a.join b).le c) : a.le c :=
  le_trans (le_join_left a b) h

theorem le_of_join_right {a b c : Label} (h : (a.join b).le c) : b.le c :=
  le_trans (le_join_right a b) h

end Label

namespace LExpr

theorem foldr_base_le (ρ : String → Label) (base : Label) :
    ∀ xs : List String, base.le (xs.foldr (fun x acc => (ρ x).join acc) base)
  | [] => Label.le_refl _
  | _ :: xs => Label.le_trans (foldr_base_le ρ base xs) (Label.le_join_right _ _)

theorem foldr_sym_le (ρ : String → Label) (base : Label) {x : String} :
    ∀ xs : List String, x ∈ xs → (ρ x).le (xs.foldr (fun x acc => (ρ x).join acc) base)
  | [], hx => by simp at hx
  | y :: ys, hx => by
    rcases List.mem_cons.mp hx with rfl | hx
    · exact Label.le_join_left _ _
    · exact Label.le_trans (foldr_sym_le ρ base ys hx) (Label.le_join_right _ _)

theorem foldr_le_of (ρ : String → Label) {base c : Label} (hb : base.le c) :
    ∀ xs : List String, (∀ x ∈ xs, (ρ x).le c) →
      (xs.foldr (fun x acc => (ρ x).join acc) base).le c
  | [], _ => hb
  | y :: ys, hs =>
    Label.join_le (hs y List.mem_cons_self)
      (foldr_le_of ρ hb ys (fun x hx => hs x (List.mem_cons_of_mem _ hx)))

theorem interp_base_le (ρ : String → Label) (e : LExpr) : e.base.le (e.interp ρ) :=
  foldr_base_le ρ e.base _

theorem interp_sym_le (ρ : String → Label) (e : LExpr) {x : String} (hx : x ∈ e.plain ++ e.star) :
    (ρ x).le (e.interp ρ) :=
  foldr_sym_le ρ e.base _ hx

theorem interp_le_of {ρ : String → Label} {e : LExpr} {c : Label} (hb : e.base.le c)
    (hs : ∀ x ∈ e.plain ++ e.star, (ρ x).le c) : (e.interp ρ).le c :=
  foldr_le_of ρ hb _ hs

/-- **The static order of rule 8 is sound for the reading.** When the checker finds `a ⊑ b`,
then for every instantiation whose labels are below `⊤`, `a`'s reading is below `b`'s. -/
theorem le_sound {top : Label} {ρ : String → Label} (hρ : ∀ x, (ρ x).le top) {a b : LExpr}
    (ha : a.base.le top) (h : LExpr.le top a b) : (a.interp ρ).le (b.interp ρ) := by
  rcases h with htop | ⟨hbase, hplain, hstar⟩
  · exact Label.le_trans (interp_le_of ha (fun x _ => hρ x))
      (Label.le_trans htop (interp_base_le ρ b))
  · apply interp_le_of (Label.le_trans hbase (interp_base_le ρ b))
    intro x hx
    apply interp_sym_le
    rcases List.mem_append.mp hx with hx | hx
    · rcases hplain x hx with h | h
      · exact List.mem_append_left _ h
      · exact List.mem_append_right _ h
    · exact List.mem_append_right _ (hstar x hx)

end LExpr

/-! ### The static order is sound for every instantiation -/



/-! ### The two-run relation, first-order -/

/-- A type with no function type and no type parameter: the instances of type parameters whose
values M1's relation covers. -/
def LTy.FirstOrder : LTy → Prop
  | .int _ | .bool _ | .text _ | .unit _ | .record _ _ => True
  | .list t _ | .option t _ | .result t _ => t.FirstOrder
  | .fn _ _ _ _ _ _ | .var _ _ | .tuple _ _ => False

/-- Two lists related element by element. -/
inductive Forall2 (R : Val → Val → Prop) : List Val → List Val → Prop where
  | nil : Forall2 R [] []
  | cons {a b as bs} : R a b → Forall2 R as bs → Forall2 R (a :: as) (b :: bs)

/-- The values two runs give at a closed first-order labeled type, related for an observer at
`L`, with `rd` the labels records declare: when the top label is not visible, any two values;
otherwise the same shape with related parts. Both values at a `Unit` position are `unit` in a
typed run. A record is compared whole, by every label it
declares (M3 relates it field by field). -/
def Rel (L : Label) (rd : String → Label) : LTy → Val → Val → Prop
  | .int l, v₁, v₂ | .bool l, v₁, v₂ | .text l, v₁, v₂ => l.base.le L → v₁ = v₂
  | .unit _, v₁, v₂ => v₁ = v₂
  | .record r l, v₁, v₂ => (l.base.join (rd r)).le L → v₁ = v₂
  | .list t l, v₁, v₂ =>
    l.base.le L → ∃ xs ys, v₁ = .list xs ∧ v₂ = .list ys ∧ Forall2 (Rel L rd t) xs ys
  | .option t l, v₁, v₂ =>
    l.base.le L → (v₁ = .none ∧ v₂ = .none) ∨ ∃ a b, v₁ = .some a ∧ v₂ = .some b ∧ Rel L rd t a b
  | .result t l, v₁, v₂ =>
    l.base.le L → (∃ c, v₁ = .err c ∧ v₂ = .err c) ∨ ∃ a b, v₁ = .ok a ∧ v₂ = .ok b ∧ Rel L rd t a b
  | .fn _ _ _ _ _ _, _, _ | .var _ _, _, _ | .tuple _ _, _, _ => True

theorem forall₂_eq {xs ys : List Val} {R : Val → Val → Prop} (hR : ∀ a b, R a b → a = b) :
    Forall2 R xs ys → xs = ys := by
  intro h
  induction h with
  | nil => rfl
  | cons hab _ ih => rw [hR _ _ hab, ih]

/-- **Related values are equal when every label of their type is visible.** -/
theorem Rel.eq_of_deep {L : Label} {rd : String → Label} :
    ∀ (t : LTy), t.FirstOrder → (t.deep rd).base.le L → ∀ (v₁ v₂ : Val), Rel L rd t v₁ v₂ → v₁ = v₂
  | .int l, _, hd, v₁, v₂, h => by
    simp only [Rel] at h
    exact h hd
  | .bool l, _, hd, v₁, v₂, h => by
    simp only [Rel] at h
    exact h hd
  | .text l, _, hd, v₁, v₂, h => by
    simp only [Rel] at h
    exact h hd
  | .unit _, _, _, v₁, v₂, h => by
    simp only [Rel] at h
    exact h
  | .record r l, _, hd, v₁, v₂, h => by
    simp only [Rel] at h
    exact h hd
  | .list t l, hfo, hd, v₁, v₂, h => by
    simp only [Rel] at h
    simp only [LTy.deep, LExpr.join] at hd
    obtain ⟨xs, ys, h1, h2, h3⟩ := h (Label.le_of_join_left hd)
    subst h1 h2
    have ih := Rel.eq_of_deep t hfo (Label.le_of_join_right hd)
    rw [forall₂_eq ih h3]
  | .option t l, hfo, hd, v₁, v₂, h => by
    simp only [Rel] at h
    simp only [LTy.deep, LExpr.join] at hd
    rcases h (Label.le_of_join_left hd) with ⟨h1, h2⟩ | ⟨a, b, h1, h2, hab⟩
    · rw [h1, h2]
    · rw [h1, h2, Rel.eq_of_deep t hfo (Label.le_of_join_right hd) a b hab]
  | .result t l, hfo, hd, v₁, v₂, h => by
    simp only [Rel] at h
    simp only [LTy.deep, LExpr.join] at hd
    rcases h (Label.le_of_join_left hd) with ⟨c, h1, h2⟩ | ⟨a, b, h1, h2, hab⟩
    · rw [h1, h2]
    · rw [h1, h2, Rel.eq_of_deep t hfo (Label.le_of_join_right hd) a b hab]

/-- **A result derived from values of `T` agrees in both runs when `ℓ*(σ(T))` is visible**,
whatever the operation: the reading of rule 8's label for a comparison, an equality, a field
or any other operation over `T` values. -/
theorem derived_agrees {L : Label} {rd : String → Label} {t : LTy} (hfo : t.FirstOrder)
    (hd : (t.deep rd).base.le L) (op : Val → Val → Val) {x₁ x₂ y₁ y₂ : Val}
    (hx : Rel L rd t x₁ x₂) (hy : Rel L rd t y₁ y₂) : op x₁ y₁ = op x₂ y₂ := by
  rw [Rel.eq_of_deep t hfo hd _ _ hx, Rel.eq_of_deep t hfo hd _ _ hy]

/-- A secret label, for the example below. -/
def secretLabel : Label := ⟨.secret, []⟩

/-- **The top label is not enough.** At `List[secret Int]` with a public top label, the top label
is visible to a public observer, and two related values differ: an equality of two values of
`T` instantiated so would reveal a secret if it were labeled `ℓ_T` read as the top label. So
rule 8 labels it `ℓ*_T`, the deep join. -/
theorem top_label_not_enough :
    let t := LTy.list (.int (.ofLabel secretLabel)) .bot
    t.top.base.le Label.bot ∧ Rel Label.bot (fun _ => Label.bot) t (.list [.int 1]) (.list [.int 2]) ∧
      (Val.list [.int 1] ≠ Val.list [.int 2]) := by
  refine ⟨⟨Nat.le_refl _, fun _ h => h⟩, ?_, ?_⟩
  · intro _
    refine ⟨[.int 1], [.int 2], rfl, rfl, .cons ?_ .nil⟩
    intro h
    exact absurd h.1 (by decide)
  · intro h
    simp at h

end Una.F1
