/-
Non-vacuity of M5b (#467): a program that writes a file, certified by the whole-program check as
the re-check certifies one (`Import.mkProgram`, with the document's host labels and the exported
file-state label), so that `t6_host_ground` applies to its runs on a lawful host. `mark` writes
`marker.txt` at `public`, as `s14_ok_failing_writer` does; the file-state label is `public`.
-/
import UnaFormal.F1.Import
import UnaFormal.F1.HostExamples
import UnaFormal.F1.HostFreeExample

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

def sec : LExpr := .ofLabel ⟨.secret, []⟩

/-- `dbl(x: secret Int) -> secret Int = x + x`: it reaches no host operation (#472). -/
def dbl : Fun where
  name := "dbl"
  tparams := []
  eqParams := []
  params := [("x", .int sec)]
  result := .int sec
  op := false
  host := {}
  fails := false
  body := .letR "y" (.int sec) (.prim .add [.var "x", .var "x"]) (.atom (.var "y"))

def prog : Program := Import.mkProgram [] Label.bot labels [] [] [mark, dbl]

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
def w₁ : World HostExamples.files := ⟨[("marker.txt", "a")], (), fun _ => none⟩
def w₂ : World HostExamples.files := ⟨[("marker.txt", "b")], (), fun _ => none⟩

theorem internal_hidden : ¬ Vis prog Label.bot (.ofLabel internalLabel) := by
  rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;>
    simp [prog, Import.mkProgram, Program.top, internalLabel, LExpr.ofLabel, Label.bot] at h <;>
    revert h <;> decide

theorem worlds_rel : WorldRel prog Label.bot w₁ w₂ :=
  ⟨fun _ => rfl, fun h => absurd h internal_hidden, fun _ => rfl⟩

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

/-! ### A declaration that reaches no host operation, beside one that writes (#472) -/

/-- `let z: Int = dbl(s) in z`, with `s` a `secret Int`. -/
def callDbl : Expr := .letR "z" (.int sec) (.call "dbl" [] [.var "s"]) (.atom (.var "z"))

/-- `dbl`'s reach, `{dbl}`, holds no host operation, though `prog` writes a file. -/
theorem dbl_reach : ∀ f fn, prog.find f = some fn → f = "dbl" →
    fn.body.hostFree = true ∧ ∀ g ∈ fn.body.callees, g = "dbl" := by
  intro f fn hf hdbl
  subst hdbl
  simp only [prog, Import.mkProgram, Program.find, List.find?] at hf
  cases hf
  exact ⟨rfl, fun _ h => by simp [dbl, Expr.callees, Rhs.callees] at h⟩

/-- Two file systems the host `s16` tells apart by a read, which breaks H1. -/
def v₁ : World HostFreeExample.s16 := HostFreeExample.fsOk
def v₂ : World HostFreeExample.s16 := HostFreeExample.fsBad

theorem v_rel : WorldRel prog Label.bot v₁ v₂ := ⟨fun _ => rfl, fun h => absurd h internal_hidden, fun _ => rfl⟩

theorem sec_hidden : ¬ Vis prog Label.bot sec := by
  rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;>
    simp [prog, Import.mkProgram, Program.top, sec, LExpr.ofLabel, Label.bot] at h <;>
    revert h <;> decide

unseal flowB flowsB in
theorem callDbl_checked :
    ∃ ε, checkExpr prog [] [("s", .int sec)] .bot (.int sec) callDbl = .ok ε := ⟨_, rfl⟩

theorem sec_hidden' : ¬ Vis prog Label.bot (LExpr.bot.join sec) := by
  rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;>
    simp [prog, Import.mkProgram, Program.top, sec, LExpr.ofLabel, LExpr.join, LExpr.bot, Label.bot,
      Label.join, Level.join] at h <;> revert h <;> decide

/-- **The reach theorem on a host that breaks H1** (#472): `dbl` reaches no host operation, so the
runs of a call of it in `prog`, which writes a file elsewhere, on secrets 1 and 2, are related on
`s16`, from the two file systems that break H1, with no premise on the host. -/
theorem dbl_any_host : ∃ t ε, LExprTy prog [] [("s", .int sec)] .bot callDbl t ε ∧ ∀ n : Nat,
    BRelR prog Label.bot (V prog Label.bot (t.lsubst prog.recordDeep []))
      (Vis prog Label.bot (ε.inst (prog.sd [])).failLabel)
      (eval HostFreeExample.s16 n prog v₁ [("s", .int 1)] callDbl)
      (eval HostFreeExample.s16 n prog v₂ [("s", .int 2)] callDbl) := by
  obtain ⟨hwf, hb1, hh⟩ := certified
  obtain ⟨ε, hc⟩ := callDbl_checked
  obtain ⟨t, ht, _⟩ := checkExpr_sound hc
  refine ⟨t, ε, ht, fun n => ?_⟩
  exact t6_release_hostFreeIn (S := (· = "dbl")) hwf hb1 hh dbl_reach
    (fun _ _ h => by simp [prog, Import.mkProgram, Program.policy] at h) (σ := [])
    (fun _ h => nomatch h) (fun _ h => by simp at h)
    (fun q hq => by simp only [List.mem_singleton] at hq; subst hq; rfl) rfl v_rel
    (.cons .int .nil) (.cons .int .nil) (.cons (.hidden sec_hidden') .nil) ht
    (by simp [Val.HostFreeEnvIn, Val.HostFreeIn]) (by simp [Val.HostFreeEnvIn, Val.HostFreeIn])
    rfl (fun g hg => by simpa [callDbl, Expr.callees, Rhs.callees] using hg) n

end Una.F1.FilesCertExample
