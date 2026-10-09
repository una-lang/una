/-
Examples that the definitions of F1 are not vacuous: a generic function the labeled typing
accepts, whose result carries `ℓ_T` in place, a program it accepts, and T1 applied to a typed
call of the function and to its run.
-/
import UnaFormal.F1.Erase

namespace Una.F1.Examples

/-- `ℓ_T`. -/
def lT : LExpr := ⟨Label.bot, ["T"], []⟩

/-- `pick[T](a: T, b: T, c: Bool) -> T = if c then a else b`. -/
def pick : Fun where
  name := "pick"
  tparams := ["T"]
  eqParams := []
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

theorem bot_le (e : LExpr) : LExpr.le prog.top LExpr.bot e :=
  .inr ⟨⟨Level.public_le _, fun _ h => by simp [LExpr.bot, LExpr.ofLabel, Label.bot] at h⟩,
    fun _ h => by simp [LExpr.bot, LExpr.ofLabel] at h,
    fun _ h => by simp [LExpr.bot, LExpr.ofLabel] at h⟩

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

/-- The labeled typing accepts the program. -/
theorem prog_wf : prog.LWellFormed where
  funs fn h := by
    simp only [prog, List.mem_singleton] at h
    subst h
    exact .inl pick_ok
  records rec h := by simp [prog] at h
  recordDeep rec h := by simp [prog] at h
  policies po h := by simp [prog] at h

/-- The type of `pick[Int](...)`: `Int`, with `ℓ_T` replaced in place by the argument's labels. -/
def pickInt : LTy := prog.lsub [("T", .int .bot)] pick.result

/-- `let x: Int = pick[Int](1, 2, true) in x`. -/
def callPick : Expr :=
  .letR "x" pickInt (.call "pick" [.int .bot] [.int 1, .int 2, .bool true]) (.atom (.var "x"))

/-- The labeled typing accepts the call at the public program counter. -/
theorem callPick_typed :
    LExprTy prog [] .bot callPick pickInt ((Eff.ofFn false false false).union .none) := by
  refine .letR (t := pickInt) ?_ (.int (le_self _)) (.atom (.var rfl))
  refine LRhsTy.call (fn := pick) (targs := [.int .bot]) rfl ?_ rfl rfl ?_ (.cons .int (.cons .int (.cons .bool .nil)))
    (.cons (.int (bot_le _)) (.cons (.int (bot_le _)) (.cons (.bool (bot_le _)) .nil)))
    ⟨fun h => (by cases h), fun h => (by cases h), fun h => (by cases h)⟩
  · intro ⟨_, h, _⟩
    simp [prog] at h
  · intro q _ h
    simp [pick] at h

/-- **T1, applied:** the run of the typed call never gets stuck, and its value is an integer. -/
theorem callPick_safe (w : World) (n : Nat) :
    (eval n prog w [] callPick).2 ≠ .stuck ∧
      ∀ v, (eval n prog w [] callPick).2 = .ok v → ValTy prog v pickInt.erase :=
  t1_labeled prog_wf w .nil callPick_typed n

/-- A run of `pick[Int](1, 2, true)` gives `1`. -/
example :
    (eval 10 prog ⟨fun _ => none⟩ []
      (.letR "x" (.int .bot) (.call "pick" [.int .bot] [.int 1, .int 2, .bool true])
        (.atom (.var "x")))).2 = .ok (.int 1) := by
  rfl

end Una.F1.Examples
