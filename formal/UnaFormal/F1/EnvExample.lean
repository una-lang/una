/-
Non-vacuity of #487 (`docs/plan/formal-track-env.md`): **the environment is a source of secrets.**
`Env.get` reads the world's environment, and two worlds whose environments hold different values
for the same variables are related (`WorldRel`'s third conjunct: whether a variable exists is
`public`, `spec/14 §3`). So T6 now speaks about environment leaks:

- **A leak has no typing** (`leak_untyped`): `main` prints `yes` at `Console`, which accepts
  `public`, when `SECRET` is `"1"`. Its runs from environments where `SECRET` is `"1"` and `"2"`
  print different lines, so T6 itself refutes every typing of it.
- **An `internal` read is certified** (`read_runs`): its runs from the same two environments are
  related, and return different values (`read_differ`), both hidden from a `public` observer.
- **Existence is public** (`exists_runs`): a program that branches on whether `SECRET` is set and
  prints is certified. Worlds where it is set in one run only are not related (`exists_unrelated`):
  that is the premise, whether a variable exists is public configuration, so an operator must not
  encode a secret in which variables are set.
-/
import UnaFormal.F1.Import

namespace Una.F1.EnvExample

open Check

def labels : Import.HostLabels :=
  { acceptsReq := ⟨.internal, []⟩, reqResult := ⟨.internal, []⟩, fileState := some Label.bot }

/-- `Console` accepts `public`. -/
def prog : Program := Import.mkProgram [] Label.bot labels [] [] []

def intl : LExpr := .ofLabel ⟨.internal, []⟩

/-- A world on the stateless host whose environment sets `SECRET` to `v`. -/
def wSet (v : String) : World Host.unit := ⟨(), (), fun x => if x = "SECRET" then some v else none⟩

/-- A world whose environment sets nothing. -/
def wUnset : World Host.unit := ⟨(), (), fun _ => none⟩

theorem internal_hidden : ¬ Vis prog Label.bot (.ofLabel internalLabel) := by
  rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;>
    simp [prog, Import.mkProgram, Program.top, internalLabel, LExpr.ofLabel, Label.bot] at h <;>
    revert h <;> decide

/-- **Two environments that differ in `SECRET`'s value are related** for a `public` observer. -/
theorem set_rel (a b : String) : WorldRel prog Label.bot (wSet a) (wSet b) :=
  ⟨fun _ => rfl, fun h => absurd h internal_hidden, fun x => by simp only [wSet]; split <;> rfl⟩

/-- **The premise**: worlds where `SECRET` is set in one run only are not related, for any
observer. -/
theorem exists_unrelated (L : Label) (a : String) : ¬ WorldRel prog L (wSet a) wUnset :=
  fun h => by simpa [wSet, wUnset] using h.2.2 "SECRET"

/-- `print(s)` at `Console`. -/
def say (s : String) : Expr := .letR "u" (.unit .bot) (.op (.text s)) (.atom (.var "u"))

/-- `let e = Env.get("SECRET")` and then `k`. -/
def readThen (k : Expr) : Expr := .letR "e" (.option (.text intl) .bot) (.envGet (.text "SECRET")) k

/-! ### A leak -/

/-- `when Env.get("SECRET") is None -> () is Some(s) -> if s == "1" then print("yes") else ()`. -/
def leak : Expr :=
  readThen (.caseOption (.var "e") (.atom .unit) "s"
    (.letR "b" (.bool intl) (.eq (.var "s") (.text "1")) (.ite (.var "b") (say "yes") (.atom .unit))))

/-- The two runs print different lines. -/
theorem leak_runs : (eval Host.unit 10 prog (wSet "1") [] leak).1 = [.out "yes"] ∧
    (eval Host.unit 10 prog (wSet "2") [] leak).1 = [] := ⟨rfl, rfl⟩

unseal flowB flowsB in
theorem certified : prog.LWellFormed ∧ prog.B1WellFormed ∧ prog.HostWF :=
  progOk_sound_nil (by rfl)

/-- `Console` accepts `public`: the observer sees its lines. -/
theorem say_vis : EvVis prog Label.bot (.out "yes") := LExpr.le_refl _

/-- **The leak has no typing**: T6, over the related worlds of `set_rel`, would make the two
runs print the same lines. -/
theorem leak_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot leak t ε := by
  intro ht
  obtain ⟨hwf, _, hh⟩ := certified
  have h := (t6_host_ground (Γ := []) hwf rfl hh Host.unit_lawful (fun _ h => nomatch h) rfl
    (set_rel "1" "2") .nil .nil .nil ht 10 (by exact True.intro)
      (by exact True.intro)).1
  rw [leak_runs.1, leak_runs.2, proj_single_vis (p := prog) (L := Label.bot) (ev := .out "yes") say_vis] at h
  simp [proj_nil] at h

/-! ### An `internal` read -/

/-- `Env.get("SECRET")`, an `Option[internal Text]`. -/
def read : Expr := readThen (.atom (.var "e"))

unseal flowB flowsB in
theorem read_checked :
    ∃ ε, checkExpr prog [] [] .bot (.option (.text intl) .bot) read = .ok ε := ⟨_, rfl⟩

/-- **T6 for an environment read** (#487): the runs from environments where `SECRET` is `"1"` and
`"2"` show a `public` observer the same events and related outcomes. -/
theorem read_runs : ∃ t ε, LExprTy prog [] [] .bot read t ε ∧ ∀ n : Nat,
    Term (eval Host.unit n prog (wSet "1") [] read).2.1 →
    Term (eval Host.unit n prog (wSet "2") [] read).2.1 →
    proj prog Label.bot (eval Host.unit n prog (wSet "1") [] read).1 =
        proj prog Label.bot (eval Host.unit n prog (wSet "2") [] read).1 ∧
      OutRel prog Label.bot (V prog Label.bot t) (Vis prog Label.bot ε.failLabel)
        (eval Host.unit n prog (wSet "1") [] read).2.1 (eval Host.unit n prog (wSet "2") [] read).2.1 ∧
      WorldRel prog Label.bot (eval Host.unit n prog (wSet "1") [] read).2.2
        (eval Host.unit n prog (wSet "2") [] read).2.2 := by
  obtain ⟨hwf, _, hh⟩ := certified
  obtain ⟨ε, hc⟩ := read_checked
  obtain ⟨t, ht, _⟩ := checkExpr_sound hc
  exact ⟨t, ε, ht, fun n => t6_host_ground (Γ := []) hwf rfl hh Host.unit_lawful
    (fun _ h => nomatch h) rfl (set_rel "1" "2") .nil .nil .nil ht n⟩

/-- The runs return different values. -/
theorem read_differ : (eval Host.unit 2 prog (wSet "1") [] read).2.1 = .ok (.some (.text "1")) ∧
    (eval Host.unit 2 prog (wSet "2") [] read).2.1 = .ok (.some (.text "2")) := ⟨rfl, rfl⟩

/-! ### Existence -/

/-- `when Env.get("SECRET") is None -> print("unset") is Some(s) -> print("set")`. -/
def exists_ : Expr := readThen (.caseOption (.var "e") (say "unset") "s" (say "set"))

unseal flowB flowsB in
theorem exists_checked : ∃ ε, checkExpr prog [] [] .bot (.unit .bot) exists_ = .ok ε := ⟨_, rfl⟩

/-- **Branching on existence is certified**, and T6 relates its runs from environments that both
set `SECRET`. -/
theorem exists_runs : ∃ t ε, LExprTy prog [] [] .bot exists_ t ε ∧ ∀ n : Nat,
    Term (eval Host.unit n prog (wSet "1") [] exists_).2.1 →
    Term (eval Host.unit n prog (wSet "2") [] exists_).2.1 →
    proj prog Label.bot (eval Host.unit n prog (wSet "1") [] exists_).1 =
        proj prog Label.bot (eval Host.unit n prog (wSet "2") [] exists_).1 := by
  obtain ⟨hwf, _, hh⟩ := certified
  obtain ⟨ε, hc⟩ := exists_checked
  obtain ⟨t, ht, _⟩ := checkExpr_sound hc
  exact ⟨t, ε, ht, fun n h₁ h₂ => (t6_host_ground (Γ := []) hwf rfl hh Host.unit_lawful
    (fun _ h => nomatch h) rfl (set_rel "1" "2") .nil .nil .nil ht n h₁ h₂).1⟩

/-- Where `SECRET` is unset, the program prints another line: the runs that `exists_unrelated`
keeps out of T6. -/
example : (eval Host.unit 10 prog (wSet "1") [] exists_).1 = [.out "set"] ∧
    (eval Host.unit 10 prog wUnset [] exists_).1 = [.out "unset"] := ⟨rfl, rfl⟩

end Una.F1.EnvExample
