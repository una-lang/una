/-
Non-vacuity of #545: **`at(xs, index)` carries the top label of `xs` and the label of `index`, and
the element keeps its own label inside the option** (`spec/14 §1`).

- **Whether there is an element, at a public index of a public list, is public**
  (`presence_checked`, `presence_runs`): a program reads `s` from the environment, so `s` is
  `internal`, and matches `at([s], 0)` only for `Some` or `None` into a print at `Console`, which
  accepts `public`. The check accepts it, it is typed, and it prints `some`.
- **The element itself is not** (`elem_rejected`, `elem_leaks`, `elem_untyped`): printing it is
  rejected, its runs from environments where `SECRET` is `"1"` and `"22"` print different lines,
  and T6 refutes every typing of it.
- **A secret index decides whether there is one** (`index_rejected`, `index_leaks`,
  `index_untyped`): `at([7, 8], len(s))`, a public list of public integers at an `internal` index,
  matched for `Some` or `None` into a public print, is rejected, it leaks (`some` and `none`), and
  it has no typing.
-/
import UnaFormal.F1.Import

namespace Una.F1.AtExample

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

/-- `when Env.get("SECRET") is None -> () is Some(s) -> k`, with `s` an `internal Text`. -/
def withSecret (k : Expr) : Expr :=
  .letR "e" (.option (.text intl) .bot) (.envGet (.text "SECRET"))
    (.caseOption (.var "e") (.atom .unit) "s" k)

/-- `let xs = [s]; let o = at(xs, 0); k`, with `xs` a public list of `internal Text` and `o` a
public option of it. -/
def withElem (k : Expr) : Expr :=
  withSecret (.letR "xs" (.list (.text intl) .bot) (.list [.var "s"])
    (.letR "o" (.option (.text intl) .bot) (.elemAt (.var "xs") (.int 0)) k))

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

/-! ### Whether there is an element -/

/-- `when o is None -> print("none") is Some(_) -> print("some")`. -/
def presence : Expr :=
  withElem (.caseOption (.var "o") (say (.text "none")) "h" (say (.text "some")))

unseal flowB flowsB in
theorem presence_checked :
    ∃ ε, checkExpr prog [] [] .bot (.unit .bot) presence = .ok ε := ⟨_, rfl⟩

theorem presence_runs : (∃ t ε, LExprTy prog [] [] .bot presence t ε ∧ ∀ n : Nat,
    Term (eval Host.unit n prog (wSet "1") [] presence).2.1 →
    Term (eval Host.unit n prog (wSet "22") [] presence).2.1 →
    proj prog Label.bot (eval Host.unit n prog (wSet "1") [] presence).1 =
      proj prog Label.bot (eval Host.unit n prog (wSet "22") [] presence).1) ∧
    (eval Host.unit 20 prog (wSet "1") [] presence).1 = [.out "some"] := by
  obtain ⟨ε, hc⟩ := presence_checked
  obtain ⟨t, ht, _⟩ := checkExpr_sound hc
  exact ⟨⟨t, ε, ht, same_lines ht⟩, rfl⟩

/-! ### The element -/

/-- `when o is None -> () is Some(h) -> print(h)`. -/
def elemLeak : Expr :=
  withElem (.caseOption (.var "o") (.atom .unit) "h" (say (.var "h")))

unseal flowB flowsB in
theorem elem_rejected : ∃ msg, checkExpr prog [] [] .bot (.unit .bot) elemLeak = .error msg :=
  ⟨_, rfl⟩

theorem elem_leaks : (eval Host.unit 20 prog (wSet "1") [] elemLeak).1 = [.out "1"] ∧
    (eval Host.unit 20 prog (wSet "22") [] elemLeak).1 = [.out "22"] := ⟨rfl, rfl⟩

theorem elem_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot elemLeak t ε := by
  intro ht
  have h := same_lines ht 20 (by exact True.intro) (by exact True.intro)
  rw [elem_leaks.1, elem_leaks.2, proj_single_vis (out_vis _), proj_single_vis (out_vis _)] at h
  simp at h

/-! ### A secret index -/

/-- `let i = len(s); let o: Option[Int] = at([7, 8], i)`, matched into a public print. -/
def indexLeak : Expr :=
  withSecret (.letR "i" (.int intl) (.prim .len [.var "s"])
    (.letR "xs" (.list (.int .bot) .bot) (.list [.int 7, .int 8])
      (.letR "o" (.option (.int .bot) .bot) (.elemAt (.var "xs") (.var "i"))
        (.caseOption (.var "o") (say (.text "none")) "h" (say (.text "some"))))))

unseal flowB flowsB in
theorem index_rejected : ∃ msg, checkExpr prog [] [] .bot (.unit .bot) indexLeak = .error msg :=
  ⟨_, rfl⟩

theorem index_leaks : (eval Host.unit 20 prog (wSet "1") [] indexLeak).1 = [.out "some"] ∧
    (eval Host.unit 20 prog (wSet "22") [] indexLeak).1 = [.out "none"] := ⟨rfl, rfl⟩

theorem index_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot indexLeak t ε := by
  intro ht
  have h := same_lines ht 20 (by exact True.intro) (by exact True.intro)
  rw [index_leaks.1, index_leaks.2, proj_single_vis (out_vis _), proj_single_vis (out_vis _)] at h
  simp at h

theorem at_out_of_range : atIndex [.int 7] 1 = .none ∧ atIndex [.int 7] (-1) = .none ∧
    atIndex [.int 7] 0 = .some (.int 7) := ⟨rfl, rfl, rfl⟩

end Una.F1.AtExample
