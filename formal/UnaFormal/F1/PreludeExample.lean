/-
Non-vacuity of #542: **`abs`, `min`, `max`, `or_else` and `slice` carry every label of their
arguments, and `is_ok` the top label of its result only** (`spec/14 §1`).

- **`min` of a secret** (`min_rejected`, `min_leaks`, `min_untyped`): `min(len(s), 100)` printed at
  `Console`, which accepts `public`, with `s` read from the environment, so `internal`. The check
  rejects it, its runs from environments where `SECRET` is `"1"` and `"22"` print different lines,
  and T6 refutes every typing of it.
- **`or_else` of a secret option** (`orElse_rejected`, `orElse_leaks`, `orElse_untyped`):
  `or_else(Env.get("SECRET"), "none")` printed, the same.
- **`is_ok` is structural** (`isOk_checked`, `isOk_runs`): `is_ok(Ok(len(s)))`, a public result
  of an `internal Int`, decides a public print, which is certified. With the result's top label
  `internal` (`isOkTop_rejected`, `isOkTop_leaks`, `isOkTop_untyped`), because a secret comparison
  chose `Ok` or `Err`, it is rejected, it leaks, and it has no typing.
- **Public arguments are certified** (`public_checked`, `public_runs`): `abs`, `min`, `max`,
  `or_else` and `slice` of public values, printed.
- **`slice` outside its precondition aborts** (`slice_out_of_range_aborts`): as a zero divisor
  (#526), an abort is outside T6's termination-insensitive premise; the precondition is an
  obligation at each call (`spec/12 §3`).
-/
import UnaFormal.F1.Import

namespace Una.F1.PreludeExample

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

/-- `print(m)` at `Console`, for a text atom `m`. -/
def say (m : Atom) : Expr := .letR "u" (.unit .bot) (.op m) (.atom (.var "u"))

/-- `print(to_text(r))` at `Console`. -/
def printR (r : String) : Expr :=
  .letR "m" (.text .bot) (.toText (.var r)) (say (.var "m"))

/-- `let e = Env.get("SECRET")`, an `Option` of an `internal Text`, then `k`. -/
def withEnv (k : Expr) : Expr :=
  .letR "e" (.option (.text intl) .bot) (.envGet (.text "SECRET")) k

/-- `when e is None -> () is Some(s) -> let l = len(s); k`, with `l` an `internal Int`. -/
def withLen (k : Expr) : Expr :=
  withEnv (.caseOption (.var "e") (.atom .unit) "s"
    (.letR "l" (.int intl) (.prim .len [.var "s"]) k))

unseal flowB flowsB in
theorem certified : prog.LWellFormed ∧ prog.B1WellFormed ∧ prog.HostWF :=
  progOk_sound_nil (by rfl)

theorem out_vis (s : String) : EvVis prog Label.bot (.out s) := LExpr.le_refl _

/-- T6 over `set_rel`: a typed program's two runs show a `public` observer the same lines. -/
theorem same_lines {e : Expr} {t : LTy} {ε : Eff} (ht : LExprTy prog [] [] .bot e t ε) (n : Nat)
    (h₁ : Term (eval Host.unit n prog (wSet "1") [] e).2.1)
    (h₂ : Term (eval Host.unit n prog (wSet "22") [] e).2.1) :
    proj prog Label.bot (eval Host.unit n prog (wSet "1") [] e).1 =
      proj prog Label.bot (eval Host.unit n prog (wSet "22") [] e).1 := by
  obtain ⟨hwf, _, hh⟩ := certified
  exact (t6_host_ground (Γ := []) hwf rfl hh Host.unit_lawful (fun _ h => nomatch h) rfl
    (set_rel "1" "22") .nil .nil .nil ht n h₁ h₂).1

/-- Two runs that print different single lines refute every typing. -/
theorem untyped_of_lines {e : Expr} {a b : String} (hab : a ≠ b)
    (h₁ : (eval Host.unit 30 prog (wSet "1") [] e).1 = [.out a])
    (h₂ : (eval Host.unit 30 prog (wSet "22") [] e).1 = [.out b])
    (t₁ : Term (eval Host.unit 30 prog (wSet "1") [] e).2.1)
    (t₂ : Term (eval Host.unit 30 prog (wSet "22") [] e).2.1)
    {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot e t ε := by
  intro ht
  have h := same_lines ht 30 t₁ t₂
  rw [h₁, h₂, proj_single_vis (out_vis _), proj_single_vis (out_vis _)] at h
  exact hab (by simpa using h)

/-! ### `min` of a secret -/

/-- `let k: Int = min(l, 100)` and print it. -/
def minLeak : Expr :=
  withLen (.letR "k" (.int .bot) (.prim .min [.var "l", .int 100]) (printR "k"))

unseal flowB flowsB in
theorem min_rejected : ∃ msg, checkExpr prog [] [] .bot (.unit .bot) minLeak = .error msg :=
  ⟨_, rfl⟩

theorem min_leaks : (eval Host.unit 30 prog (wSet "1") [] minLeak).1 = [.out "1"] ∧
    (eval Host.unit 30 prog (wSet "22") [] minLeak).1 = [.out "2"] := ⟨rfl, rfl⟩

/-- **The leak has no typing.** -/
theorem min_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot minLeak t ε :=
  untyped_of_lines (by decide) min_leaks.1 min_leaks.2 (by exact True.intro) (by exact True.intro)

/-! ### `or_else` of a secret option -/

/-- `let k: Text = or_else(e, "none")` and print it. -/
def orElseLeak : Expr :=
  withEnv (.letR "k" (.text .bot) (.orElse (.var "e") (.text "none")) (say (.var "k")))

unseal flowB flowsB in
theorem orElse_rejected :
    ∃ msg, checkExpr prog [] [] .bot (.unit .bot) orElseLeak = .error msg := ⟨_, rfl⟩

theorem orElse_leaks : (eval Host.unit 30 prog (wSet "1") [] orElseLeak).1 = [.out "1"] ∧
    (eval Host.unit 30 prog (wSet "22") [] orElseLeak).1 = [.out "22"] := ⟨rfl, rfl⟩

theorem orElse_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot orElseLeak t ε :=
  untyped_of_lines (by decide) orElse_leaks.1 orElse_leaks.2 (by exact True.intro)
    (by exact True.intro)

/-! ### `is_ok` is structural -/

/-- `if b then print("ok") else print("err")`. -/
def sayIf (b : String) : Expr := .ite (.var b) (say (.text "ok")) (say (.text "err"))

/-- `let r: Result[internal Int, Int] = Ok(l); let b: Bool = is_ok(r)`, and print by `b`. -/
def isOkPublic : Expr :=
  withLen (.letR "r" (.result (.int intl) (.int .bot) .bot) (.mkOk (.var "l"))
    (.letR "b" (.bool .bot) (.isOk (.var "r")) (sayIf "b")))

unseal flowB flowsB in
theorem isOk_checked :
    ∃ ε, checkExpr prog [] [] .bot (.unit .bot) isOkPublic = .ok ε := ⟨_, rfl⟩

/-- **`is_ok` of a public result of a secret is certified**, and it runs and prints. -/
theorem isOk_runs : (∃ t ε, LExprTy prog [] [] .bot isOkPublic t ε ∧ ∀ n : Nat,
    Term (eval Host.unit n prog (wSet "1") [] isOkPublic).2.1 →
    Term (eval Host.unit n prog (wSet "22") [] isOkPublic).2.1 →
    proj prog Label.bot (eval Host.unit n prog (wSet "1") [] isOkPublic).1 =
      proj prog Label.bot (eval Host.unit n prog (wSet "22") [] isOkPublic).1) ∧
    (eval Host.unit 30 prog (wSet "1") [] isOkPublic).1 = [.out "ok"] := by
  obtain ⟨ε, hc⟩ := isOk_checked
  obtain ⟨t, ht, _⟩ := checkExpr_sound hc
  exact ⟨⟨t, ε, ht, same_lines ht⟩, rfl⟩

/-- `Ok(1)` or `Err(0)` chosen by `l < 2`, so the result's top label is `internal`. -/
def isOkTop : Expr :=
  withLen (.letR "c" (.bool intl) (.prim .lt [.var "l", .int 2])
    (.letE "r" (.result (.int .bot) (.int .bot) intl)
      (.ite (.var "c")
        (.letR "o" (.result (.int .bot) (.int .bot) intl) (.mkOk (.int 1)) (.atom (.var "o")))
        (.letR "o" (.result (.int .bot) (.int .bot) intl) (.mkErr (.int 0)) (.atom (.var "o"))))
      (.letR "b" (.bool .bot) (.isOk (.var "r")) (sayIf "b"))))

unseal flowB flowsB in
theorem isOkTop_rejected : ∃ msg, checkExpr prog [] [] .bot (.unit .bot) isOkTop = .error msg :=
  ⟨_, rfl⟩

theorem isOkTop_leaks : (eval Host.unit 30 prog (wSet "1") [] isOkTop).1 = [.out "ok"] ∧
    (eval Host.unit 30 prog (wSet "22") [] isOkTop).1 = [.out "err"] := ⟨rfl, rfl⟩

theorem isOkTop_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot isOkTop t ε :=
  untyped_of_lines (by decide) isOkTop_leaks.1 isOkTop_leaks.2 (by exact True.intro)
    (by exact True.intro)

/-! ### Public arguments -/

/-- `print(to_text(r))`, then `k`. -/
def printThen (r : String) (k : Expr) : Expr :=
  .letR ("m" ++ r) (.text .bot) (.toText (.var r))
    (.letR ("u" ++ r) (.unit .bot) (.op (.var ("m" ++ r))) k)

/-- `abs(-5)`, `min(3, 4)`, `max(3, 4)`, `or_else(None, 7)` and `slice("hello", 1, 3)`, printed. -/
def publicAll : Expr :=
  .letR "a" (.int .bot) (.prim .abs [.int (-5)]) (printThen "a"
    (.letR "n" (.int .bot) (.prim .min [.int 3, .int 4]) (printThen "n"
      (.letR "x" (.int .bot) (.prim .max [.int 3, .int 4]) (printThen "x"
        (.letR "o" (.option (.int .bot) .bot) .none
          (.letR "d" (.int .bot) (.orElse (.var "o") (.int 7)) (printThen "d"
            (.letR "t" (.text .bot) (.prim .slice [.text "hello", .int 1, .int 3])
              (say (.var "t")))))))))))

unseal flowB flowsB in
theorem public_checked :
    ∃ ε, checkExpr prog [] [] .bot (.unit .bot) publicAll = .ok ε := ⟨_, rfl⟩

/-- **Public arguments are certified**, and the run prints `5`, `3`, `4`, `7` and `el`. -/
theorem public_runs : (∃ t ε, LExprTy prog [] [] .bot publicAll t ε) ∧
    (eval Host.unit 40 prog (wSet "1") [] publicAll).1 =
      [.out "5", .out "3", .out "4", .out "7", .out "el"] := by
  obtain ⟨ε, hc⟩ := public_checked
  obtain ⟨t, ht, _⟩ := checkExpr_sound hc
  exact ⟨⟨t, ε, ht⟩, rfl⟩

/-! ### `slice` outside its precondition -/

/-- `let t: Text = slice("abc", 2, 1)`, then `t`. -/
def sliceBad : Expr :=
  .letR "t" (.text .bot) (.prim .slice [.text "abc", .int 2, .int 1]) (.atom (.var "t"))

/-- **Outside `0 <= start <= end <= t.len` the run aborts**, at every fuel that reaches it. -/
theorem slice_out_of_range_aborts (n : Nat) :
    (eval Host.unit (n + 2) prog (wSet "1") [] sliceBad).2.1 = .aborted := rfl

theorem slice_scalars : evalPrim .slice [.text "héllo", .int 1, .int 3] = some (.text "él") := rfl

end Una.F1.PreludeExample
