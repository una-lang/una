/-
M6, a finding of the proof (`docs/plan/formal-track-m6.md`): **an iteration's failure decides
whether the later iterations run**, so T-Loop bounds it by what decides whether the iteration runs.
M6's draft rule (`Program.oldLoop`) typed the body under the list's top label only. Under it, the
probe below has a typing: over a `public` list, each iteration prints `hello`, then reads a file,
whose failure the file-state label, `internal`, decides (ADR 0085). Two runs from worlds a `public`
observer cannot tell apart, one without the file and one with it, print one and two lines at
`Console`, which accepts `public`. Under the rule as fixed, the probe has no typing. The surface
language rejects the shape already: a failure a `for` body passes on must be public (U4002,
`una check` 0.3.5, with `parse_int`, a user function, and a nested loop).
-/
import UnaFormal.F1.BinaryLemmas
import UnaFormal.F1.CheckSoundCore

namespace Una.F1.LoopLeak

/-- `internal`. -/
def intl : LExpr := .ofLabel ⟨.internal, []⟩

/-- A file system as the list of the files that exist; a read fails when its file does not. -/
def files : Host where
  FS := List String
  Srv := Unit
  Shape := List String
  shape := id
  read := fun path fs => if fs.contains path then some "" else none
  write := fun path _ fs => some (path :: fs)
  request := fun _ _ s => (.error 0, s)

/-- The host meets H1, H2 and H4: its shape is the whole file system, and it answers no request. -/
theorem files_lawful : files.Lawful where
  read_fails := fun _ h => by simp only [files, id] at h; rw [h]
  write_shape := fun _ _ _ h => by simp only [files, id] at h; rw [h]; rfl
  resp_typed := fun _ _ _ _ _ h => nomatch h

/-- A program whose `Console` and `Files.read` accept `public`, and whose file-state label is
`internal`, under M6's draft rule for loops. -/
def draftProg : Program where
  categories := []
  acceptsOp := Label.bot
  acceptsRead := Label.bot
  acceptsWrite := ⟨.internal, []⟩
  acceptsReq := ⟨.internal, []⟩
  readResult := ⟨.internal, []⟩
  reqResult := ⟨.internal, []⟩
  reqFloor := ⟨.internal, []⟩
  fileState := ⟨.internal, []⟩
  oldLoop := true
  records := []
  policies := []
  funs := []

/-- The same program under the rule as fixed. -/
def fixedProg : Program := { draftProg with oldLoop := false }

/-- A `public` list of integers. -/
def Γ : LCtx := [("xs", .list (.int .bot) .bot)]

/-- One iteration: print `hello`, then read `a.txt`, which fails when it does not exist. -/
def iter : Expr :=
  .letR "o" (.unit .bot) (.op (.text "hello"))
    (.letR "c" (.text intl) (.read (.text "a.txt"))
      (.letR "t" (.tuple [] .bot) (.tuple []) (.atom (.var "t"))))

/-- **The probe**: `for x in xs`, with no state. -/
def probe : Expr := .loop (.list (.var "xs")) "x" (.int .bot) [] [] iter

unseal Check.flowB Check.flowsB in
/-- Under the draft rule, the probe has a typing: the checker accepts it. -/
theorem draft_typed : ∃ t ε, LExprTy draftProg [] Γ .bot probe t ε := by
  obtain ⟨t, ht, _⟩ := Check.checkExpr_sound (p := draftProg) (E := []) (Γ := Γ) (pc := .bot)
    (s := .tuple [] .bot) (e := probe) (ε := _) rfl
  exact ⟨t, _, ht⟩

/-- The list `[1, 2]`. -/
def env : Env := [("xs", .list [.int 1, .int 2])]

/-- Two worlds: without `a.txt`, and with it. -/
def w₁ : World files := ⟨([] : List String), (), fun _ => none⟩
def w₂ : World files := ⟨["a.txt"], (), fun _ => none⟩

theorem intl_hidden (P : Program) (hP : P.categories = []) : ¬ Vis P Label.bot intl := by
  rintro (h | ⟨h, _⟩) <;>
    simp [Program.top, hP, intl, LExpr.ofLabel, Label.le, Label.bot] at h <;> revert h <;> decide

/-- A `public` observer cannot tell the worlds apart: the file-state label is `internal`. -/
theorem worlds_rel : WorldRel draftProg Label.bot w₁ w₂ :=
  ⟨fun h => absurd h (intl_hidden _ rfl), fun h => absurd h (intl_hidden _ rfl), fun _ => rfl⟩

/-- The runs: the first fails after one `hello`, the second ends after two. -/
theorem runs :
    eval files 20 draftProg w₁ env probe = ([.out "hello", .read "a.txt"], .failed hostFailure, w₁) ∧
    eval files 20 draftProg w₂ env probe =
      ([.out "hello", .read "a.txt", .out "hello", .read "a.txt"], .ok (.tuple []), w₂) :=
  ⟨rfl, rfl⟩

/-- **The draft rule leaks**: both runs end, and the `public` observer sees one `hello` in the
first and two in the second. -/
theorem draft_leaks :
    proj draftProg Label.bot (eval files 20 draftProg w₁ env probe).1 ≠
      proj draftProg Label.bot (eval files 20 draftProg w₂ env probe).1 := by
  rw [runs.1, runs.2]
  have ho : EvVis draftProg Label.bot (.out "hello") := LExpr.le_refl _
  have hr : EvVis draftProg Label.bot (.read "a.txt") := LExpr.le_refl _
  simp only [proj, List.filter, ho, hr, decide_true]
  simp

/-- **Under the rule as fixed, the probe has no typing.** -/
theorem fixed_untyped {E : List String} {t : LTy} {ε : Eff} :
    ¬ LExprTy fixedProg E Γ .bot probe t ε := by
  intro h
  cases h with
  | loopList hxs _ _ _ _ _ hb _ hfe =>
    cases hxs with
    | var hl =>
      simp [Γ, lctxLookup] at hl
      obtain ⟨rfl, rfl⟩ := hl
      have hfe := hfe rfl
      cases hb with
      | letR _ _ hb₂ _ =>
        cases hb₂ with
        | letR hrd _ _ _ =>
          cases hrd with
          | read _ _ _ _ =>
            apply intl_hidden fixedProg rfl
            refine LExpr.le_trans ?_ (LExpr.le_trans hfe ?_)
            · refine LExpr.le_trans ?_ (failLabel_union_right _ _)
              refine LExpr.le_trans ?_ (failLabel_union_left _ _)
              simp only [Eff.failLabel, Option.getD_some, Program.fsFail, fixedProg, draftProg,
                Bool.false_eq_true, ite_false]
              exact LExpr.le_join_right _ _
            · exact LExpr.join_le (LExpr.le_refl _) (LExpr.le_refl _)

end Una.F1.LoopLeak
