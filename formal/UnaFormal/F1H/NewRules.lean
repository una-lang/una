/-
M5, part 2: the probes of `OldLeaks.lean`, under ADR 0085's rules, have no typing in the demo
program, whatever its file-state label: the hypotheses of `Untyped.lean` hold of it.
-/
import UnaFormal.F1H.OldLeaks
import UnaFormal.F1H.Untyped

namespace Una.F1H.NewRules

open Una.F1 (Label LExpr)
open OldLeaks (oldProg intl)
open Untyped

/-- The demo program under the new rules, with file-state label `fs`. -/
def newProg (fs : Label) : Program := { oldProg with oldHost := false, fileState := fs }

/-- `internal` is not below what `Console` accepts, `public`. -/
theorem intl_not_public (fs : Label) : ¬ (newProg fs).le intl (.ofLabel (newProg fs).acceptsOp) := by
  rintro (h | ⟨h, _⟩) <;>
    simp [newProg, oldProg, Program.top, intl, LExpr.ofLabel, Label.le, Label.bot] at h <;>
    revert h <;> decide

/-- **S-14 is rejected**, for every file-state label and every annotation. -/
theorem s14_rejected (fs : Label) (aw au ar ac a₁ a₂ : LTy) {E : List String} {t : LTy} {ε : Eff} :
    ¬ LExprTy (newProg fs) E [("x", .text intl)] .bot (s14 aw au ar ac a₁ a₂) t ε :=
  s14_untyped rfl (intl_not_public fs) aw au ar ac a₁ a₂

/-- **S-15 is rejected**, for every file-state label and every annotation. -/
theorem s15_rejected (fs : Label) (ar ac a₁ a₂ : LTy) {E : List String} {t : LTy} {ε : Eff} :
    ¬ LExprTy (newProg fs) E [("x", .text intl)] .bot (s15 ar ac a₁ a₂) t ε :=
  s15_untyped rfl (intl_not_public fs) ar ac a₁ a₂

/-- The read before the write, and the write under a secret condition, are rejected too. -/
theorem shapes_rejected (fs : Label) :
    (∀ ar ac aw au a₁ a₂ (E : List String) (t : LTy) (ε : Eff),
      ¬ LExprTy (newProg fs) E [("x", .text intl)] .bot (s14rbw ar ac aw au a₁ a₂) t ε) ∧
    (∀ aw au au' ar ac a₁ a₂ (E : List String) (t : LTy) (ε : Eff),
      ¬ LExprTy (newProg fs) E [("c", .bool intl)] .bot (s14cond aw au au' ar ac a₁ a₂) t ε) :=
  ⟨fun ar ac aw au a₁ a₂ _ _ _ => s14rbw_untyped rfl (intl_not_public fs) ar ac aw au a₁ a₂,
    fun aw au au' ar ac a₁ a₂ _ _ _ => s14cond_untyped rfl (intl_not_public fs) aw au au' ar ac a₁ a₂⟩

/-- The probe of S-14 as `OldLeaks` writes it is one of the probes `s14_rejected` covers: the same
program text, typed under the old rules and rejected under the new. -/
theorem s14_same_probe : OldLeaks.s14 =
    s14 (.result (.unit .bot) intl) (.unit .bot) (.result (.text intl) .bot) (.text intl)
      (.unit .bot) (.unit .bot) := rfl

theorem s15_same_probe : OldLeaks.s15 =
    s15 (.result (.text intl) .bot) (.text intl) (.unit .bot) (.unit .bot) := rfl

end Una.F1H.NewRules
