/-
Non-vacuity of #524: **an operation's handler runs under the effect floor, and its failure reaches
the caller at the caller's program counter.**

The program has one operation with a handler, `Door.open(code: Int) -> Int fails Shut`, which
accepts `internal`; its handler, `m.door.open`, fails. The secret is the environment's `SECRET`,
`internal`, and the flag `c` is `SECRET == "1"`.

- **H5, handled** (`handled`): `if c then attempt @door.open(1)`, then a public print: certified,
  and it runs. The failure, decided at `c`'s label, never leaves the branch.
- **H5, unhandled** (`unhandled`): `if c then @door.open(1)`, the failure not handled in the branch,
  then a public print: rejected, it leaks (the print happens only when `c` is false), and it has no
  typing. The failure is labeled `pc`, and T-Let runs the print under it.
- **H6, print-after** and **value-out**: rejected, they leak, no typing.
- **The effect floor** (`progE`, `progC`): a handler that prints at `Console`, which accepts
  `public`, below the operation's `internal`, is not lawful, directly or through a function it
  calls; and the program would leak through it.
-/
import UnaFormal.F1.Import

namespace Una.F1.HandlerExample

open Check

def labels : Import.HostLabels :=
  { acceptsReq := ⟨.internal, []⟩, reqResult := ⟨.internal, []⟩, fileState := some Label.bot }

def intl : LExpr := .ofLabel ⟨.internal, []⟩

/-- The handler of `Door.open`: its parameter raised by `accepts internal`, failing with `Closed`. -/
def door : Fun where
  name := "m.door.open"
  tparams := []
  eqParams := []
  params := [("code", .int intl)]
  op := false
  host := HostFx.none
  result := .int .bot
  fails := true
  body := .fail 0

def ops : List (String × Label) := [("m.door.open", ⟨.internal, []⟩)]

/-- `Console` accepts `public`. -/
def prog : Program := Import.mkProgram [] Label.bot labels [] [] [door] [] ops

def wSet (v : String) : World Host.unit := ⟨(), (), fun x => if x = "SECRET" then some v else none⟩

theorem internal_hidden : ¬ Vis prog Label.bot (.ofLabel internalLabel) := by
  rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;>
    simp [prog, Import.mkProgram, Program.top, internalLabel, LExpr.ofLabel, Label.bot] at h <;>
    revert h <;> decide

theorem set_rel (a b : String) : WorldRel prog Label.bot (wSet a) (wSet b) :=
  ⟨fun _ => rfl, fun h => absurd h internal_hidden, fun x => by simp only [wSet]; split <;> rfl⟩

/-- `print("t")`, then `k`. -/
def printLit (t : String) (k : Expr) : Expr :=
  .letR "%t" (.text .bot) (.atom (.text t)) (.letR "%u" (.unit .bot) (.op (.var "%t")) k)

/-- `print(to_text(r))`, then `k`. -/
def printThen (r : String) (k : Expr) : Expr :=
  .letR "%m" (.text .bot) (.toText (.var r)) (.letR "%u" (.unit .bot) (.op (.var "%m")) k)

/-- `when Env.get("SECRET") is None -> () is Some(s) -> let c = s == "1" in k`. -/
def withFlag (k : Expr) : Expr :=
  .letR "e" (.option (.text intl) .bot) (.envGet (.text "SECRET"))
    (.caseOption (.var "e") (.atom .unit) "s"
      (.letR "c" (.bool intl) (.eq (.var "s") (.text "1")) k))

/-- `@door.open(1)`. -/
def openDoor : Expr := .letR "v" (.int .bot) (.invoke "m.door.open" [.int 1]) (.atom (.var "v"))

def resTy : LTy := .result (.int .bot) (.int .bot) intl

unseal flowB flowsB in
theorem certified : prog.LWellFormed ∧ prog.B1WellFormed ∧ prog.HostWF :=
  progOk_sound_nil (by rfl)

theorem out_vis (s : String) : EvVis prog Label.bot (.out s) := LExpr.le_refl _

theorem same_lines {e : Expr} {t : LTy} {ε : Eff} (ht : LExprTy prog [] [] .bot e t ε) (n : Nat)
    (h₁ : Term (eval Host.unit n prog (wSet "1") [] e).2.1)
    (h₂ : Term (eval Host.unit n prog (wSet "22") [] e).2.1) :
    proj prog Label.bot (eval Host.unit n prog (wSet "1") [] e).1 =
      proj prog Label.bot (eval Host.unit n prog (wSet "22") [] e).1 := by
  obtain ⟨hwf, _, hh⟩ := certified
  exact (t6_host_ground (Γ := []) hwf rfl hh Host.unit_lawful (fun _ h => nomatch h) rfl
    (set_rel "1" "22") .nil .nil .nil ht n h₁ h₂).1

/-! ### H5, handled under the secret -/

/-- `if c then attempt @door.open(1) else ()`, then `print("after")`. -/
def handled : Expr :=
  withFlag (.letE "b" (.unit .bot)
    (.ite (.var "c") (.letE "r" resTy (.attempt openDoor) (.atom .unit)) (.atom .unit))
    (printLit "after" (.atom .unit)))

unseal flowB flowsB in
theorem handled_checked :
    ∃ ε, checkExpr prog [] [] .bot (.unit .bot) handled = .ok ε := ⟨_, rfl⟩

/-- **A failure handled in the branch is certified**, and both runs print. -/
theorem handled_runs : (eval Host.unit 20 prog (wSet "1") [] handled).1 = [.out "after"] ∧
    (eval Host.unit 20 prog (wSet "22") [] handled).1 = [.out "after"] := ⟨rfl, rfl⟩

/-! ### H5, unhandled under the secret -/

/-- `if c then @door.open(1) else ()`, the failure not handled, then `print("after")`. -/
def unhandled : Expr :=
  withFlag (.letE "b" (.unit .bot)
    (.ite (.var "c") (.letR "w" (.int .bot) (.invoke "m.door.open" [.int 1]) (.atom .unit))
      (.atom .unit))
    (printLit "after" (.atom .unit)))

unseal flowB flowsB in
theorem unhandled_rejected :
    ∃ msg, checkExpr prog [] [] .bot (.unit .bot) unhandled = .error msg := ⟨_, rfl⟩

theorem unhandled_leaks : (eval Host.unit 20 prog (wSet "1") [] unhandled).1 = [] ∧
    (eval Host.unit 20 prog (wSet "22") [] unhandled).1 = [.out "after"] := ⟨rfl, rfl⟩

theorem unhandled_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot unhandled t ε := by
  intro ht
  have h := same_lines ht 20 (by exact True.intro) (by exact True.intro)
  rw [unhandled_leaks.1, unhandled_leaks.2, proj_single_vis (out_vis _)] at h
  simp [proj] at h

/-! ### H6 -/

/-- `if c then (attempt @door.open(1); print("after")) else ()`. -/
def printAfter : Expr :=
  withFlag (.ite (.var "c") (.letE "r" resTy (.attempt openDoor) (printLit "after" (.atom .unit)))
    (.atom .unit))

unseal flowB flowsB in
theorem printAfter_rejected :
    ∃ msg, checkExpr prog [] [] .bot (.unit .bot) printAfter = .error msg := ⟨_, rfl⟩

theorem printAfter_leaks : (eval Host.unit 20 prog (wSet "1") [] printAfter).1 = [.out "after"] ∧
    (eval Host.unit 20 prog (wSet "22") [] printAfter).1 = [] := ⟨rfl, rfl⟩

theorem printAfter_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot printAfter t ε := by
  intro ht
  have h := same_lines ht 20 (by exact True.intro) (by exact True.intro)
  rw [printAfter_leaks.1, printAfter_leaks.2, proj_single_vis (out_vis _)] at h
  simp [proj] at h

/-- `let x: Int = if c then (when attempt @door.open(1) is Ok(n) -> n is Err -> 0) else 5`, printed. -/
def valueOut : Expr :=
  withFlag (.letE "x" (.int .bot)
    (.ite (.var "c")
      (.letE "r" resTy (.attempt openDoor)
        (.caseResult (.var "r") "n" (.atom (.var "n")) "%e" (.atom (.int 0))))
      (.atom (.int 5)))
    (printThen "x" (.atom .unit)))

unseal flowB flowsB in
theorem valueOut_rejected :
    ∃ msg, checkExpr prog [] [] .bot (.unit .bot) valueOut = .error msg := ⟨_, rfl⟩

theorem valueOut_leaks : (eval Host.unit 20 prog (wSet "1") [] valueOut).1 = [.out "0"] ∧
    (eval Host.unit 20 prog (wSet "22") [] valueOut).1 = [.out "5"] := ⟨rfl, rfl⟩

theorem valueOut_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot valueOut t ε := by
  intro ht
  have h := same_lines ht 20 (by exact True.intro) (by exact True.intro)
  rw [valueOut_leaks.1, valueOut_leaks.2, proj_single_vis (out_vis _), proj_single_vis (out_vis _)] at h
  simp at h

/-! ### The effect floor -/

/-- A handler that prints at `Console` before it fails: an effect below its `internal` floor. -/
def doorE : Fun := { door with op := true, body := printLit "opened" (.fail 0) }

def progE : Program := Import.mkProgram [] Label.bot labels [] [] [doorE] [] ops

/-- `shout()` prints. -/
def shout : Fun where
  name := "m.shout"
  tparams := []
  eqParams := []
  params := []
  op := true
  host := HostFx.none
  result := .unit .bot
  fails := false
  body := printLit "shout" (.atom .unit)

/-- A handler whose only effect below its floor is inside a function it calls. -/
def doorC : Fun :=
  { door with op := true, body := .letR "u" (.unit .bot) (.call "m.shout" [] []) (.fail 0) }

def progC : Program := Import.mkProgram [] Label.bot labels [] [] [doorC, shout] [] ops

unseal flowB flowsB in
/-- **A handler that prints below its floor is not lawful**, and the whole-program check refuses
it. -/
theorem floor_direct : progOk [] progE = false := by rfl

unseal flowB flowsB in
/-- **The floor holds through calls**: a handler whose print is inside a function it calls must
declare the effect (T-Fun), and then fails the floor. -/
theorem floor_call : progOk [] progC = false := by rfl

/-- What the floor prevents: under the secret, the handler's print is seen only when `c` holds. -/
theorem floor_leaks : (eval Host.unit 20 progE (wSet "1") [] handled).1 = [.out "opened", .out "after"] ∧
    (eval Host.unit 20 progE (wSet "22") [] handled).1 = [.out "after"] := ⟨rfl, rfl⟩

end Una.F1.HandlerExample
