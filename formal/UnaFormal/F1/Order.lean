/-
The static order of labels (`spec/07 §7` rule 8) and flows (rule 2) are preorders, and joins are
least upper bounds for the order: the facts M3's proofs use about the checker's comparisons.
-/
import UnaFormal.F1.LTyping

namespace Una.F1

namespace LExpr

variable {top : Label}

theorem le_refl (e : LExpr) : LExpr.le top e e :=
  .inr ⟨Label.le_refl _, fun _ hx => .inl hx, fun _ hx => hx⟩

theorem le_trans {a b c : LExpr} (h₁ : LExpr.le top a b) (h₂ : LExpr.le top b c) :
    LExpr.le top a c := by
  rcases h₂ with htop | ⟨hb₂, hp₂, hs₂⟩
  · exact .inl htop
  rcases h₁ with htop | ⟨hb₁, hp₁, hs₁⟩
  · exact .inl (Label.le_trans htop hb₂)
  refine .inr ⟨Label.le_trans hb₁ hb₂, fun x hx => ?_, fun x hx => hs₂ x (hs₁ x hx)⟩
  rcases hp₁ x hx with h | h
  · exact hp₂ x h
  · exact .inr (hs₂ x h)

theorem le_join_left (a b : LExpr) : LExpr.le top a (a.join b) :=
  .inr ⟨Label.le_join_left _ _, fun _ hx => .inl (List.mem_append_left _ hx),
    fun _ hx => List.mem_append_left _ hx⟩

theorem le_join_right (a b : LExpr) : LExpr.le top b (a.join b) :=
  .inr ⟨Label.le_join_right _ _, fun _ hx => .inl (List.mem_append_right _ hx),
    fun _ hx => List.mem_append_right _ hx⟩

theorem join_le {a b c : LExpr} (ha : LExpr.le top a c) (hb : LExpr.le top b c) :
    LExpr.le top (a.join b) c := by
  rcases ha with htop | ⟨hba, hpa, hsa⟩
  · exact .inl htop
  rcases hb with htop | ⟨hbb, hpb, hsb⟩
  · exact .inl htop
  refine .inr ⟨Label.join_le hba hbb, fun x hx => ?_, fun x hx => ?_⟩
  · rcases List.mem_append.mp hx with h | h
    · exact hpa x h
    · exact hpb x h
  · rcases List.mem_append.mp hx with h | h
    · exact hsa x h
    · exact hsb x h

theorem le_of_join_left {a b c : LExpr} (h : LExpr.le top (a.join b) c) : LExpr.le top a c :=
  le_trans (le_join_left a b) h

theorem le_of_join_right {a b c : LExpr} (h : LExpr.le top (a.join b) c) : LExpr.le top b c :=
  le_trans (le_join_right a b) h

theorem bot_le (e : LExpr) : LExpr.le top LExpr.bot e :=
  .inr ⟨⟨Level.public_le _, fun _ h => by simp [bot, ofLabel, Label.bot] at h⟩,
    fun _ h => by simp [bot, ofLabel] at h, fun _ h => by simp [bot, ofLabel] at h⟩

theorem join_mono {a b c d : LExpr} (h₁ : LExpr.le top a c) (h₂ : LExpr.le top b d) :
    LExpr.le top (a.join b) (c.join d) :=
  join_le (le_trans h₁ (le_join_left c d)) (le_trans h₂ (le_join_right c d))

end LExpr


mutual
  theorem Flow.refl {top : Label} : ∀ t : LTy, Flow top t t
    | .int l => .int (LExpr.le_refl l)
    | .bool l => .bool (LExpr.le_refl l)
    | .text l => .text (LExpr.le_refl l)
    | .unit _ => .unit
    | .list t l => .list (Flow.refl t) (LExpr.le_refl l)
    | .option t l => .option (Flow.refl t) (LExpr.le_refl l)
    | .result t e l => .result (Flow.refl t) (Flow.refl e) (LExpr.le_refl l)
    | .record _ l => .record (LExpr.le_refl l)
    | .fn ps r _ _ _ l =>
      .fn (Flows.refl ps) (Flow.refl r) id (HostFx.Le.refl _) id (LExpr.le_refl l)
    | .var _ l => .var (LExpr.le_refl l)
    | .tuple ts l => .tuple (Flows.refl ts) (LExpr.le_refl l)

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
    | _, _, _, .result f₁ g₁ h₁, .result f₂ g₂ h₂ =>
      .result (Flow.trans f₁ f₂) (Flow.trans g₁ g₂) (LExpr.le_trans h₁ h₂)
    | _, _, _, .record h₁, .record h₂ => .record (LExpr.le_trans h₁ h₂)
    | _, _, _, .fn ps₁ r₁ o₁ g₁ f₁ h₁, .fn ps₂ r₂ o₂ g₂ f₂ h₂ =>
      .fn (Flows.trans ps₂ ps₁) (Flow.trans r₁ r₂) (o₂ ∘ o₁) (g₁.trans g₂) (f₂ ∘ f₁)
        (LExpr.le_trans h₁ h₂)
    | _, _, _, .var h₁, .var h₂ => .var (LExpr.le_trans h₁ h₂)
    | _, _, _, .tuple f₁ h₁, .tuple f₂ h₂ => .tuple (Flows.trans f₁ f₂) (LExpr.le_trans h₁ h₂)

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
  | .result t e' l => .result (Flow.refl t) (Flow.refl e') (LExpr.le_join_left l e)
  | .record _ l => .record (LExpr.le_join_left l e)
  | .fn ps r _ _ _ l => .fn (Flows.refl ps) (Flow.refl r) id (HostFx.Le.refl _) id (LExpr.le_join_left l e)
  | .var _ l => .var (LExpr.le_join_left l e)
  | .tuple ts l => .tuple (Flows.refl ts) (LExpr.le_join_left l e)

end Una.F1
