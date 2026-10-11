/-
M5, non-vacuity of `t6_host` (`docs/plan/formal-track-m5.md` §4), on a lawful host whose files
hold contents:

- **public writes** (`exA_*`): a package whose writes are all `public`, so its file-state label is
  `public` and its read failures are `public`. Two runs from worlds with the same files but
  different contents print the same lines to a `public` observer.
- **an internal write, the S-14 fix proven positively** (`exB_*`): a package that writes to an
  `internal` path, so its file-state label is `internal` and every `Files` failure is. Two runs from
  worlds with different files, and different secret paths, read `a.txt` with different outcomes,
  yet a `public` observer sees the same events.
-/
import UnaFormal.F1.Runs
import UnaFormal.F1.CheckSound

namespace Una.F1.HostExamples

open Una (Level)

/-- `internal`. -/
def intl : LExpr := .ofLabel ⟨.internal, []⟩

/-- A file system as a list of files with their contents; every write succeeds. -/
def files : Host where
  FS := List (String × String)
  Srv := Unit
  Shape := List String
  shape := fun fs => fs.map Prod.fst
  read := fun path fs => fs.lookup path
  write := fun path c fs => some ((path, c) :: fs)
  request := fun _ _ _ => (.error 0, ())

theorem lookup_isSome {path : String} : ∀ fs : List (String × String),
    (fs.lookup path).isSome ↔ path ∈ fs.map Prod.fst
  | [] => by simp
  | (k, v) :: fs => by
    by_cases hk : path = k
    · subst hk; simp [List.lookup]
    · have hb : (path == k) = false := by simpa using hk
      simp [List.lookup, hb, lookup_isSome fs, hk]

theorem files_lawful : files.Lawful where
  resp_typed := fun _ _ _ _ _ h => by cases h
  read_fails := fun {a b} path h => by
    have h' : (a : List (String × String)).map Prod.fst = (b : List (String × String)).map Prod.fst := h
    have e₁ := lookup_isSome (path := path) a
    have e₂ := lookup_isSome (path := path) b
    rw [h'] at e₁
    exact e₁.trans e₂.symm
  write_shape := fun {a b} path c c' h => by
    have h' : (a : List (String × String)).map Prod.fst = (b : List (String × String)).map Prod.fst := h
    show some (path :: (a : List (String × String)).map Prod.fst) =
      some (path :: (b : List (String × String)).map Prod.fst)
    rw [h']

/-- A program of the demo shape, with file-state label at level `fs`. -/
def prog (fs : Level) : Program where
  categories := []
  acceptsOp := Label.bot
  acceptsRead := Label.bot
  acceptsWrite := ⟨.internal, []⟩
  acceptsReq := ⟨.internal, []⟩
  readResult := ⟨.internal, []⟩
  reqResult := ⟨.internal, []⟩
  reqFloor := ⟨.internal, []⟩
  fileState := ⟨fs, []⟩
  records := []
  policies := []
  funs := []

theorem le_of {P : Program} {a b : LExpr} (h : Una.F1.Check.leB P.top a b = true) : P.le a b :=
  Una.F1.Check.leB_sound h

theorem sf {e : LExpr} (h : Una.F1.Check.symFree e = true) : e.SymFree := Una.F1.Check.symFree_sound h

theorem hostWF (fs : Level) : (prog fs).HostWF where
  oldHost := rfl
  read := le_of rfl
  write := le_of rfl
  req := le_of rfl
  readResult := le_of rfl
  reqResult := le_of rfl
  reqFloor := le_of rfl
  oldLoop := rfl

theorem wf (fs : Level) : (prog fs).LWellFormed where
  funs := by simp [prog]
  records := by simp [prog]
  recordDeep := by simp [prog]
  policies := by simp [prog]
  categories := by simp [Program.CatsDeclared, prog, Label.bot]
  recordsGround := by simp [prog]
  sums := by simp [prog]
  disjoint _ := .inr (by simp [Program.sum, prog])
  ops _ h := by simp [prog] at h
  eqScoped := by simp [prog]
  sigsScoped := by simp [prog]
  inPlace := by simp [prog]

/-- `internal` is not seen by a `public` observer. -/
theorem intl_hidden (fs : Level) : ¬ Vis (prog fs) Label.bot (.ofLabel internalLabel) := by
  rintro (h | ⟨h, _⟩) <;>
    simp [prog, Program.top, internalLabel, LExpr.ofLabel, Label.le, Label.bot] at h <;>
    revert h <;> decide

/-! ### (A) Public writes -/

/-- Write `log`, read it back, and print whether the read succeeded. -/
def eA : Expr :=
  .letE "w" (.result (.unit .bot) (.int .bot) .bot)
    (.attempt (.letR "u" (.unit .bot) (.write (.text "log") (.text "x")) (.atom (.var "u"))))
    (.letE "r" (.result (.text intl) (.int .bot) .bot)
      (.attempt (.letR "c" (.text intl) (.read (.text "log")) (.atom (.var "c"))))
      (.caseResult (.var "r") "v"
        (.letR "o" (.unit .bot) (.op (.text "yes")) (.atom (.var "o"))) "e"
        (.letR "o" (.unit .bot) (.op (.text "no")) (.atom (.var "o")))))

theorem eA_typed : ∃ t ε, LExprTy (prog .public) [] [] .bot eA t ε := by
  have hw : LExprTy (prog .public) [] [] .bot
      (.letR "u" (.unit .bot) (.write (.text "log") (.text "x")) (.atom (.var "u"))) _ _ :=
    .letR (.write .text rfl .text rfl (le_of rfl) (fun _ => le_of rfl) (sf rfl)) .unit
      (.atom (.var rfl)) rfl
  have hr : LExprTy (prog .public) [] [("w", .result (.unit .bot) (.int .bot) .bot)]
      (LExpr.bot.join LExpr.bot)
      (.letR "c" (.text intl) (.read (.text "log")) (.atom (.var "c"))) _ _ :=
    .letR (.read .text rfl (le_of rfl) (sf rfl)) (.text (le_of rfl)) (.atom (.var rfl)) rfl
  have hall : LExprTy (prog .public) [] [] .bot eA _ _ :=
    .letE (x := "w") (ann := .result (.unit .bot) (.int .bot) .bot) (.attempt hw (by simp [Eff.union, Eff.none]))
      (.result .unit (.int (le_of rfl)) (le_of rfl))
      (.letE (x := "r") (ann := .result (.text intl) (.int .bot) .bot) (.attempt hr (by simp [Eff.union, Eff.none]))
        (.result (.text (le_of rfl)) (.int (le_of rfl)) (le_of rfl))
        (.caseResult (s := .unit .bot) (.var rfl)
          (.letR (.op .text trivial (le_of rfl) (sf rfl)) .unit (.atom (.var rfl)) rfl)
          (.letR (.op .text trivial (le_of rfl) (sf rfl)) .unit (.atom (.var rfl)) rfl)
          .unit .unit rfl)
        (by simp [LTy.InPlace, intl, LExpr.ofLabel, LExpr.bot]))
      (by simp [LTy.InPlace, LExpr.ofLabel, LExpr.bot])
  exact ⟨_, _, hall⟩

/-- Two worlds with the same file, `log`, and different contents. -/
def wA₁ : World files := ⟨([("log", "1")] : List (String × String)), (), fun _ => none⟩
def wA₂ : World files := ⟨([("log", "2")] : List (String × String)), (), fun _ => none⟩

theorem wA_rel : WorldRel (prog .public) Label.bot wA₁ wA₂ :=
  ⟨fun _ => rfl, fun h => absurd h (intl_hidden _), fun _ => rfl⟩

/-- **(A)**: `t6_host_ground` applies, so a public observer sees the same events of the two runs. -/
theorem exA : proj (prog .public) Label.bot (eval files 12 (prog .public) wA₁ [] eA).1 =
    proj (prog .public) Label.bot (eval files 12 (prog .public) wA₂ [] eA).1 := by
  obtain ⟨t, ε, he⟩ := eA_typed
  have r₁ : (eval files 12 (prog .public) wA₁ [] eA).2.1 = .ok .unit := rfl
  have r₂ : (eval files 12 (prog .public) wA₂ [] eA).2.1 = .ok .unit := rfl
  exact (t6_host_ground (Γ := []) (wf _) rfl (hostWF _) files_lawful (fun _ h => nomatch h) rfl wA_rel
    .nil .nil .nil he 12 (by rw [r₁]; trivial) (by rw [r₂]; trivial)).1

/-- The two worlds differ, and both runs print `yes`: the read failure is public. -/
theorem exA_runs : wA₁ ≠ wA₂ ∧
    (eval files 12 (prog .public) wA₁ [] eA).1.filter (· matches .out _) = [.out "yes"] ∧
    (eval files 12 (prog .public) wA₂ [] eA).1.filter (· matches .out _) = [.out "yes"] :=
  ⟨fun h => by
    have := congrArg World.fs h
    exact absurd (show ([("log", "1")] : List (String × String)) = [("log", "2")] from this)
      (by decide), rfl, rfl⟩

/-! ### (B) An internal write: the S-14 fix, proven positively -/

/-- Write to the secret path `x`, read `a.txt`, and return 1 or 2 by whether the read succeeded:
no print, since the read's failure is `internal`. -/
def eB : Expr :=
  .letE "w" (.result (.unit .bot) (.int .bot) intl)
    (.attempt (.letR "u" (.unit .bot) (.write (.var "x") (.text "x")) (.atom (.var "u"))))
    (.letE "r" (.result (.text intl) (.int .bot) intl)
      (.attempt (.letR "c" (.text intl) (.read (.text "a.txt")) (.atom (.var "c"))))
      (.caseResult (.var "r") "v" (.atom (.int 1)) "e" (.atom (.int 2))))

def ΓB : LCtx := [("x", .text intl)]

theorem eB_typed : ∃ t ε, LExprTy (prog .internal) [] ΓB .bot eB t ε := by
  have hw : LExprTy (prog .internal) [] ΓB .bot
      (.letR "u" (.unit .bot) (.write (.var "x") (.text "x")) (.atom (.var "u"))) _ _ :=
    .letR (.write (.var rfl) rfl .text rfl (le_of rfl) (fun _ => le_of rfl) (sf rfl)) .unit
      (.atom (.var rfl)) rfl
  have hr : LExprTy (prog .internal) [] (("w", .result (.unit .bot) (.int .bot) intl) :: ΓB)
      (LExpr.bot.join LExpr.bot)
      (.letR "c" (.text intl) (.read (.text "a.txt")) (.atom (.var "c"))) _ _ :=
    .letR (.read .text rfl (le_of rfl) (sf rfl)) (.text (le_of rfl)) (.atom (.var rfl)) rfl
  have hall : LExprTy (prog .internal) [] ΓB .bot eB _ _ :=
    .letE (x := "w") (ann := .result (.unit .bot) (.int .bot) intl) (.attempt hw (by simp [Eff.union, Eff.none]))
      (.result .unit (.int (le_of rfl)) (le_of rfl))
      (.letE (x := "r") (ann := .result (.text intl) (.int .bot) intl) (.attempt hr (by simp [Eff.union, Eff.none]))
        (.result (.text (le_of rfl)) (.int (le_of rfl)) (le_of rfl))
        (.caseResult (s := .int .bot) (.var rfl) (.atom .int) (.atom .int) (.int (le_of rfl))
          (.int (le_of rfl)) rfl)
        (by simp [LTy.InPlace, intl, LExpr.ofLabel, LExpr.bot]))
      (by simp [LTy.InPlace, intl, LExpr.ofLabel, LExpr.bot])
  exact ⟨_, _, hall⟩

/-- Two worlds with different files. -/
def wB₁ : World files := ⟨([] : List (String × String)), (), fun _ => none⟩
def wB₂ : World files := ⟨([("c.txt", "z")] : List (String × String)), (), fun _ => none⟩

theorem wB_rel : WorldRel (prog .internal) Label.bot wB₁ wB₂ :=
  ⟨fun h => absurd (Vis.of_le (le_of rfl) h) (intl_hidden _), fun h => absurd h (intl_hidden _), fun _ => rfl⟩

def envB (s : String) : Env := [("x", .text s)]

theorem intl_hidden' : ¬ Vis (prog .internal) Label.bot intl := intl_hidden _

/-- **(B)**: the secret path `a.txt` against `b.txt`, from worlds of different shapes. -/
theorem exB : proj (prog .internal) Label.bot (eval files 12 (prog .internal) wB₁ (envB "a.txt") eB).1 =
    proj (prog .internal) Label.bot (eval files 12 (prog .internal) wB₂ (envB "b.txt") eB).1 := by
  obtain ⟨t, ε, he⟩ := eB_typed
  have r₁ : (eval files 12 (prog .internal) wB₁ [("x", .text "a.txt")] eB).2.1 = .ok (.int 1) := rfl
  have r₂ : (eval files 12 (prog .internal) wB₂ [("x", .text "b.txt")] eB).2.1 = .ok (.int 2) := rfl
  exact (t6_host_ground (Γ := ΓB) (wf _) rfl (hostWF _) files_lawful
    (fun q hq => by simp only [ΓB, List.mem_singleton] at hq; subst hq; rfl) rfl wB_rel
    (.cons .text .nil) (.cons .text .nil) (.cons (.hidden intl_hidden') .nil) he 12
    (by rw [r₁]; trivial) (by rw [r₂]; trivial)).1

/-- The read of `a.txt` succeeds in one run and fails in the other, so the runs return 1 and 2,
which the observer cannot see: the outcomes differ, and the theorem still relates the runs. -/
theorem exB_runs : (eval files 12 (prog .internal) wB₁ (envB "a.txt") eB).2.1 = .ok (.int 1) ∧
    (eval files 12 (prog .internal) wB₂ (envB "b.txt") eB).2.1 = .ok (.int 2) := ⟨rfl, rfl⟩

end Una.F1.HostExamples
