/-
Labels (`spec/07 §2`): a level and a set of categories, ordered componentwise. M0 states the
order of levels and proves it is a partial order with `public` at the bottom, as the first
theorems the `formal` gate checks for `sorry`.
-/
namespace Una

/-- The levels of `spec/07 §2`, in order. -/
inductive Level where
  | public
  | internal
  | confidential
  | secret
  | restricted
  deriving DecidableEq, Repr

namespace Level

/-- The rank of a level in the order of `spec/07 §2`. -/
def rank : Level → Nat
  | .public => 0
  | .internal => 1
  | .confidential => 2
  | .secret => 3
  | .restricted => 4

/-- `a ⊑ b`: `a` is at most `b`. -/
def le (a b : Level) : Prop := a.rank ≤ b.rank

instance : LE Level := ⟨le⟩

instance (a b : Level) : Decidable (a ≤ b) := Nat.decLe a.rank b.rank

theorem rank_injective {a b : Level} (h : a.rank = b.rank) : a = b := by
  cases a <;> cases b <;> simp_all [rank]

/-- The order of levels is reflexive. -/
theorem le_refl (a : Level) : a ≤ a := Nat.le_refl a.rank

/-- The order of levels is transitive. -/
theorem le_trans {a b c : Level} (h₁ : a ≤ b) (h₂ : b ≤ c) : a ≤ c := Nat.le_trans h₁ h₂

/-- The order of levels is antisymmetric. -/
theorem le_antisymm {a b : Level} (h₁ : a ≤ b) (h₂ : b ≤ a) : a = b :=
  rank_injective (Nat.le_antisymm h₁ h₂)

/-- `public` is the bottom level. -/
theorem public_le (a : Level) : Level.public ≤ a := Nat.zero_le a.rank

/-- The join of two levels, the higher one. -/
def join (a b : Level) : Level := if a.rank ≤ b.rank then b else a

/-- The join is an upper bound of both levels. -/
theorem le_join_left (a b : Level) : a ≤ join a b := by
  unfold join
  split
  · assumption
  · exact le_refl a

theorem le_join_right (a b : Level) : b ≤ join a b := by
  unfold join
  split
  · exact le_refl b
  · show b.rank ≤ a.rank
    omega

end Level

end Una
