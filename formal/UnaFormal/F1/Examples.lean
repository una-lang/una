/-
Examples that the definitions of F1 are not vacuous: a generic function the labeled typing
accepts, whose result carries `ℓ_T` in place, and a run of it.
-/
import UnaFormal.F1.Erase

namespace Una.F1.Examples

/-- `ℓ_T`. -/
def lT : LExpr := ⟨Label.bot, ["T"], []⟩

/-- `pick[T](a: T, b: T, c: Bool) -> T = if c then a else b`. -/
def pick : Fun where
  name := "pick"
  tparams := ["T"]
  params := [("a", .var "T" lT), ("b", .var "T" lT), ("c", .bool .bot)]
  result := .var "T" lT
  op := false
  get := false
  fails := false
  body := .ite (.var "c") (.atom (.var "a")) (.atom (.var "b"))

def prog : Program where
  categories := []
  acceptsOp := Label.bot
  acceptsGet := Label.bot
  getResult := Label.bot
  records := []
  policies := []
  funs := [pick]

theorem le_self (e : LExpr) : LExpr.le prog.top e e :=
  .inr ⟨Label.le_refl _, fun _ hx => .inl hx, fun _ hx => hx⟩

theorem lT_join_le : LExpr.le prog.top (lT.join LExpr.bot) lT :=
  .inr ⟨⟨Nat.le_refl _, fun _ hx => by
      simp [lT, LExpr.join, LExpr.bot, LExpr.ofLabel, Label.bot, Label.join] at hx⟩,
    fun x hx => .inl (by simpa [lT, LExpr.join, LExpr.bot, LExpr.ofLabel] using hx),
    fun _ hx => by simp [lT, LExpr.join, LExpr.bot, LExpr.ofLabel] at hx⟩

/-- The labeled typing accepts `pick`: its result is `T` with `ℓ_T`, which a call replaces by
the type argument's own labels. -/
theorem pick_ok : prog.FunOk pick := by
  refine ⟨.var "T" (lT.join .bot), ⟨false || false, false || false, Option.none⟩,
    ?_, .var lT_join_le, by simp, by simp, by simp, ?_⟩
  · have hthen : LExprTy prog [("a", .var "T" lT), ("b", .var "T" lT), ("c", .bool .bot)]
        (LExpr.bot.join .bot) (.atom (.var "a")) (.var "T" lT) .none :=
      .atom (.var rfl)
    have helse : LExprTy prog [("a", .var "T" lT), ("b", .var "T" lT), ("c", .bool .bot)]
        (LExpr.bot.join .bot) (.atom (.var "b")) (.var "T" lT) .none :=
      .atom (.var rfl)
    exact .ite (.var rfl) hthen helse (.var (le_self lT)) (.var (le_self lT))
  · exact le_self _

/-- A run of `pick[Int](1, 2, true)` gives `1`. -/
example :
    (eval 10 prog ⟨fun _ => none⟩ []
      (.letR "x" (.int .bot) (.call "pick" [.int .bot] [.int 1, .int 2, .bool true])
        (.atom (.var "x")))).2 = .ok (.int 1) := by
  rfl

end Una.F1.Examples
