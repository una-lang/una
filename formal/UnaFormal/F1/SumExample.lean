/-
Non-vacuity of #512: **sum kinds and failures with a payload carry their fields' labels.**

The program has one payload kind, `Err = Oops(info: Int, key: internal Text) | Plain`, and
`boom(k)`, which fails with `Oops(1, k)`. The secret is the environment's `SECRET`, `internal`.

- **A secret payload field read back** (P2): `attempt boom(s)`, and a handler of `boom(s)`, then
  print the failure's `key`. The check rejects both, their runs from environments where `SECRET`
  is `"1"` and `"22"` print different lines, so T6 refutes every typing of them
  (`attempt_rejected`, `attempt_leaks`, `attempt_untyped`; `handle_*`).
- **A variant chosen by a secret** (P3): `attempt (if s == "1" then fail Plain else ())`, then
  print whether it failed. Rejected by T-Fail's public program counter, and it leaks
  (`choice_*`).
- **`case` on a secret sum, with a print in an arm** (S1): rejected, and it leaks (`case_*`).
- **The public twins are certified and run**: printing `info` after `attempt` and after a
  handler, a variant chosen by a public value, and an exhaustive `case`, whose last `else` is
  `unreachable`, which the run never reaches (`*_public_*`, `exhaustive_*`).
- **The relation on payloads** (`payload_related`, `payload_public_differs`): two failures of the
  same public variant whose secret fields differ are related for a `public` observer, and two
  whose public fields differ are not, when the observer sees what decides them.
-/
import UnaFormal.F1.Import

namespace Una.F1.SumExample

open Check

def labels : Import.HostLabels :=
  { acceptsReq := ⟨.internal, []⟩, reqResult := ⟨.internal, []⟩, fileState := some Label.bot }

def intl : LExpr := .ofLabel ⟨.internal, []⟩

/-- `Err = Oops(info: Int, key: internal Text) | Plain`. -/
def errSum : Sum := ⟨"m.Err", [[.int .bot, .text intl], []], ⟨.internal, []⟩⟩

/-- `Shape = Circle(r: Int) | Square`: a sum whose `case` below is exhaustive. -/
def shapeSum : Sum := ⟨"m.Shape", [[.int .bot], []], Label.bot⟩

/-- `boom(k: internal Text) -> Int fails Err = fail Oops(1, k)`. -/
def boom : Fun where
  name := "m.boom"
  params := [("k", .text intl)]
  result := .int .bot
  fails := true
  raises := ["m.Err"]
  body := .raise "m.Err" 0 [.int 1, .var "k"]

/-- `Console` accepts `public`. -/
def prog : Program := Import.mkProgram [] Label.bot labels [] [] [boom] [errSum, shapeSum]

def wSet (v : String) : World Host.unit := ⟨(), (), fun x => if x = "SECRET" then some v else none⟩

theorem internal_hidden : ¬ Vis prog Label.bot (.ofLabel internalLabel) := by
  rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;>
    simp [prog, Import.mkProgram, Program.top, internalLabel, LExpr.ofLabel, Label.bot] at h <;>
    revert h <;> decide

theorem set_rel (a b : String) : WorldRel prog Label.bot (wSet a) (wSet b) :=
  ⟨fun _ => rfl, fun h => absurd h internal_hidden, fun x => by simp only [wSet]; split <;> rfl⟩

/-- `print(to_text(r))` at `Console`, then `k`. -/
def printThen (r : String) (k : Expr) : Expr :=
  .letR "%m" (.text .bot) (.toText (.var r)) (.letR "%u" (.unit .bot) (.op (.var "%m")) k)

/-- `print(r)` of a text, then `k`. -/
def printText (r : String) (k : Expr) : Expr :=
  .letR "%u" (.unit .bot) (.op (.var r)) k

/-- `when Env.get("SECRET") is None -> () is Some(s) -> k`, with `s` an `internal Text`. -/
def withSecret (k : Expr) : Expr :=
  .letR "e" (.option (.text intl) .bot) (.envGet (.text "SECRET"))
    (.caseOption (.var "e") (.atom .unit) "s" k)

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

/-- `boom(s)`'s value. -/
def callBoom : Expr := .letR "v" (.int .bot) (.call "m.boom" [] [.var "s"]) (.atom (.var "v"))

def errTy : LTy := .record "m.Err" .bot

/-! ### P2: a secret payload field read back after `attempt` -/

/-- `let r = attempt boom(s)`; on `Err(Oops(i, key))`, print `key` (`leak`) or `i`. -/
def afterAttempt (leak : Bool) : Expr :=
  withSecret
    (.letE "r" (.result (.int .bot) errTy .bot) (.attemptK "m.Err" callBoom)
      (.caseResult (.var "r") "x" (.atom .unit) "y"
        (.caseSum (.var "y") 0 ["i", "key"]
          (if leak then printText "key" (.atom .unit) else printThen "i" (.atom .unit))
          (.atom .unit))))

unseal flowB flowsB in
theorem attempt_rejected :
    ∃ msg, checkExpr prog [] [] .bot (.unit .bot) (afterAttempt true) = .error msg := ⟨_, rfl⟩

theorem attempt_leaks : (eval Host.unit 20 prog (wSet "1") [] (afterAttempt true)).1 = [.out "1"] ∧
    (eval Host.unit 20 prog (wSet "22") [] (afterAttempt true)).1 = [.out "22"] := ⟨rfl, rfl⟩

theorem attempt_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot (afterAttempt true) t ε := by
  intro ht
  have h := same_lines ht 20 (by exact True.intro) (by exact True.intro)
  rw [attempt_leaks.1, attempt_leaks.2, proj_single_vis (out_vis _), proj_single_vis (out_vis _)] at h
  simp at h

unseal flowB flowsB in
theorem attempt_public_checked :
    ∃ ε, checkExpr prog [] [] .bot (.unit .bot) (afterAttempt false) = .ok ε := ⟨_, rfl⟩

/-- **Reading the public field is certified**, and it runs and prints it. -/
theorem attempt_public_runs :
    (eval Host.unit 20 prog (wSet "1") [] (afterAttempt false)).1 = [.out "1"] := rfl

/-! ### P2: a secret payload field read back by a handler -/

/-- `boom(s) if error e -> (when e is Oops(i, key) -> print key or i; 0) otherwise 0`. -/
def afterHandler (leak : Bool) : Expr :=
  withSecret
    (.letE "x" (.int .bot)
      (.handleK "m.Err" callBoom "f"
        (.caseSum (.var "f") 0 ["i", "key"]
          (if leak then printText "key" (.atom (.int 0)) else printThen "i" (.atom (.int 0)))
          (.atom (.int 0))))
      (.atom .unit))

unseal flowB flowsB in
theorem handle_rejected :
    ∃ msg, checkExpr prog [] [] .bot (.unit .bot) (afterHandler true) = .error msg := ⟨_, rfl⟩

theorem handle_leaks : (eval Host.unit 20 prog (wSet "1") [] (afterHandler true)).1 = [.out "1"] ∧
    (eval Host.unit 20 prog (wSet "22") [] (afterHandler true)).1 = [.out "22"] := ⟨rfl, rfl⟩

theorem handle_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot (afterHandler true) t ε := by
  intro ht
  have h := same_lines ht 20 (by exact True.intro) (by exact True.intro)
  rw [handle_leaks.1, handle_leaks.2, proj_single_vis (out_vis _), proj_single_vis (out_vis _)] at h
  simp at h

unseal flowB flowsB in
theorem handle_public_checked :
    ∃ ε, checkExpr prog [] [] .bot (.unit .bot) (afterHandler false) = .ok ε := ⟨_, rfl⟩

theorem handle_public_runs :
    (eval Host.unit 20 prog (wSet "1") [] (afterHandler false)).1 = [.out "1"] := rfl

/-! ### P3: a variant chosen by a secret -/

/-- `let r = attempt (if c then fail Plain else ())`; print `"failed"` on `Err`. The condition
is `s == "1"` (`secret`) or `true`. -/
def choice (secret : Bool) : Expr :=
  withSecret
    (.letR "c" (.bool (if secret then intl else .bot))
      (if secret then .eq (.var "s") (.text "1") else .atom (.bool true))
      (.letE "r" (.result (.unit .bot) errTy .bot)
        (.attemptK "m.Err" (.ite (.var "c") (.raise "m.Err" 1 []) (.atom .unit)))
        (.caseResult (.var "r") "x" (.atom .unit) "y"
          (.letR "t" (.text .bot) (.atom (.text "failed")) (printText "t" (.atom .unit))))))

unseal flowB flowsB in
theorem choice_rejected :
    ∃ msg, checkExpr prog [] [] .bot (.unit .bot) (choice true) = .error msg := ⟨_, rfl⟩

theorem choice_leaks : (eval Host.unit 20 prog (wSet "1") [] (choice true)).1 = [.out "failed"] ∧
    (eval Host.unit 20 prog (wSet "22") [] (choice true)).1 = [] := ⟨rfl, rfl⟩

theorem choice_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot (choice true) t ε := by
  intro ht
  have h := same_lines ht 20 (by exact True.intro) (by exact True.intro)
  rw [choice_leaks.1, choice_leaks.2, proj_single_vis (out_vis _)] at h
  simp [proj] at h

unseal flowB flowsB in
theorem choice_public_checked :
    ∃ ε, checkExpr prog [] [] .bot (.unit .bot) (choice false) = .ok ε := ⟨_, rfl⟩

theorem choice_public_runs :
    (eval Host.unit 20 prog (wSet "1") [] (choice false)).1 = [.out "failed"] := rfl

/-! ### S1: `case` on a secret sum, with a print in an arm -/

/-- `let w = if c then Circle(1) else Square`, then `when w is Square -> print "square"`, with
the arms exhaustive: `Circle(_) -> ()`, and `unreachable` last. -/
def onShape (secret : Bool) : Expr :=
  withSecret
    (.letR "c" (.bool (if secret then intl else .bot))
      (if secret then .eq (.var "s") (.text "1") else .atom (.bool false))
      (.letE "w" (.record "m.Shape" (if secret then intl else .bot))
        (.ite (.var "c")
          (.letR "w0" (.record "m.Shape" .bot) (.variant "m.Shape" 0 [.int 1]) (.atom (.var "w0")))
          (.letR "w1" (.record "m.Shape" .bot) (.variant "m.Shape" 1 []) (.atom (.var "w1"))))
        (.caseSum (.var "w") 0 ["%unread"] (.atom .unit)
          (.caseSum (.var "w") 1 []
            (.letR "t" (.text .bot) (.atom (.text "square")) (printText "t" (.atom .unit)))
            .unreachable))))

unseal flowB flowsB in
theorem case_rejected :
    ∃ msg, checkExpr prog [] [] .bot (.unit .bot) (onShape true) = .error msg := ⟨_, rfl⟩

theorem case_leaks : (eval Host.unit 20 prog (wSet "1") [] (onShape true)).1 = [] ∧
    (eval Host.unit 20 prog (wSet "22") [] (onShape true)).1 = [.out "square"] := ⟨rfl, rfl⟩

theorem case_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot (onShape true) t ε := by
  intro ht
  have h := same_lines ht 20 (by exact True.intro) (by exact True.intro)
  rw [case_leaks.1, case_leaks.2, proj_single_vis (out_vis _)] at h
  simp [proj] at h

unseal flowB flowsB in
theorem exhaustive_public_checked :
    ∃ ε, checkExpr prog [] [] .bot (.unit .bot) (onShape false) = .ok ε := ⟨_, rfl⟩

/-- **An exhaustive `case` runs to a value**: its `unreachable` is not reached. -/
theorem exhaustive_runs :
    eval Host.unit 20 prog (wSet "1") [] (onShape false) =
      ([.out "square"], .ok .unit, wSet "1") := rfl

/-! ### The relation on payloads -/

/-- **Two failures of the same public variant whose secret fields differ are related**, whatever
the observer sees of what decides them. -/
theorem payload_related (R : Val → Val → Prop) (vf : Prop) :
    OutRel prog Label.bot R vf (.raised "m.Err" 0 [.int 1, .text "a"])
      (.raised "m.Err" 0 [.int 1, .text "b"]) := fun _ =>
  ⟨rfl, rfl, errSum, _, rfl, rfl,
    .cons .int (.cons (.hidden (by simpa [LTy.top] using internal_hidden)) .nil)⟩

/-- **Two whose public fields differ are not**, when the observer sees what decides them. -/
theorem payload_public_differs (R : Val → Val → Prop) :
    ¬ OutRel prog Label.bot R True (.raised "m.Err" 0 [.int 1, .text "a"])
      (.raised "m.Err" 0 [.int 2, .text "a"]) := by
  intro h
  obtain ⟨_, _, sm, fs, hs, hv, hl⟩ := h trivial
  simp [prog, Import.mkProgram, Program.sum, List.find?, errSum, shapeSum] at hs
  subst hs
  simp [errSum] at hv
  subst hv
  cases hl with
  | cons h₁ _ =>
    cases h₁ with
    | hidden hn => exact hn (LExpr.bot_le _)

end Una.F1.SumExample
