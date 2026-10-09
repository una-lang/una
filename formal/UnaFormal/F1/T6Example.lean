/-
A worked instance of T6: a typed body reads a secret and a public input, computes with the
secret, prints a line to a public sink and returns the public input. Two runs whose secrets differ
show a public observer the same events, and the same value, as `t6` says.
-/
import UnaFormal.F1.Mono
import UnaFormal.F1.Examples

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
  refine .letR (t := .int ((LExpr.joinAll [sec, LExpr.bot]))) ?_ (.int ?_) ?_ (by simp [LTy.InPlace, sec, LExpr.ofLabel])
  · exact .prim (ts := [.int sec, .int .bot]) (.cons (.var rfl) (.cons (.var rfl) .nil)) .add
  · refine .inr ⟨⟨?_, ?_⟩, ?_, ?_⟩ <;>
      simp [sec, LExpr.joinAll, LExpr.join, LExpr.bot, LExpr.ofLabel, Label.join, Label.bot,
        Level.join, Level.rank] <;> decide
  · refine .letR (t := .unit .bot) (.op .text rfl ?_ ?_) .unit (.atom (.var rfl)) (by simp [LTy.InPlace, LExpr.bot, LExpr.ofLabel])
    · refine .inr ⟨⟨?_, ?_⟩, ?_, ?_⟩ <;>
        simp [prog, LExpr.join, LExpr.bot, LExpr.ofLabel, Label.join, Label.bot, Eff.failLabel,
          Eff.none, Program.deep, LTy.deep, Level.join, Level.rank] <;> decide
    · simp [LExpr.SymFree, LExpr.join, LExpr.bot, LExpr.ofLabel, Eff.failLabel, Eff.none]

theorem prog_wf : prog.LWellFormed where
  funs _ h := nomatch h
  records _ h := nomatch h
  recordDeep _ h := nomatch h
  policies _ h := nomatch h
  categories := by
    refine ⟨?_, ?_, ?_, ?_, ?_, ?_⟩ <;> simp [prog, Label.bot]
  recordsGround _ h := nomatch h
  eqScoped _ h := nomatch h
  sigsScoped _ h := nomatch h
  inPlace := by refine ⟨?_, ?_, ?_⟩ <;> simp [prog]

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

/-- **T6, applied.** The two runs, at a fuel at which both end, show a public observer the same
events and return the same public value, although their secrets differ. -/
theorem body_noninterferent (w : World) (n : Nat)
    (h₁ : Term (eval n prog w env₁ body).2) (h₂ : Term (eval n prog w env₂ body).2) :
    proj prog Label.bot (eval n prog w env₁ body).1 = proj prog Label.bot (eval n prog w env₂ body).1 ∧
      OutRel (V prog Label.bot (.int .bot)) (Vis prog Label.bot eff.failLabel)
        (eval n prog w env₁ body).2 (eval n prog w env₂ body).2 :=
  t6_ground prog_wf rfl prog_get w
    (fun q hq => by simp only [List.mem_cons, List.not_mem_nil, or_false] at hq
                    rcases hq with rfl | rfl <;> rfl) rfl
    (.cons .int (.cons .int .nil)) (.cons .int (.cons .int .nil)) envs_rel body_typed n h₁ h₂

/-- The runs do end, and both print the line and return 5. -/
example (w : World) :
    eval 3 prog w env₁ body = ([.out "hi"], .ok (.int 5)) ∧
      eval 3 prog w env₂ body = ([.out "hi"], .ok (.int 5)) := ⟨rfl, rfl⟩

/-! ### Branching on the secret -/

/-- `s: secret Int, x: Text`. -/
def Γb : LCtx := [("s", .int sec), ("x", .text .bot)]

/-- The label `secret ⊔ ⊥` that the comparison and the branch give. -/
def cl : LExpr := LExpr.joinAll [sec, LExpr.bot]

/-- `let c: secret Bool = s < 0; let y: secret Int = if c then 1 else 2; @log.write(x); y`. -/
def branching : Expr :=
  .letR "c" (.bool sec) (.prim .lt [.var "s", .int 0])
    (.letE "y" (.int sec) (.ite (.var "c") (.atom (.int 1)) (.atom (.int 2)))
      (.letR "u" (.unit .bot) (.op (.var "x")) (.atom (.var "y"))))

theorem le_sec {a : LExpr} (hb : a.base.level.rank ≤ 3) (hc : a.base.cats = [])
    (hp : a.plain = []) (hs : a.star = []) : LExpr.le prog.top a sec :=
  .inr ⟨⟨hb, fun _ h => (by rw [hc] at h; nomatch h)⟩, fun _ h => (by rw [hp] at h; nomatch h),
    fun _ h => (by rw [hs] at h; nomatch h)⟩

def brEff : Eff :=
  Eff.none.union ((Eff.none.union Eff.none).union ((⟨true, false, Option.none⟩ : Eff).union Eff.none))

theorem branching_typed : LExprTy prog [] Γb .bot branching (.int sec) brEff := by
  refine .letR (t := .bool cl) ?r1 (.bool ?l1) ?b1 (by simp [LTy.InPlace, sec, LExpr.ofLabel])
  case r1 => exact .prim (ts := [.int sec, .int .bot]) (.cons (.var rfl) (.cons .int .nil)) .lt
  case l1 =>
    apply le_sec <;> simp [cl, sec, LExpr.joinAll, LExpr.join, LExpr.bot, LExpr.ofLabel,
      Label.join, Label.bot, Level.join, Level.rank]
  case b1 =>
    refine .letE (t := (LTy.int .bot).raise sec) ?r2 (.int ?l2) ?b2 (by simp [LTy.InPlace, sec, LExpr.ofLabel])
    case r2 =>
      exact .ite (.var rfl) (.atom .int) (.atom .int) (.int (LExpr.le_refl _))
        (.int (LExpr.le_refl _)) (by simp [LTy.InPlace, LExpr.bot, LExpr.ofLabel])
    case l2 =>
      apply le_sec <;> simp [sec, LExpr.join, LExpr.bot, LExpr.ofLabel, Label.join,
        Label.bot, Level.join, Level.rank]
    case b2 =>
      refine .letR (t := .unit .bot) (.op (.var rfl) rfl ?le ?sf) .unit (.atom (.var rfl)) (by simp [LTy.InPlace, LExpr.bot, LExpr.ofLabel])
      case le =>
        refine .inr ⟨⟨?_, ?_⟩, ?_, ?_⟩ <;>
          simp [prog, LExpr.join, LExpr.bot, LExpr.ofLabel, Label.join, Label.bot, Eff.failLabel,
            Eff.none, Eff.union, Program.deep, LTy.deep, Level.join, Level.rank] <;> decide
      case sf =>
        simp [LExpr.SymFree, LExpr.join, LExpr.bot, LExpr.ofLabel, Eff.failLabel, Eff.none,
          Eff.union]

/-- Two runs whose secrets send them down different branches. -/
def envNeg : Env := [("s", .int (-1)), ("x", .text "hi")]
def envPos : Env := [("s", .int 5), ("x", .text "hi")]

theorem branch_envs_rel : EnvRel prog Label.bot Γb envNeg envPos :=
  .cons (.hidden sec_hidden) (.cons .text .nil)

/-- The runs take different branches and return different values, 1 and 2, after the same line. -/
example (w : World) :
    eval 4 prog w envNeg branching = ([.out "hi"], .ok (.int 1)) ∧
      eval 4 prog w envPos branching = ([.out "hi"], .ok (.int 2)) := ⟨rfl, rfl⟩

/-- **T6 on a branch over the secret.** The public observer sees the same events from both runs,
and their values, 1 and 2, are related at the type whose label it does not see. -/
theorem branching_noninterferent (w : World) :
    proj prog Label.bot (eval 4 prog w envNeg branching).1 =
        proj prog Label.bot (eval 4 prog w envPos branching).1 ∧
      V prog Label.bot (.int sec) (.int 1) (.int 2) := by
  have h := t6_ground (e₁ := envNeg) (e₂ := envPos) prog_wf rfl prog_get w
    (fun q hq => by simp only [List.mem_cons, List.not_mem_nil, or_false] at hq
                    rcases hq with rfl | rfl <;> rfl) rfl
    (.cons .int (.cons .text .nil)) (.cons .int (.cons .text .nil)) branch_envs_rel
    branching_typed 4 trivial trivial
  have r₁ : eval 4 prog w envNeg branching = ([.out "hi"], .ok (.int 1)) := rfl
  have r₂ : eval 4 prog w envPos branching = ([.out "hi"], .ok (.int 2)) := rfl
  rw [r₁, r₂] at h ⊢
  exact h

/-- `c: secret Bool`. -/
def Γc : LCtx := [("c", .bool sec)]

/-- `if c then @log.write("a") else @log.write("b")`: which line is written reveals `c`. -/
def leaking : Expr :=
  .ite (.var "c") (.letR "u" (.unit .bot) (.op (.text "a")) (.atom (.var "u")))
    (.letR "u" (.unit .bot) (.op (.text "b")) (.atom (.var "u")))

/-- **The labeled typing rejects it**: under the secret program counter, the sink, which accepts
`public`, is U4003. -/
theorem leaking_untyped {E : List String} {pc : LExpr} {t : LTy} {ε : Eff} :
    ¬ LExprTy prog E Γc pc leaking t ε := by
  intro h
  cases h with
  | ite hc ht _ _ _ =>
    cases hc with
    | var hx =>
      simp only [Γc, lctxLookup, ite_true] at hx
      cases hx
      cases ht with
      | letR hr _ _ =>
        cases hr with
        | op _ _ hle _ =>
          exact sec_hidden (LExpr.le_of_join_right (LExpr.le_of_join_left hle))

/-! ### A generic call -/

/-- `s: secret Int, c: Bool`. -/
def Γg : LCtx := [("s", .int sec), ("c", .bool .bot)]

/-- The type `pick[secret Int]` returns. -/
def pickSec : LTy := Examples.prog.lsub [("T", .int sec)] Examples.pick.result

/-- `let y = pick[secret Int](s, 0, c); c`: a generic function instantiated with a secret type. -/
def genericCall : Expr :=
  .letR "y" pickSec (.call "pick" [.int sec] [.var "s", .int 0, .var "c"]) (.atom (.var "c"))

theorem ground_sec : GroundSubst [("T", LTy.int sec)] := by
  intro q hq
  simp only [List.mem_singleton] at hq
  subst hq
  exact ⟨rfl, rfl⟩

theorem genericCall_typed :
    LExprTy Examples.prog [] Γg .bot genericCall (.bool .bot)
      ((Eff.ofFn false false false).union .none) := by
  refine .letR (t := pickSec) ?_ (Flow.refl _) (.atom (.var rfl))
    (LTy.lsubst_inPlace _ ground_sec (by simp [Examples.pick, LTy.InPlace, Examples.lT]))
  refine LRhsTy.call (fn := Examples.pick) (targs := [.int sec]) rfl ?_ rfl rfl ?_ ⟨⟨rfl, rfl⟩, trivial⟩
    (.cons (.var rfl) (.cons .int (.cons (.var rfl) .nil)))
    (.cons (.int (LExpr.le_join_left _ _)) (.cons (.int (LExpr.bot_le _)) (.cons (.bool (LExpr.bot_le _)) .nil)))
    ⟨fun h => (by cases h), fun h => (by cases h), fun h => (by cases h)⟩
  · intro ⟨_, h, _⟩; simp [Examples.prog] at h
  · intro q _ h; simp [Examples.pick] at h

def genEnv₁ : Env := [("s", .int 1), ("c", .bool true)]
def genEnv₂ : Env := [("s", .int 2), ("c", .bool true)]

theorem genericCall_noninterferent (w : World) :
    proj Examples.prog Label.bot (eval 5 Examples.prog w genEnv₁ genericCall).1 =
        proj Examples.prog Label.bot (eval 5 Examples.prog w genEnv₂ genericCall).1 ∧
      OutRel (V Examples.prog Label.bot (.bool .bot))
        (Vis Examples.prog Label.bot ((Eff.ofFn false false false).union .none).failLabel)
        (eval 5 Examples.prog w genEnv₁ genericCall).2 (eval 5 Examples.prog w genEnv₂ genericCall).2 := by
  have hsec : ¬ Vis Examples.prog Label.bot sec := by
    rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;>
      simp [Examples.prog, Program.top, sec, LExpr.ofLabel, Label.bot] at h <;> revert h <;> decide
  exact t6_ground (e₁ := genEnv₁) (e₂ := genEnv₂) Examples.prog_wf rfl (LExpr.le_refl _) w
    (fun q hq => by simp only [List.mem_cons, List.not_mem_nil, or_false] at hq
                    rcases hq with rfl | rfl <;> rfl) rfl
    (.cons .int (.cons .bool .nil)) (.cons .int (.cons .bool .nil))
    (.cons (.hidden hsec) (.cons .bool .nil)) genericCall_typed 5 trivial trivial

/-! ### The result of a generic call -/

/-- `s: secret Int`, which the call does not read. -/
def Γs : LCtx := [("s", .int sec)]

/-- `pick[Int](1, 2, true)` typed with the secret in scope. -/
theorem callPick_typedΓ :
    LExprTy Examples.prog [] Γs .bot Examples.callPick Examples.pickInt
      ((Eff.ofFn false false false).union .none) := by
  refine .letR (t := Examples.pickInt) ?_ (.int (LExpr.le_refl _)) (.atom (.var rfl)) ?ip
  case ip => simp [Examples.pickInt, Program.lsub, LTy.lsubst, Examples.pick, Examples.lT,
    LTy.InPlace, LTy.raise, LExpr.join, LExpr.inst, LExpr.dropPlain, LExpr.joinAll, LExpr.bot,
    LExpr.ofLabel]
  refine LRhsTy.call (fn := Examples.pick) (targs := [.int .bot]) rfl ?_ rfl rfl ?_
    (by simp [LTy.GroundAll, LTy.Ground, LExpr.SymFree, LExpr.bot, LExpr.ofLabel])
    (.cons .int (.cons .int (.cons .bool .nil)))
    (.cons (.int (LExpr.bot_le _)) (.cons (.int (LExpr.bot_le _)) (.cons (.bool (LExpr.bot_le _)) .nil)))
    ⟨fun h => (by cases h), fun h => (by cases h), fun h => (by cases h)⟩
  · intro ⟨_, h, _⟩; simp [Examples.prog] at h
  · intro q _ h; simp [Examples.pick] at h

def sEnv₁ : Env := [("s", .int 1)]
def sEnv₂ : Env := [("s", .int 2)]

/-- The runs do return a value: `pick[Int](1, 2, true)` is 1. -/
example (w : World) : (eval 5 Examples.prog w sEnv₁ Examples.callPick).2 = .ok (.int 1) := rfl

/-- **T6 on the result of a generic call.** Two runs that differ in a secret return, when both
return, equal values of `pick[public Int]`: the relation at the instance type `pickInt`, from
`t6_ground` alone, makes them equal for a public observer, without computing either run. -/
theorem pickPublic_result_equal (w : World) {v₁ v₂ : Val}
    (h₁ : (eval 5 Examples.prog w sEnv₁ Examples.callPick).2 = .ok v₁)
    (h₂ : (eval 5 Examples.prog w sEnv₂ Examples.callPick).2 = .ok v₂) : v₁ = v₂ := by
  have hsec : ¬ Vis Examples.prog Label.bot sec := by
    rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;>
      simp [Examples.prog, Program.top, sec, LExpr.ofLabel, Label.bot] at h <;> revert h <;> decide
  have h := t6_ground (e₁ := sEnv₁) (e₂ := sEnv₂) Examples.prog_wf rfl (LExpr.le_refl _) w
    (fun q hq => by simp only [List.mem_cons, List.not_mem_nil, or_false] at hq
                    subst hq; rfl) rfl
    (.cons .int .nil) (.cons .int .nil) (.cons (.hidden hsec) .nil) callPick_typedΓ 5
    (by rw [h₁]; trivial) (by rw [h₂]; trivial)
  rw [h₁, h₂] at h
  have hv : V Examples.prog Label.bot (.int _) v₁ v₂ := h.2
  refine V.eq_int hv ?_
  simp [Vis, Program.le, LExpr.le, Label.le, LExpr.join, LExpr.inst, LExpr.dropPlain,
    LExpr.joinAll, LExpr.bot, LExpr.ofLabel, Label.bot, Label.join, Examples.lT, Level.join]
  exact Or.inr (Level.le_refl _)

end Una.F1.T6Example

