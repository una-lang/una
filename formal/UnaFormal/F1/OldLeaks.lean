/-
M5, part 1 (`docs/plan/formal-track-m5.md` §6): **the model captures S-14 and S-15.** Under the
rules before ADR 0085 (`Program.oldHost`), each finding's probe has a typing, and two runs of it
that differ only in an `internal` input print different lines at `Console`, which accepts
`public`: what a public observer sees depends on an `internal` value. So M3's assumption 3, a
stateless host, was needed by the old rules, and the host breaks it.

- **S-14** (`s14_old_typed`, `s14_old_leaks`): a write to an `internal` path decides whether a
  read of a `public` path fails.
- **S-15** (`s15_old_typed`, `s15_old_leaks`): a server decides a request's failure by its
  `internal` body.
-/
import UnaFormal.F1.LTyping
import UnaFormal.F1.CheckSoundCore

namespace Una.F1.OldLeaks

open Una (Level)

/-- `internal`. -/
def intl : LExpr := .ofLabel ⟨.internal, []⟩

/-- A file system as the list of the files that exist, empty when created; every write succeeds,
and a read fails when its file does not exist. A server that fails a request whose body (its
second argument) is `"1"`. -/
def demo : Host where
  FS := List String
  Srv := Unit
  Shape := List String
  shape := id
  read := fun path fs => if fs.contains path then some "" else none
  write := fun path _ fs => some (path :: fs)
  request := fun _ args _ =>
    match args with
    | [_, _, .text "1", _] => (.error 0, ())
    | _ => (.ok (.record "prelude.Response" [.int 200, .list [], .text ""]), ())

/-- The demo host meets H1 and H2: its shape is the whole file system. -/
theorem demo_lawful : demo.Lawful where
  read_fails := fun _ h => by simp only [demo, id] at h; rw [h]
  write_shape := fun _ _ _ h => by simp only [demo, id] at h; rw [h]; rfl
  resp_typed := fun _ vs _ _ _ h => by
    simp only [demo] at h
    split at h <;> cases h
    simp [Val.IsResponse]

/-- The empty world. -/
def w₀ : World demo := ⟨[], ()⟩

/-- A program under the old rules: `Console` accepts `public`, `Files.read` `public`,
`Files.write` and `Http` `internal`. -/
def oldProg : Program where
  categories := []
  acceptsOp := Label.bot
  acceptsRead := Label.bot
  acceptsWrite := ⟨.internal, []⟩
  acceptsReq := ⟨.internal, []⟩
  readResult := ⟨.internal, []⟩
  reqResult := ⟨.internal, []⟩
  reqFloor := ⟨.internal, []⟩
  fileState := Label.bot
  oldHost := true
  records := [⟨"prelude.Header", [.text .bot, .text intl], ⟨.internal, []⟩⟩,
    ⟨"prelude.Response", [.int .bot, .list (.record "prelude.Header" .bot) .bot, .text .bot], ⟨.internal, []⟩⟩]
  policies := []
  funs := []

/-- `x`, an `internal` text the environment gives: the secret path of S-14, the body of S-15. -/
def Γ : LCtx := [("x", .text intl)]

/-- Print `yes` or `no` by whether the result `r` holds a value. -/
def branch : Expr :=
  .caseResult (.var "r") "v"
    (.letR "o" (.unit .bot) (.op (.text "yes")) (.atom (.var "o"))) "e"
    (.letR "o" (.unit .bot) (.op (.text "no")) (.atom (.var "o")))

/-- **S-14's probe**: write to the secret path, then read `a.txt`, and print whether it exists. -/
def s14 : Expr :=
  .letE "w" (.result (.unit .bot) intl)
    (.attempt (.letR "u" (.unit .bot) (.write (.var "x") (.text "x")) (.atom (.var "u"))))
    (.letE "r" (.result (.text intl) .bot)
      (.attempt (.letR "c" (.text intl) (.read (.text "a.txt")) (.atom (.var "c"))))
      branch)

/-- **S-15's probe**: send the secret as a request's body, with no header and no key name, and
print whether it failed. -/
def s15 : Expr :=
  .letE "r" (.result (.record "prelude.Response" intl) .bot)
    (.attempt (.letR "h" (.list (.record "prelude.Header" .bot) .bot) (.list [])
      (.letR "k" (.option (.text .bot) .bot) .none
        (.letR "c" (.record "prelude.Response" intl)
          (.req "post" [.text "https://example.com", .var "h", .var "x", .var "k"])
          (.atom (.var "c"))))))
    branch

/-- The environment with the secret `s`. -/
def env (s : String) : Env := [("x", .text s)]

theorem le_of {a b : LExpr} (h : Una.F1.Check.leB oldProg.top a b = true) : oldProg.le a b :=
  Una.F1.Check.leB_sound h

theorem sf {e : LExpr} (h : Una.F1.Check.symFree e = true) : e.SymFree := Una.F1.Check.symFree_sound h

/-- Under the old rules, S-14's probe has a typing. -/
theorem s14_old_typed : ∃ t ε, LExprTy oldProg [] Γ .bot s14 t ε := by
  have hw : LExprTy oldProg [] Γ .bot
      (.letR "u" (.unit .bot) (.write (.var "x") (.text "x")) (.atom (.var "u"))) _ _ :=
    .letR (.write (.var rfl) rfl .text rfl (le_of rfl) (by simp [oldProg]) (sf rfl)) .unit
      (.atom (.var rfl)) rfl
  have hr : LExprTy oldProg [] (("w", .result (.unit .bot) intl) :: Γ) (LExpr.bot.join LExpr.bot)
      (.letR "c" (.text intl) (.read (.text "a.txt")) (.atom (.var "c"))) _ _ :=
    .letR (.read .text rfl (le_of rfl) (sf rfl)) (.text (le_of rfl)) (.atom (.var rfl)) rfl
  have hall : LExprTy oldProg [] Γ .bot s14 _ _ :=
    .letE (x := "w") (ann := .result (.unit .bot) intl) (.attempt hw) (.result .unit (le_of rfl))
      (.letE (x := "r") (ann := .result (.text intl) .bot) (.attempt hr)
        (.result (.text (le_of rfl)) (le_of rfl))
        (.caseResult (s := .unit .bot) (.var rfl)
          (.letR (.op .text trivial (le_of rfl) (sf rfl)) .unit (.atom (.var rfl)) rfl)
          (.letR (.op .text trivial (le_of rfl) (sf rfl)) .unit (.atom (.var rfl)) rfl)
          .unit .unit rfl)
        (by simp [LTy.InPlace, intl, LExpr.ofLabel, LExpr.bot]))
      (by simp [LTy.InPlace, intl, LExpr.ofLabel, LExpr.bot])
  exact ⟨_, _, hall⟩

/-- **S-14 in the model, under the old rules**: the two runs differ only in the `internal` path,
start from the same world, and print different lines at `Console`, which accepts `public`. -/
theorem s14_old_leaks :
    (eval demo 10 oldProg w₀ (env "a.txt") s14).1.filter (· matches .out _) = [.out "yes"] ∧
    (eval demo 10 oldProg w₀ (env "b.txt") s14).1.filter (· matches .out _) = [.out "no"] := by
  constructor <;> rfl

unseal Check.flowB Check.flowsB in
/-- Under the old rules, S-15's probe has a typing: the checker accepts it. -/
theorem s15_old_typed : ∃ t ε, LExprTy oldProg [] Γ .bot s15 t ε := by
  obtain ⟨t, ht, _⟩ := Check.checkExpr_sound (p := oldProg) (E := []) (Γ := Γ) (pc := .bot)
    (s := .unit .bot) (e := s15) (ε := _) rfl
  exact ⟨t, _, ht⟩

/-- **S-15 in the model, under the old rules**: the server fails the request whose body is `"1"`,
so two runs that differ only in the `internal` body print different lines at `Console`. -/
theorem s15_old_leaks :
    (eval demo 10 oldProg w₀ (env "1") s15).1.filter (· matches .out _) = [.out "no"] ∧
    (eval demo 10 oldProg w₀ (env "2") s15).1.filter (· matches .out _) = [.out "yes"] := by
  constructor <;> rfl

end Una.F1.OldLeaks
