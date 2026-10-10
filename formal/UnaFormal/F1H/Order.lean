/-
The static order of labels (`spec/07 §7` rule 8) and flows (rule 2) are preorders, and joins are
least upper bounds for the order: the facts M3's proofs use about the checker's comparisons.
-/
import UnaFormal.F1H.LTyping
import UnaFormal.F1.Order

namespace Una.F1H

open Una.F1 (Label LExpr)


mutual
  theorem Flow.refl {top : Label} : ∀ t : LTy, Flow top t t
    | .int l => .int (LExpr.le_refl l)
    | .bool l => .bool (LExpr.le_refl l)
    | .text l => .text (LExpr.le_refl l)
    | .unit _ => .unit
    | .list t l => .list (Flow.refl t) (LExpr.le_refl l)
    | .option t l => .option (Flow.refl t) (LExpr.le_refl l)
    | .result t l => .result (Flow.refl t) (LExpr.le_refl l)
    | .record _ l => .record (LExpr.le_refl l)
    | .fn ps r _ _ _ l =>
      .fn (Flows.refl ps) (Flow.refl r) id (HostFx.Le.refl _) id (LExpr.le_refl l)
    | .var _ l => .var (LExpr.le_refl l)

  theorem Flows.refl {top : Label} : ∀ ts : List LTy, Flows top ts ts
    | [] => .nil
    | t :: ts => .cons (Flow.refl t) (Flows.refl ts)
end

mutual
  theorem Flow.trans {top : Label} :
      ∀ {a b c : LTy}, Flow top a b → Flow top b c → Flow top a c
    | _, _, _, .int h₁, .int h₂ => .int (LExpr.le_trans h₁ h₂)
    | _, _, _, .bool h₁, .bool h₂ => .bool (LExpr.le_trans h₁ h₂)
    | _, _, _, .text h₁, .text h₂ => .text (LExpr.le_trans h₁ h₂)
    | _, _, _, .unit, .unit => .unit
    | _, _, _, .list f₁ h₁, .list f₂ h₂ => .list (Flow.trans f₁ f₂) (LExpr.le_trans h₁ h₂)
    | _, _, _, .option f₁ h₁, .option f₂ h₂ => .option (Flow.trans f₁ f₂) (LExpr.le_trans h₁ h₂)
    | _, _, _, .result f₁ h₁, .result f₂ h₂ => .result (Flow.trans f₁ f₂) (LExpr.le_trans h₁ h₂)
    | _, _, _, .record h₁, .record h₂ => .record (LExpr.le_trans h₁ h₂)
    | _, _, _, .fn ps₁ r₁ o₁ g₁ f₁ h₁, .fn ps₂ r₂ o₂ g₂ f₂ h₂ =>
      .fn (Flows.trans ps₂ ps₁) (Flow.trans r₁ r₂) (o₂ ∘ o₁) (g₁.trans g₂) (f₂ ∘ f₁)
        (LExpr.le_trans h₁ h₂)
    | _, _, _, .var h₁, .var h₂ => .var (LExpr.le_trans h₁ h₂)

  theorem Flows.trans {top : Label} :
      ∀ {a b c : List LTy}, Flows top a b → Flows top b c → Flows top a c
    | _, _, _, .nil, .nil => .nil
    | _, _, _, .cons f₁ fs₁, .cons f₂ fs₂ => .cons (Flow.trans f₁ f₂) (Flows.trans fs₁ fs₂)
end

/-- Raising a type's top label is a flow into it. -/
theorem Flow.raise {top : Label} (e : LExpr) : ∀ t : LTy, Flow top t (t.raise e)
  | .int l => .int (LExpr.le_join_left l e)
  | .bool l => .bool (LExpr.le_join_left l e)
  | .text l => .text (LExpr.le_join_left l e)
  | .unit _ => .unit
  | .list t l => .list (Flow.refl t) (LExpr.le_join_left l e)
  | .option t l => .option (Flow.refl t) (LExpr.le_join_left l e)
  | .result t l => .result (Flow.refl t) (LExpr.le_join_left l e)
  | .record _ l => .record (LExpr.le_join_left l e)
  | .fn ps r _ _ _ l => .fn (Flows.refl ps) (Flow.refl r) id (HostFx.Le.refl _) id (LExpr.le_join_left l e)
  | .var _ l => .var (LExpr.le_join_left l e)

end Una.F1H
