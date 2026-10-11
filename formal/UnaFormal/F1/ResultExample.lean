/-
Non-vacuity of #508: **a result's error type carries labels.** `Ok(a)` and `Err(a)` build results
directly, and `to_text` and `==` read the whole value, so the labels of both positions count.
S-04's `result_ok` and `result_err` shapes (`docs/reviews/formal-m2-replay`) are these:

- **An `Err` that holds a secret** (`err_rejected`, `err_leaks`, `err_untyped`): a program prints
  `to_text(Err(s))` at `Console`, which accepts `public`, with `s` read from the environment. The
  check rejects it, and its runs from environments where `SECRET` is `"1"` and `"2"` print
  different lines, so T6 refutes every typing of it.
- **An `Ok` that holds a secret** (`ok_rejected`, `ok_leaks`, `ok_untyped`): the same at the
  value's position.
- **The tag decided by a secret** (`tag_rejected`, `tag_leaks`, `tag_untyped`): `Ok(1)` or
  `Err(2)`, chosen by a condition on `s`, then printed. Neither payload is secret; which
  constructor is built is, and the branch's rule raises the result by it.
- **A public payload is certified** (`public_runs`): the same shape with a constant payload is
  accepted and prints.
- **The relation hides an `internal` payload** (`err_rel_hidden`): two `Err` values whose payloads
  differ are related at an `internal` error type, and their texts differ.
-/
import UnaFormal.F1.Import

namespace Una.F1.ResultExample

open Check

def labels : Import.HostLabels :=
  { acceptsReq := ⟨.internal, []⟩, reqResult := ⟨.internal, []⟩, fileState := some Label.bot }

/-- `Console` accepts `public`. -/
def prog : Program := Import.mkProgram [] Label.bot labels [] [] []

def intl : LExpr := .ofLabel ⟨.internal, []⟩

/-- A world on the stateless host whose environment sets `SECRET` to `v`. -/
def wSet (v : String) : World Host.unit := ⟨(), (), fun x => if x = "SECRET" then some v else none⟩

theorem internal_hidden : ¬ Vis prog Label.bot (.ofLabel internalLabel) := by
  rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;>
    simp [prog, Import.mkProgram, Program.top, internalLabel, LExpr.ofLabel, Label.bot] at h <;>
    revert h <;> decide

/-- Two environments that differ in `SECRET`'s value are related for a `public` observer. -/
theorem set_rel (a b : String) : WorldRel prog Label.bot (wSet a) (wSet b) :=
  ⟨fun _ => rfl, fun h => absurd h internal_hidden, fun x => by simp only [wSet]; split <;> rfl⟩

/-- `print(to_text(r))` at `Console`. -/
def printR (r : String) : Expr :=
  .letR "m" (.text .bot) (.toText (.var r))
    (.letR "u" (.unit .bot) (.op (.var "m")) (.atom (.var "u")))

/-- `when Env.get("SECRET") is None -> () is Some(s) -> k`, with `s` an `internal Text`. -/
def withSecret (k : Expr) : Expr :=
  .letR "e" (.option (.text intl) .bot) (.envGet (.text "SECRET"))
    (.caseOption (.var "e") (.atom .unit) "s" k)

unseal flowB flowsB in
theorem certified : prog.LWellFormed ∧ prog.B1WellFormed ∧ prog.HostWF :=
  progOk_sound_nil (by rfl)

theorem out_vis (s : String) : EvVis prog Label.bot (.out s) := LExpr.le_refl _

/-- T6 over `set_rel`: a typed program's two runs show a `public` observer the same lines. -/
theorem same_lines {e : Expr} {t : LTy} {ε : Eff} (ht : LExprTy prog [] [] .bot e t ε) (n : Nat)
    (h₁ : Term (eval Host.unit n prog (wSet "1") [] e).2.1)
    (h₂ : Term (eval Host.unit n prog (wSet "2") [] e).2.1) :
    proj prog Label.bot (eval Host.unit n prog (wSet "1") [] e).1 =
      proj prog Label.bot (eval Host.unit n prog (wSet "2") [] e).1 := by
  obtain ⟨hwf, _, hh⟩ := certified
  exact (t6_host_ground (Γ := []) hwf rfl hh Host.unit_lawful (fun _ h => nomatch h) rfl
    (set_rel "1" "2") .nil .nil .nil ht n h₁ h₂).1

/-! ### `Err(s)`: S-04's `result_err` -/

/-- `let r: Result[Int, internal Text] = Err(s)` and print it. -/
def errLeak : Expr :=
  withSecret (.letR "r" (.result (.int .bot) (.text intl) .bot) (.mkErr (.var "s")) (printR "r"))

unseal flowB flowsB in
theorem err_rejected : ∃ msg, checkExpr prog [] [] .bot (.unit .bot) errLeak = .error msg :=
  ⟨_, rfl⟩

theorem err_leaks : (eval Host.unit 10 prog (wSet "1") [] errLeak).1 = [.out "Err(1)"] ∧
    (eval Host.unit 10 prog (wSet "2") [] errLeak).1 = [.out "Err(2)"] := ⟨rfl, rfl⟩

/-- **The leak has no typing.** -/
theorem err_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot errLeak t ε := by
  intro ht
  have h := same_lines ht 10 (by exact True.intro) (by exact True.intro)
  rw [err_leaks.1, err_leaks.2, proj_single_vis (out_vis _), proj_single_vis (out_vis _)] at h
  simp at h

/-! ### `Ok(s)`: S-04's `result_ok` -/

/-- `let r: Result[internal Text, Int] = Ok(s)` and print it. -/
def okLeak : Expr :=
  withSecret (.letR "r" (.result (.text intl) (.int .bot) .bot) (.mkOk (.var "s")) (printR "r"))

unseal flowB flowsB in
theorem ok_rejected : ∃ msg, checkExpr prog [] [] .bot (.unit .bot) okLeak = .error msg :=
  ⟨_, rfl⟩

theorem ok_leaks : (eval Host.unit 10 prog (wSet "1") [] okLeak).1 = [.out "Ok(1)"] ∧
    (eval Host.unit 10 prog (wSet "2") [] okLeak).1 = [.out "Ok(2)"] := ⟨rfl, rfl⟩

theorem ok_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot okLeak t ε := by
  intro ht
  have h := same_lines ht 10 (by exact True.intro) (by exact True.intro)
  rw [ok_leaks.1, ok_leaks.2, proj_single_vis (out_vis _), proj_single_vis (out_vis _)] at h
  simp at h

/-! ### The tag decided by a secret -/

/-- `let r: internal Result[Int, Int] = if s == "1" then Ok(1) else Err(2)` and print it. Both
payloads are public constants; the condition decides the constructor. -/
def tagLeak : Expr :=
  withSecret
    (.letR "b" (.bool intl) (.eq (.var "s") (.text "1"))
      (.letE "r" (.result (.int .bot) (.int .bot) intl)
        (.ite (.var "b")
          (.letR "o" (.result (.int .bot) (.int .bot) .bot) (.mkOk (.int 1)) (.atom (.var "o")))
          (.letR "o" (.result (.int .bot) (.int .bot) .bot) (.mkErr (.int 2)) (.atom (.var "o"))))
        (printR "r")))

unseal flowB flowsB in
theorem tag_rejected : ∃ msg, checkExpr prog [] [] .bot (.unit .bot) tagLeak = .error msg :=
  ⟨_, rfl⟩

theorem tag_leaks : (eval Host.unit 10 prog (wSet "1") [] tagLeak).1 = [.out "Ok(1)"] ∧
    (eval Host.unit 10 prog (wSet "2") [] tagLeak).1 = [.out "Err(2)"] := ⟨rfl, rfl⟩

theorem tag_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot tagLeak t ε := by
  intro ht
  have h := same_lines ht 10 (by exact True.intro) (by exact True.intro)
  rw [tag_leaks.1, tag_leaks.2, proj_single_vis (out_vis _), proj_single_vis (out_vis _)] at h
  simp at h

/-! ### A public payload -/

/-- `let r: Result[Int, Text] = Err("x")` and print it, after reading the secret. -/
def public_ : Expr :=
  withSecret (.letR "r" (.result (.int .bot) (.text .bot) .bot) (.mkErr (.text "x")) (printR "r"))

unseal flowB flowsB in
theorem public_checked : ∃ ε, checkExpr prog [] [] .bot (.unit .bot) public_ = .ok ε := ⟨_, rfl⟩

/-- **A public payload is certified**, and it runs and prints. -/
theorem public_runs : (∃ t ε, LExprTy prog [] [] .bot public_ t ε ∧ ∀ n : Nat,
    Term (eval Host.unit n prog (wSet "1") [] public_).2.1 →
    Term (eval Host.unit n prog (wSet "2") [] public_).2.1 →
    proj prog Label.bot (eval Host.unit n prog (wSet "1") [] public_).1 =
      proj prog Label.bot (eval Host.unit n prog (wSet "2") [] public_).1) ∧
    (eval Host.unit 10 prog (wSet "1") [] public_).1 = [.out "Err(x)"] := by
  obtain ⟨ε, hc⟩ := public_checked
  obtain ⟨t, ht, _⟩ := checkExpr_sound hc
  exact ⟨⟨t, ε, ht, same_lines ht⟩, rfl⟩

/-! ### The relation -/

/-- **An `internal` payload is hidden**: two `Err` values that differ are related at
`Result[Int, internal Text]`, while their texts differ. -/
theorem err_rel_hidden :
    V prog Label.bot (.result (.int .bot) (.text intl) .bot) (.err (.text "1")) (.err (.text "2")) ∧
      (Val.err (.text "1")).render ≠ (Val.err (.text "2")).render :=
  ⟨.err (.hidden (fun h => internal_hidden (by simpa [LTy.top, intl, internalLabel] using h))),
    by decide⟩

end Una.F1.ResultExample
