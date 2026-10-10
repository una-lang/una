/-
Operations on types: the structure of a labeled type (`erase`), substitution of type arguments
for type parameters, structural (`Ty.subst`) and labeled (`LTy.lsubst`, `spec/10 §5`, generic
instantiation), and the labels of a labeled type: its top label, and `ℓ*`, the join of every
label (`deep`).
-/
import UnaFormal.F1.Syntax

namespace Una.F1

/-- The value of `a` in an association list, the first binding. -/
theorem lookup_map {α β γ : Type} [BEq α] (f : β → γ) (a : α) :
    ∀ (σ : List (α × β)),
      List.lookup a (σ.map (fun q => (q.1, f q.2))) = (List.lookup a σ).map f
  | [] => rfl
  | (k, v) :: σ => by
    simp only [List.map_cons, List.lookup]
    split <;> simp_all [lookup_map f a σ]

theorem lookup_append {α β : Type} [BEq α] (a : α) :
    ∀ (σ τ : List (α × β)),
      List.lookup a (σ ++ τ) = match List.lookup a σ with
        | some v => some v
        | none => List.lookup a τ
  | [], τ => rfl
  | (k, v) :: σ, τ => by
    simp only [List.cons_append, List.lookup]
    split
    · rfl
    · exact lookup_append a σ τ

mutual
  /-- Substitution of types for type parameters. A parameter that `σ` does not cover stays. -/
  def Ty.subst (σ : List (String × Ty)) : Ty → Ty
    | .int => .int
    | .bool => .bool
    | .text => .text
    | .unit => .unit
    | .list t => .list (t.subst σ)
    | .option t => .option (t.subst σ)
    | .result t => .result (t.subst σ)
    | .record r => .record r
    | .fn ps r => .fn (Ty.substs σ ps) (r.subst σ)
    | .var a => (List.lookup a σ).getD (.var a)
    | .tuple ts => .tuple (Ty.substs σ ts)

  def Ty.substs (σ : List (String × Ty)) : List Ty → List Ty
    | [] => []
    | t :: ts => t.subst σ :: Ty.substs σ ts
end

theorem Ty.substs_eq (σ : List (String × Ty)) :
    ∀ ts : List Ty, Ty.substs σ ts = ts.map (Ty.subst σ)
  | [] => rfl
  | t :: ts => by simp [Ty.substs, Ty.substs_eq σ ts]

/-- A substitution after another is one substitution. -/
theorem Ty.subst_subst (σ τ : List (String × Ty)) :
    ∀ t : Ty, (t.subst σ).subst τ = t.subst (σ.map (fun q => (q.1, q.2.subst τ)) ++ τ)
  | .int | .bool | .text | .unit | .record _ => by simp [Ty.subst]
  | .list t | .option t | .result t => by simp [Ty.subst, Ty.subst_subst σ τ t]
  | .fn ps r => by
    simp only [Ty.subst, Ty.substs_eq, List.map_map, Ty.subst_subst σ τ r]
    congr 1
    apply List.map_congr_left
    intro t _
    exact Ty.subst_subst σ τ t
  | .var a => by
    simp only [Ty.subst, lookup_append, lookup_map]
    cases List.lookup a σ <;> simp [Ty.subst]
  | .tuple ts => by
    simp only [Ty.subst, Ty.substs_eq, List.map_map]
    congr 1
    apply List.map_congr_left
    intro t _
    exact Ty.subst_subst σ τ t

theorem Ty.subst_nil : ∀ t : Ty, t.subst [] = t
  | .int | .bool | .text | .unit | .record _ => rfl
  | .list t | .option t | .result t => by simp [Ty.subst, Ty.subst_nil t]
  | .fn ps r => by
    simp only [Ty.subst, Ty.substs_eq, Ty.subst_nil r]
    congr 1
    conv => rhs; rw [← List.map_id ps]
    apply List.map_congr_left
    intro t _
    exact Ty.subst_nil t
  | .var a => rfl
  | .tuple ts => by
    simp only [Ty.subst, Ty.substs_eq]
    congr 1
    conv => rhs; rw [← List.map_id ts]
    apply List.map_congr_left
    intro t _
    exact Ty.subst_nil t

/-- A type without type parameters, which no substitution changes. -/
def Ty.Closed (t : Ty) : Prop := ∀ σ, t.subst σ = t

mutual
  /-- The structure of a labeled type. -/
  def LTy.erase : LTy → Ty
    | .int _ => .int
    | .bool _ => .bool
    | .text _ => .text
    | .unit _ => .unit
    | .list t _ => .list t.erase
    | .option t _ => .option t.erase
    | .result t _ => .result t.erase
    | .record r _ => .record r
    | .fn ps r _ _ _ _ => .fn (LTy.erases ps) r.erase
    | .var a _ => .var a
    | .tuple ts _ => .tuple (LTy.erases ts)

  def LTy.erases : List LTy → List Ty
    | [] => []
    | t :: ts => t.erase :: LTy.erases ts
end

theorem LTy.erases_eq : ∀ ts : List LTy, LTy.erases ts = ts.map LTy.erase
  | [] => rfl
  | t :: ts => by simp [LTy.erases, LTy.erases_eq ts]

/-- The top label of a labeled type. -/
def LTy.top : LTy → LExpr
  | .int l | .bool l | .text l | .unit l | .list _ l | .option _ l | .result _ l
  | .record _ l | .fn _ _ _ _ _ l | .var _ l | .tuple _ l => l

/-- `τ ⊔ ℓ`: the type with its top label raised by `ℓ`. -/
def LTy.raise (e : LExpr) : LTy → LTy
  | .int l => .int (l.join e)
  | .bool l => .bool (l.join e)
  | .text l => .text (l.join e)
  | .unit l => .unit (l.join e)
  | .list t l => .list t (l.join e)
  | .option t l => .option t (l.join e)
  | .result t l => .result t (l.join e)
  | .record r l => .record r (l.join e)
  | .fn ps r o g f l => .fn ps r o g f (l.join e)
  | .var a l => .var a (l.join e)
  | .tuple ts l => .tuple ts (l.join e)

theorem LTy.erase_raise (e : LExpr) (t : LTy) : (t.raise e).erase = t.erase := by
  cases t <;> rfl

mutual
  /-- `ℓ*`: the join of every label of a labeled type (`spec/07 §7` rule 3). A position of type
  `T` gives `ℓ*_T`, every label of `T`'s type argument, and a record type the labels its fields
  declare, which `rd` gives by name (`Program.recordDeep`). -/
  def LTy.deep (rd : String → Label) : LTy → LExpr
    | .int l | .bool l | .text l | .unit l => l
    | .record r l => l.join (.ofLabel (rd r))
    | .list t l | .option t l | .result t l => l.join (t.deep rd)
    | .fn ps r _ _ _ l => l.join ((LTy.deeps rd ps).join (r.deep rd))
    | .var a l => l.promote.join ⟨Label.bot, [], [a]⟩
    | .tuple ts l => l.join (LTy.deeps rd ts)

  def LTy.deeps (rd : String → Label) : List LTy → LExpr
    | [] => LExpr.bot
    | t :: ts => (t.deep rd).join (LTy.deeps rd ts)
end

mutual
  /-- Labeled instantiation (`spec/10 §5`): each type parameter `T` that `σ` covers becomes its
  type argument, raised by the position's other labels; elsewhere `ℓ_T` and `ℓ*_T` become
  `ℓ*(σ(T))`. At a position of type `T`, `ℓ_T` is the type argument's own labels, in place
  (ADR 0067). -/
  def LTy.lsubst (rd : String → Label) (σ : List (String × LTy)) : LTy → LTy
    | .int l => .int (l.inst (σ.map (fun q => (q.1, q.2.deep rd))))
    | .bool l => .bool (l.inst (σ.map (fun q => (q.1, q.2.deep rd))))
    | .text l => .text (l.inst (σ.map (fun q => (q.1, q.2.deep rd))))
    | .unit l => .unit (l.inst (σ.map (fun q => (q.1, q.2.deep rd))))
    | .list t l => .list (t.lsubst rd σ) (l.inst (σ.map (fun q => (q.1, q.2.deep rd))))
    | .option t l => .option (t.lsubst rd σ) (l.inst (σ.map (fun q => (q.1, q.2.deep rd))))
    | .result t l => .result (t.lsubst rd σ) (l.inst (σ.map (fun q => (q.1, q.2.deep rd))))
    | .record r l => .record r (l.inst (σ.map (fun q => (q.1, q.2.deep rd))))
    | .fn ps r o g f l =>
      .fn (LTy.lsubsts rd σ ps) (r.lsubst rd σ) o g f (l.inst (σ.map (fun q => (q.1, q.2.deep rd))))
    | .var a l =>
      match List.lookup a σ with
      | some t => t.raise ((l.dropPlain a).inst (σ.map (fun q => (q.1, q.2.deep rd))))
      | none => .var a (l.inst (σ.map (fun q => (q.1, q.2.deep rd))))
    | .tuple ts l => .tuple (LTy.lsubsts rd σ ts) (l.inst (σ.map (fun q => (q.1, q.2.deep rd))))

  def LTy.lsubsts (rd : String → Label) (σ : List (String × LTy)) : List LTy → List LTy
    | [] => []
    | t :: ts => t.lsubst rd σ :: LTy.lsubsts rd σ ts
end

theorem LTy.lsubsts_eq (rd : String → Label) (σ : List (String × LTy)) :
    ∀ ts : List LTy, LTy.lsubsts rd σ ts = ts.map (LTy.lsubst rd σ)
  | [] => rfl
  | t :: ts => by simp [LTy.lsubsts, LTy.lsubsts_eq rd σ ts]

/-- The structure of a substitution of labeled types. -/
def eraseSubst (σ : List (String × LTy)) : List (String × Ty) :=
  σ.map (fun q => (q.1, q.2.erase))

/-- Labeled instantiation has the structure of structural substitution: labels never change
the structure. -/
theorem LTy.erase_lsubst (rd : String → Label) (σ : List (String × LTy)) :
    ∀ t : LTy, (t.lsubst rd σ).erase = t.erase.subst (eraseSubst σ)
  | .int _ | .bool _ | .text _ | .unit _ | .record _ _ => by simp [LTy.lsubst, LTy.erase, Ty.subst]
  | .list t _ | .option t _ | .result t _ => by
    simp [LTy.lsubst, LTy.erase, Ty.subst, LTy.erase_lsubst rd σ t]
  | .fn ps r _ _ _ _ => by
    simp only [LTy.lsubst, LTy.erase, Ty.subst, LTy.erases_eq, LTy.lsubsts_eq, Ty.substs_eq,
      List.map_map, LTy.erase_lsubst rd σ r]
    congr 1
    apply List.map_congr_left
    intro t _
    exact LTy.erase_lsubst rd σ t
  | .var a _ => by
    simp only [LTy.lsubst, LTy.erase, Ty.subst, eraseSubst, lookup_map]
    cases List.lookup a σ <;> simp [LTy.erase_raise, LTy.erase]
  | .tuple ts _ => by
    simp only [LTy.lsubst, LTy.erase, Ty.subst, LTy.erases_eq, LTy.lsubsts_eq, Ty.substs_eq,
      List.map_map]
    congr 1
    apply List.map_congr_left
    intro t _
    exact LTy.erase_lsubst rd σ t

end Una.F1
