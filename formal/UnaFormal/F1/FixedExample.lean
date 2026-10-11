/-
Non-vacuity of #548: **a fixed-width integer is F1's `Int` at its label, its arithmetic the exact
result checked into its range by `fit`, which aborts outside it** (`spec/18`, ADR 0057).

- **A public `U8` sum is certified** (`public_sum_checked`, `public_sum_runs`): `200 + 50` at
  `U8` is accepted, typed, and prints `250`.
- **A `U8` sum of a secret** (`sum_rejected`, `sum_leaks`, `sum_untyped`): `len(s) + 1` at `U8`,
  printed at `Console`, which accepts `public`, with `s` read from the environment: rejected, it
  leaks, and T6 refutes every typing of it. `fit` carries its operand's label.
- **A narrowing conversion of a secret** (`narrow_rejected`, `narrow_leaks`, `narrow_untyped`):
  `len(s).to_u8()` printed, the same.
- **An overflow aborts** (`overflow_aborts`): `200 + 100` at `U8` aborts at every fuel that
  reaches it, neither a value nor a failure. As a zero divisor (#526), and `/` and `mod` on a
  fixed-width type keep that abort too, an abort is outside T6's termination-insensitive premise;
  Una's guard is the `overflow` obligation (U5001).
-/
import UnaFormal.F1.Import

namespace Una.F1.FixedExample

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

/-- `print(to_text(k))` after `k = fit(lo, hi, t)` for `t = a op b`, the reading of a `U8` step. -/
def u8Step (op : Prim) (a b : Atom) (k : Expr) : Expr :=
  .letR "%fit-k" (.int .bot) (.prim op [a, b])
    (.letR "k" (.int .bot) (.prim (.fit 0 255) [.var "%fit-k"]) k)

/-! ### A public sum -/

def publicSum : Expr := u8Step .add (.int 200) (.int 50) (printR "k")

unseal flowB flowsB in
theorem public_sum_checked :
    ∃ ε, checkExpr prog [] [] .bot (.unit .bot) publicSum = .ok ε := ⟨_, rfl⟩

theorem public_sum_runs : (∃ t ε, LExprTy prog [] [] .bot publicSum t ε) ∧
    (eval Host.unit 10 prog (wSet "1") [] publicSum).1 = [.out "250"] := by
  obtain ⟨ε, hc⟩ := public_sum_checked
  obtain ⟨t, ht, _⟩ := checkExpr_sound hc
  exact ⟨⟨t, ε, ht⟩, rfl⟩

/-! ### A sum of a secret -/

/-- `let l: internal Int = len(s)`, then `let k: U8 = l + 1` printed. -/
def sumLeak : Expr :=
  withSecret (.letR "l" (.int intl) (.prim .len [.var "s"]) (u8Step .add (.var "l") (.int 1) (printR "k")))

unseal flowB flowsB in
theorem sum_rejected : ∃ msg, checkExpr prog [] [] .bot (.unit .bot) sumLeak = .error msg :=
  ⟨_, rfl⟩

theorem sum_leaks : (eval Host.unit 20 prog (wSet "1") [] sumLeak).1 = [.out "2"] ∧
    (eval Host.unit 20 prog (wSet "22") [] sumLeak).1 = [.out "3"] := ⟨rfl, rfl⟩

theorem sum_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot sumLeak t ε := by
  intro ht
  have h := same_lines ht 20 (by exact True.intro) (by exact True.intro)
  rw [sum_leaks.1, sum_leaks.2, proj_single_vis (out_vis _), proj_single_vis (out_vis _)] at h
  simp at h

/-! ### A narrowing conversion of a secret -/

/-- `let k: U8 = len(s).to_u8()` printed. -/
def narrowLeak : Expr :=
  withSecret (.letR "l" (.int intl) (.prim .len [.var "s"])
    (.letR "k" (.int .bot) (.prim (.fit 0 255) [.var "l"]) (printR "k")))

unseal flowB flowsB in
theorem narrow_rejected : ∃ msg, checkExpr prog [] [] .bot (.unit .bot) narrowLeak = .error msg :=
  ⟨_, rfl⟩

theorem narrow_leaks : (eval Host.unit 20 prog (wSet "1") [] narrowLeak).1 = [.out "1"] ∧
    (eval Host.unit 20 prog (wSet "22") [] narrowLeak).1 = [.out "2"] := ⟨rfl, rfl⟩

theorem narrow_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot narrowLeak t ε := by
  intro ht
  have h := same_lines ht 20 (by exact True.intro) (by exact True.intro)
  rw [narrow_leaks.1, narrow_leaks.2, proj_single_vis (out_vis _), proj_single_vis (out_vis _)] at h
  simp at h

/-! ### An overflow -/

def overflow : Expr := u8Step .add (.int 200) (.int 100) (.atom (.var "k"))

/-- **An overflow aborts**, at every fuel that reaches it. -/
theorem overflow_aborts (n : Nat) :
    (eval Host.unit (n + 3) prog (wSet "1") [] overflow).2.1 = .aborted := rfl

/-- `255 / 0` at `U8`: the zero divisor aborts first, then `fit` would. -/
def zeroDiv : Expr := u8Step .div (.int 255) (.int 0) (.atom (.var "k"))

theorem zero_div_aborts (n : Nat) :
    (eval Host.unit (n + 2) prog (wSet "1") [] zeroDiv).2.1 = .aborted := rfl

end Una.F1.FixedExample
