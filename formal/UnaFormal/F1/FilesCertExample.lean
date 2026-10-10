/-
Non-vacuity of M5b (#467): a program that writes a file, certified by the whole-program check as
the re-check certifies one (`Import.mkProgram`, with the document's host labels and the exported
file-state label), so that `t6_host_ground` applies to its runs on a lawful host. `mark` writes
`marker.txt` at `public`, as `s14_ok_failing_writer` does; the file-state label is `public`.
-/
import UnaFormal.F1.Import
import UnaFormal.F1.HostExamples

namespace Una.F1.FilesCertExample

open Check

/-- `mark() -> Unit uses Files[write] fails = files.write("marker.txt", "")`. -/
def mark : Fun where
  name := "mark"
  tparams := []
  eqParams := []
  params := []
  result := .unit .bot
  op := false
  host := { wr := true }
  fails := true
  body := .letR "u" (.unit .bot) (.write (.text "marker.txt") (.text "")) (.atom (.var "u"))

/-- The host's labels as the document declares them (`Files.read` accepts `public` and gives
`internal`, `Files.write` accepts `internal`), with the exported file-state label `public`. -/
def labels : Import.HostLabels :=
  { acceptsRead := Label.bot, readResult := ⟨.internal, []⟩, acceptsWrite := ⟨.internal, []⟩,
    fileState := some Label.bot }

def prog : Program := Import.mkProgram [] Label.bot labels [] [] [mark]

/-- `let x: Unit = mark() in x`. -/
def callMark : Expr := .letR "x" (.unit .bot) (.call "mark" [] []) (.atom (.var "x"))

-- `flowB` is defined by well-founded recursion: it is unsealed for the kernel to evaluate it.
unseal flowB flowsB in
/-- **The whole-program check certifies the program**, with no hypothesis: its write is typed by
ADR 0085's rules, and its host labels are as `t6_host` asks. -/
theorem certified : prog.LWellFormed ∧ prog.B1WellFormed ∧ prog.HostWF :=
  progOk_sound_nil (by rfl)

unseal flowB flowsB in
theorem callMark_checked :
    ∃ ε, checkExpr prog [] [] .bot (.unit .bot) callMark = .ok ε := ⟨_, rfl⟩

/-- Two file systems with the same file and different contents. -/
def w₁ : World HostExamples.files := ⟨[("marker.txt", "a")], ()⟩
def w₂ : World HostExamples.files := ⟨[("marker.txt", "b")], ()⟩

theorem internal_hidden : ¬ Vis prog Label.bot (.ofLabel internalLabel) := by
  rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;>
    simp [prog, Import.mkProgram, Program.top, internalLabel, LExpr.ofLabel, Label.bot] at h <;>
    revert h <;> decide

theorem worlds_rel : WorldRel prog Label.bot w₁ w₂ :=
  ⟨fun _ => rfl, fun h => absurd h internal_hidden⟩

/-- **T6 for a certified program that writes a file**, on the lawful host `files`, from two file
systems that differ in a file's content: the runs show a public observer the same events and
leave file systems it cannot tell apart. -/
theorem mark_runs (n : Nat) (h₁ : Term (eval HostExamples.files n prog w₁ [] callMark).2.1)
    (h₂ : Term (eval HostExamples.files n prog w₂ [] callMark).2.1) :
    proj prog Label.bot (eval HostExamples.files n prog w₁ [] callMark).1 =
        proj prog Label.bot (eval HostExamples.files n prog w₂ [] callMark).1 ∧
      WorldRel prog Label.bot (eval HostExamples.files n prog w₁ [] callMark).2.2
        (eval HostExamples.files n prog w₂ [] callMark).2.2 := by
  obtain ⟨hwf, _, hh⟩ := certified
  obtain ⟨ε, hc⟩ := callMark_checked
  obtain ⟨t, ht, _⟩ := checkExpr_sound hc
  have h := t6_host_ground (Γ := []) hwf rfl hh HostExamples.files_lawful
    (fun _ h => nomatch h) rfl worlds_rel .nil .nil .nil ht n h₁ h₂
  exact ⟨h.1, h.2.2⟩

/-- The runs end, write the file, and leave file systems that differ in its content only. -/
example : (eval HostExamples.files 4 prog w₁ [] callMark).2.1 = .ok .unit ∧
    (eval HostExamples.files 4 prog w₁ [] callMark).1 = [.write "marker.txt" ""] := ⟨rfl, rfl⟩

end Una.F1.FilesCertExample
