/-
Non-vacuity of #526: **the arithmetic primitives carry their operands' labels, and a zero divisor
aborts.**

- **A product of a secret** (`mul_rejected`, `mul_leaks`, `mul_untyped`): a program prints
  `to_text(len(s) * 2)` at `Console`, which accepts `public`, where `s` is read from the
  environment, so `len(s)` is `internal`. The check rejects it, and its runs from environments where
  `SECRET` is `"1"` and `"22"` print different lines, so T6 refutes every typing of it.
- **A public product is certified** (`public_mul_checked`, `public_mul_runs`): `3 * 4` is
  accepted, typed, and prints `12`.
- **Euclidean division** (`div_euclid`, `mod_euclid`): `-7 div 2 = -4` and `-7 mod 2 = 1`, the
  remainder non-negative, as `spec/11` and SMT-LIB define them.
- **A zero divisor aborts** (`div_by_zero_aborts`): the run's outcome is `aborted` at every fuel
  that reaches the division, neither a value nor a failure. T6 is termination-insensitive: `Term`
  excludes `aborted`, so two runs of which one aborts are outside its premise. Una's soundness
  there rests on the divisor obligation (U5001) and on `spec/07 §1`, which puts aborts outside the
  model; a divisor that depends on a secret and escapes the obligation is a termination channel,
  as the fixed-width overflow abort of ADR 0057 is.
-/
import UnaFormal.F1.Import

namespace Una.F1.ArithExample

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
    (h₂ : Term (eval Host.unit n prog (wSet "22") [] e).2.1) :
    proj prog Label.bot (eval Host.unit n prog (wSet "1") [] e).1 =
      proj prog Label.bot (eval Host.unit n prog (wSet "22") [] e).1 := by
  obtain ⟨hwf, _, hh⟩ := certified
  exact (t6_host_ground (Γ := []) hwf rfl hh Host.unit_lawful (fun _ h => nomatch h) rfl
    (set_rel "1" "22") .nil .nil .nil ht n h₁ h₂).1

/-! ### A product of a secret -/

/-- `let k: Int = len(s) * 2` and print it, with the length of the secret text. -/
def mulLeak : Expr :=
  withSecret
    (.letR "l" (.int intl) (.prim .len [.var "s"])
      (.letR "k" (.int .bot) (.prim .mul [.var "l", .int 2]) (printR "k")))

unseal flowB flowsB in
theorem mul_rejected : ∃ msg, checkExpr prog [] [] .bot (.unit .bot) mulLeak = .error msg :=
  ⟨_, rfl⟩

theorem mul_leaks : (eval Host.unit 10 prog (wSet "1") [] mulLeak).1 = [.out "2"] ∧
    (eval Host.unit 10 prog (wSet "22") [] mulLeak).1 = [.out "4"] := ⟨rfl, rfl⟩

/-- **The leak has no typing.** -/
theorem mul_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot mulLeak t ε := by
  intro ht
  have h := same_lines ht 10 (by exact True.intro) (by exact True.intro)
  rw [mul_leaks.1, mul_leaks.2, proj_single_vis (out_vis _), proj_single_vis (out_vis _)] at h
  simp at h

/-! ### A public product -/

/-- `let k: Int = 3 * 4` and print it, after reading the secret. -/
def publicMul : Expr :=
  withSecret (.letR "k" (.int .bot) (.prim .mul [.int 3, .int 4]) (printR "k"))

unseal flowB flowsB in
theorem public_mul_checked :
    ∃ ε, checkExpr prog [] [] .bot (.unit .bot) publicMul = .ok ε := ⟨_, rfl⟩

/-- **A public product is certified**, and it runs and prints. -/
theorem public_mul_runs : (∃ t ε, LExprTy prog [] [] .bot publicMul t ε ∧ ∀ n : Nat,
    Term (eval Host.unit n prog (wSet "1") [] publicMul).2.1 →
    Term (eval Host.unit n prog (wSet "22") [] publicMul).2.1 →
    proj prog Label.bot (eval Host.unit n prog (wSet "1") [] publicMul).1 =
      proj prog Label.bot (eval Host.unit n prog (wSet "22") [] publicMul).1) ∧
    (eval Host.unit 10 prog (wSet "1") [] publicMul).1 = [.out "12"] := by
  obtain ⟨ε, hc⟩ := public_mul_checked
  obtain ⟨t, ht, _⟩ := checkExpr_sound hc
  exact ⟨⟨t, ε, ht, same_lines ht⟩, rfl⟩

/-! ### Euclidean division, and a zero divisor -/

theorem div_euclid : evalPrim .div [.int (-7), .int 2] = some (.int (-4)) := rfl

theorem mod_euclid : evalPrim .mod [.int (-7), .int 2] = some (.int 1) := rfl

/-- A negative divisor (the review of #526): Euclidean, so the remainder stays non-negative. -/
theorem div_neg_divisor : evalPrim .div [.int 7, .int (-2)] = some (.int (-3)) := rfl

theorem mod_neg_divisor : evalPrim .mod [.int 7, .int (-2)] = some (.int 1) := rfl

/-- `le(a, b)` as the importer reads it: `not (lt b a)` (`Import.expr`, #526). -/
def leRead (a b : Int) : Expr :=
  .letR "t" (.bool .bot) (.prim .lt [.int b, .int a])
    (.letR "x" (.bool .bot) (.prim .not [.var "t"]) (.atom (.var "x")))

/-- `ge(a, b)` as the importer reads it: `not (lt a b)`. -/
def geRead (a b : Int) : Expr :=
  .letR "t" (.bool .bot) (.prim .lt [.int a, .int b])
    (.letR "x" (.bool .bot) (.prim .not [.var "t"]) (.atom (.var "x")))

/-- **The operand order** (the review of #526): `2 <= 3` and `!(2 >= 3)`. -/
theorem le_two_three : (eval Host.unit 4 prog (wSet "1") [] (leRead 2 3)).2.1 = .ok (.bool true) := rfl

theorem ge_two_three : (eval Host.unit 4 prog (wSet "1") [] (geRead 2 3)).2.1 = .ok (.bool false) :=
  rfl

/-- And the boundary: `3 <= 3` and `3 >= 3`. -/
theorem le_ge_equal : (eval Host.unit 4 prog (wSet "1") [] (leRead 3 3)).2.1 = .ok (.bool true) ∧
    (eval Host.unit 4 prog (wSet "1") [] (geRead 3 3)).2.1 = .ok (.bool true) := ⟨rfl, rfl⟩

/-- `let q: Int = 7 div 0`, then `q`. -/
def divZero : Expr := .letR "q" (.int .bot) (.prim .div [.int 7, .int 0]) (.atom (.var "q"))

/-- **A zero divisor aborts**, at every fuel that reaches the division: never a value or a
failure. -/
theorem div_by_zero_aborts (n : Nat) :
    (eval Host.unit (n + 2) prog (wSet "1") [] divZero).2.1 = .aborted := rfl

end Una.F1.ArithExample
