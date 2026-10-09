/-
The labels of F1 (`spec/07 §2`, `§7` rule 8): a concrete label is a level and a set of
categories; a label of a generic body may also hold the symbolic labels of the function's type
parameters, `ℓ_T`, the label of a value of `T` as it was passed in, and `ℓ*_T`, every label of
the type argument of `T`.
-/
import UnaFormal.Label

namespace Una.F1

/-- A concrete label (`spec/07 §2`): a level and a set of categories, here a list. -/
structure Label where
  level : Level
  cats : List String

namespace Label

/-- `public` with no categories. -/
def bot : Label := ⟨.«public», []⟩

/-- `a ⊑ b`, componentwise. -/
def le (a b : Label) : Prop := a.level ≤ b.level ∧ ∀ c ∈ a.cats, c ∈ b.cats

/-- The join, componentwise. -/
def join (a b : Label) : Label := ⟨Level.join a.level b.level, a.cats ++ b.cats⟩

theorem le_refl (a : Label) : a.le a := ⟨Level.le_refl _, fun _ h => h⟩

theorem le_trans {a b c : Label} (h₁ : a.le b) (h₂ : b.le c) : a.le c :=
  ⟨Level.le_trans h₁.1 h₂.1, fun x hx => h₂.2 x (h₁.2 x hx)⟩

theorem le_join_left (a b : Label) : a.le (a.join b) :=
  ⟨Level.le_join_left _ _, fun _ h => List.mem_append_left _ h⟩

theorem le_join_right (a b : Label) : b.le (a.join b) :=
  ⟨Level.le_join_right _ _, fun _ h => List.mem_append_right _ h⟩

theorem join_le {a b c : Label} (ha : a.le c) (hb : b.le c) : (a.join b).le c := by
  refine ⟨?_, ?_⟩
  · show (Level.join a.level b.level).rank ≤ c.level.rank
    unfold Level.join
    split
    · exact hb.1
    · exact ha.1
  · intro x hx
    rcases List.mem_append.mp hx with h | h
    · exact ha.2 x h
    · exact hb.2 x h

end Label

/-- A label as the checker writes it in a generic body: a concrete part joined with the
symbolic labels `ℓ_T` (`plain`) and `ℓ*_T` (`star`) of type parameters, named. Outside a
generic body both lists are empty. -/
structure LExpr where
  base : Label
  plain : List String
  star : List String

namespace LExpr

/-- A concrete label. -/
def ofLabel (l : Label) : LExpr := ⟨l, [], []⟩

/-- `⊥`. -/
def bot : LExpr := ofLabel Label.bot

/-- The join: the join of the concrete parts and the union of the symbolic ones. -/
def join (a b : LExpr) : LExpr := ⟨a.base.join b.base, a.plain ++ b.plain, a.star ++ b.star⟩

/-- What a deep join collects: every `ℓ_T` becomes `ℓ*_T` (`spec/07 §7` rule 8, "every label
that a deep join collects gives `ℓ*_T`"). -/
def promote (e : LExpr) : LExpr := ⟨e.base, [], e.plain ++ e.star⟩

/-- No symbolic label. Effects and failures need a program counter without one (`spec/07 §7`
rule 8, "no effects or failures under a symbolic program counter"). -/
def SymFree (e : LExpr) : Prop := e.plain = [] ∧ e.star = []

/-- The static order of `spec/07 §7` rule 8, relative to the program's `⊤`: a symbolic label is
below a declared label only if the declared label is `⊤` or holds the same symbolic label, and
`ℓ_T ⊑ ℓ*_T`. -/
def le (top : Label) (a b : LExpr) : Prop :=
  top.le b.base ∨
    (a.base.le b.base ∧ (∀ x ∈ a.plain, x ∈ b.plain ∨ x ∈ b.star) ∧ ∀ x ∈ a.star, x ∈ b.star)

/-- The join of the labels a substitution gives the symbolic labels it covers. -/
def joinAll : List LExpr → LExpr
  | [] => bot
  | e :: es => e.join (joinAll es)

/-- Instantiation: each symbolic label `ℓ_T` or `ℓ*_T` whose `T` the substitution `σ` covers
becomes `σ(T)`, which a call gives as `ℓ*` of the type argument (`spec/10 §5`, generic
instantiation); the others stay. -/
def inst (σ : List (String × LExpr)) (e : LExpr) : LExpr :=
  let covered := fun x => (List.lookup x σ).isSome
  let hit := (e.plain ++ e.star).filterMap (fun x => List.lookup x σ)
  (joinAll hit).join
    ⟨e.base, e.plain.filter (fun x => !covered x), e.star.filter (fun x => !covered x)⟩

/-- `e` without `ℓ_x`: at a position of type `x`, `ℓ_x` is the type argument's own labels,
which substitution puts there (ADR 0067), so it adds nothing. -/
def dropPlain (x : String) (e : LExpr) : LExpr := ⟨e.base, e.plain.filter (· != x), e.star⟩

/-- The semantic reading of a label under an instantiation (the M1 spike): `ρ T` is `ℓ*` of the
type argument of `T`, and both `ℓ_T` off a position of type `T` and `ℓ*_T` read as it. -/
def interp (ρ : String → Label) (e : LExpr) : Label :=
  (e.plain ++ e.star).foldr (fun x acc => (ρ x).join acc) e.base

end LExpr

end Una.F1
