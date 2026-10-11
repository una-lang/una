/-
Non-vacuity of #532: **`first(xs)` carries the top label of `xs`, and the head keeps its own
label inside the option** (`spec/14 §1`).

- **Whether there is a head is public for a public list** (`presence_checked`, `presence_runs`): a
  program reads `s` from the environment, so `s` is `internal`, builds the public list `[s]`, and
  matches `first([s])` only for `Some` or `None` into a print at `Console`, which accepts `public`.
  The check accepts it, it is typed, and it prints `some`.
- **The head itself is not** (`head_rejected`, `head_leaks`, `head_untyped`): the same program
  printing the head is rejected, and its runs from environments where `SECRET` is `"1"` and `"22"`
  print different lines, so T6 refutes every typing of it.
- **The empty list** (`first_empty`): `first([])` is `None`.
-/
import UnaFormal.F1.Import

namespace Una.F1.FirstExample

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

/-- `when Env.get("SECRET") is None -> () is Some(s) -> let xs = [s]; let o = first(xs); k`, with
`xs` a public list of `internal Text` and `o` a public option of it. -/
def withHead (k : Expr) : Expr :=
  .letR "e" (.option (.text intl) .bot) (.envGet (.text "SECRET"))
    (.caseOption (.var "e") (.atom .unit) "s"
      (.letR "xs" (.list (.text intl) .bot) (.list [.var "s"])
        (.letR "o" (.option (.text intl) .bot) (.first (.var "xs")) k)))

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

/-! ### Whether there is a head -/

/-- `when o is None -> print("none") is Some(_) -> print("some")`. -/
def presence : Expr :=
  withHead (.caseOption (.var "o") (say (.text "none")) "h" (say (.text "some")))

unseal flowB flowsB in
theorem presence_checked :
    ∃ ε, checkExpr prog [] [] .bot (.unit .bot) presence = .ok ε := ⟨_, rfl⟩

/-- **Matching a public list's head only for `Some` or `None` is certified**, and it runs and
prints. -/
theorem presence_runs : (∃ t ε, LExprTy prog [] [] .bot presence t ε ∧ ∀ n : Nat,
    Term (eval Host.unit n prog (wSet "1") [] presence).2.1 →
    Term (eval Host.unit n prog (wSet "22") [] presence).2.1 →
    proj prog Label.bot (eval Host.unit n prog (wSet "1") [] presence).1 =
      proj prog Label.bot (eval Host.unit n prog (wSet "22") [] presence).1) ∧
    (eval Host.unit 20 prog (wSet "1") [] presence).1 = [.out "some"] := by
  obtain ⟨ε, hc⟩ := presence_checked
  obtain ⟨t, ht, _⟩ := checkExpr_sound hc
  exact ⟨⟨t, ε, ht, same_lines ht⟩, rfl⟩

/-! ### The head -/

/-- `when o is None -> () is Some(h) -> print(h)`. -/
def headLeak : Expr :=
  withHead (.caseOption (.var "o") (.atom .unit) "h" (say (.var "h")))

unseal flowB flowsB in
theorem head_rejected : ∃ msg, checkExpr prog [] [] .bot (.unit .bot) headLeak = .error msg :=
  ⟨_, rfl⟩

theorem head_leaks : (eval Host.unit 20 prog (wSet "1") [] headLeak).1 = [.out "1"] ∧
    (eval Host.unit 20 prog (wSet "22") [] headLeak).1 = [.out "22"] := ⟨rfl, rfl⟩

/-- **The leak has no typing.** -/
theorem head_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot headLeak t ε := by
  intro ht
  have h := same_lines ht 20 (by exact True.intro) (by exact True.intro)
  rw [head_leaks.1, head_leaks.2, proj_single_vis (out_vis _), proj_single_vis (out_vis _)] at h
  simp at h

/-! ### The empty list -/

/-- `let xs: List[Int] = []; let o: Option[Int] = first(xs); o`. -/
def firstOfEmpty : Expr :=
  .letR "xs" (.list (.int .bot) .bot) (.list [])
    (.letR "o" (.option (.int .bot) .bot) (.first (.var "xs")) (.atom (.var "o")))

theorem first_empty : (eval Host.unit 6 prog (wSet "1") [] firstOfEmpty).2.1 = .ok .none := rfl

end Una.F1.FirstExample
