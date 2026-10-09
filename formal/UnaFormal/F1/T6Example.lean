/-
A worked instance of T6: a typed body reads a secret and a public input, computes with the
secret, prints a line to a public sink and returns the public input. Two runs whose secrets differ
show a public observer the same events, and the same value, as `t6` says.
-/
import UnaFormal.F1.Mono

namespace Una.F1.T6Example

def sec : LExpr := .ofLabel ⟨.secret, []⟩

def prog : Program where
  categories := []
  acceptsOp := Label.bot
  acceptsGet := Label.bot
  getResult := Label.bot
  records := []
  policies := []
  funs := []

/-- `s: secret Int, x: Int`. -/
def Γ : LCtx := [("s", .int sec), ("x", .int .bot)]

/-- `let y: secret Int = s + x; let u = @log.write("hi"); x`. -/
def body : Expr :=
  .letR "y" (.int sec) (.prim .add [.var "s", .var "x"])
    (.letR "u" (.unit .bot) (.op (.text "hi")) (.atom (.var "x")))

def eff : Eff := Eff.none.union ((⟨true, false, Option.none⟩ : Eff).union Eff.none)

theorem body_typed : LExprTy prog [] Γ .bot body (.int .bot) eff := by
  refine .letR (t := .int ((LExpr.joinAll [sec, LExpr.bot]))) ?_ (.int ?_) ?_
  · exact .prim (ts := [.int sec, .int .bot]) (.cons (.var rfl) (.cons (.var rfl) .nil)) .add
  · refine .inr ⟨⟨?_, ?_⟩, ?_, ?_⟩ <;>
      simp [sec, LExpr.joinAll, LExpr.join, LExpr.bot, LExpr.ofLabel, Label.join, Label.bot,
        Level.join, Level.rank] <;> decide
  · refine .letR (t := .unit .bot) (.op .text rfl ?_ ?_) .unit (.atom (.var rfl))
    · refine .inr ⟨⟨?_, ?_⟩, ?_, ?_⟩ <;>
        simp [prog, LExpr.join, LExpr.bot, LExpr.ofLabel, Label.join, Label.bot, Eff.failLabel,
          Eff.none, Program.deep, LTy.deep, Level.join, Level.rank] <;> decide
    · simp [LExpr.SymFree, LExpr.join, LExpr.bot, LExpr.ofLabel, Eff.failLabel, Eff.none]

theorem prog_wf : prog.LWellFormed where
  funs _ h := nomatch h
  records _ h := nomatch h
  recordDeep _ h := nomatch h
  policies _ h := nomatch h

theorem prog_gf : prog.GenericFree := ⟨(fun _ h => nomatch h), rfl⟩

theorem prog_get : prog.le (.ofLabel prog.acceptsGet) (.ofLabel prog.getResult) :=
  LExpr.le_refl _

/-- Two inputs with different secrets and the same public value. -/
def env₁ : Env := [("s", .int 1), ("x", .int 5)]
def env₂ : Env := [("s", .int 2), ("x", .int 5)]

/-- A public observer does not see a secret. -/
theorem sec_hidden : ¬ Vis prog Label.bot sec := by
  rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;> simp [prog, Program.top, sec, LExpr.ofLabel, Label.bot] at h <;>
    revert h <;> decide

theorem envs_rel : EnvRel prog Label.bot Γ env₁ env₂ :=
  .cons (.hidden sec_hidden) (.cons .int .nil)

/-- **T6, applied.** The two runs, at any fuel at which both end, show a public observer the same
events and return the same public value, although their secrets differ. -/
theorem body_noninterferent (w : World) (n : Nat)
    (h₁ : Term (eval n prog w env₁ body).2) (h₂ : Term (eval n prog w env₂ body).2) :
    proj prog Label.bot (eval n prog w env₁ body).1 = proj prog Label.bot (eval n prog w env₂ body).1 ∧
      OutRel (V prog Label.bot (.int .bot)) (Vis prog Label.bot eff.failLabel)
        (eval n prog w env₁ body).2 (eval n prog w env₂ body).2 :=
  t6 prog_wf prog_gf prog_get w (.cons .int (.cons .int .nil)) (.cons .int (.cons .int .nil))
    envs_rel body_typed n h₁ h₂

/-- The runs do end, and both print the line and return 5. -/
example (w : World) :
    eval 3 prog w env₁ body = ([.out "hi"], .ok (.int 5)) ∧
      eval 3 prog w env₂ body = ([.out "hi"], .ok (.int 5)) := ⟨rfl, rfl⟩

end Una.F1.T6Example
