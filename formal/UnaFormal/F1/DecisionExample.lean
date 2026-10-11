/-
Non-vacuity of ruling D (#524, `docs/plan/formal-track-handlers.md` §4a): **what decides a
failure is not where it happens**, on the lawful file host of `HostExamples`.

`HandlerExample` shows these shapes with `parse`, whose successful runs a proof cannot evaluate
(`String.toInt?` does not reduce), so their leaks are shown here with `Files.read`, whose failure
is decided by the path, as `parse`'s is by its text. The paths `s` and `t` are `internal`; the
program's `Files` accepts `internal`, and its file-state label is `public`.

- **A secret failure at a public program counter** (`secretAtPublic`): `attempt read(s)`,
  annotated public. The read's decision label is `s`'s, so the annotation is rejected; it leaks
  (`Ok` and `Err` at a public type), and it has no typing at it.
- **Read, then attempt** (`readThen`): `let y = read(s) in attempt read(t)`. `y`'s failure raises
  the program counter to `internal`, so the program counter alone (ruling B) would accept the
  public annotation; the decision label rejects it, and it leaks, with `s` the same in both runs.
-/
import UnaFormal.F1.HostExamples

namespace Una.F1.DecisionExample

open HostExamples (files files_lawful)

def intl : LExpr := .ofLabel ⟨.internal, []⟩

/-- `Files` accepts `internal`; the file state is `public`. -/
def prog : Program where
  categories := []
  acceptsOp := Label.bot
  acceptsRead := ⟨.internal, []⟩
  acceptsWrite := ⟨.internal, []⟩
  acceptsReq := ⟨.internal, []⟩
  readResult := ⟨.internal, []⟩
  reqResult := ⟨.internal, []⟩
  reqFloor := ⟨.internal, []⟩
  fileState := Label.bot
  records := []
  policies := []
  funs := []

unseal Check.flowB Check.flowsB in
theorem certified : prog.LWellFormed ∧ prog.B1WellFormed ∧ prog.HostWF :=
  Check.progOk_sound_nil (by rfl)

theorem intl_hidden : ¬ Vis prog Label.bot intl := by
  rintro (h | ⟨h, _⟩) <;>
    simp [prog, Program.top, LExpr.ofLabel, Label.le, Label.bot] at h <;>
    revert h <;> decide

/-- The secret paths. -/
def Γ : LCtx := [("s", .text intl), ("t", .text intl)]

def env (s t : String) : Env := [("s", .text s), ("t", .text t)]

/-- One file, `a`. -/
def w : World files := ⟨([("a", "1")] : List (String × String)), (), fun _ => none⟩

theorem w_rel : WorldRel prog Label.bot w w :=
  ⟨fun _ => rfl, fun _ => rfl, fun _ => rfl⟩

theorem env_typed (s t : String) : LEnvTy prog (env s t) Γ := .cons .text (.cons .text .nil)

theorem env_rel (s t s' t' : String) : EnvRel prog Label.bot Γ (env s t) (env s' t') :=
  .cons (.hidden intl_hidden) (.cons (.hidden intl_hidden) .nil)

theorem Γ_inPlace : Γ.InPlace := fun q hq => by
  simp only [Γ, List.mem_cons, List.mem_singleton, List.not_mem_nil, or_false] at hq
  rcases hq with rfl | rfl <;> rfl

/-- The result of `attempt read(…)`, public. -/
def resPub : LTy := .result (.text intl) (.int .bot) .bot

/-- `attempt read(v)`. -/
def attemptRead (v : String) : Expr :=
  .attempt (.letR "c" (.text intl) (.read (.var v)) (.atom (.var "c")))

/-- A failure and a value of a public result are told apart. -/
theorem not_outRel_pub {vf : Prop} {a b : Val} :
    ¬ OutRel prog Label.bot (V prog Label.bot resPub) vf (.ok (.err a)) (.ok (.ok b)) := by
  intro h
  simp only [OutRel] at h
  cases h with
  | hidden hv => exact hv (by simp only [resPub, LTy.top]; exact Vis.bot)

/-- The outcomes T6 relates, for a typing in `Γ` at `⊥`. -/
theorem outcomes {e : Expr} {t : LTy} {ε : Eff} (ht : LExprTy prog [] Γ .bot e t ε)
    (s t' s' t'' : String) (n : Nat)
    (h₁ : Term (eval files n prog w (env s t') e).2.1) (h₂ : Term (eval files n prog w (env s' t'') e).2.1) :
    OutRel prog Label.bot (V prog Label.bot t) (Vis prog Label.bot ε.failLabel)
      (eval files n prog w (env s t') e).2.1 (eval files n prog w (env s' t'') e).2.1 := by
  obtain ⟨hwf, _, hh⟩ := certified
  exact (t6_host_ground hwf rfl hh files_lawful Γ_inPlace rfl w_rel (env_typed _ _) (env_typed _ _)
    (env_rel _ _ _ _) ht n h₁ h₂).2.1

/-! ### A secret failure at a public program counter -/

def secretAtPublic : Expr := attemptRead "s"

unseal Check.flowB Check.flowsB in
theorem secretAtPublic_rejected :
    ∃ msg, Check.checkExpr prog [] Γ .bot resPub secretAtPublic = .error msg := ⟨_, rfl⟩

/-- Labeled by its decision label, `internal`, the same code is certified. -/
unseal Check.flowB Check.flowsB in
theorem secretAtPublic_checked_internal :
    ∃ ε, Check.checkExpr prog [] Γ .bot (.result (.text intl) (.int .bot) intl) secretAtPublic = .ok ε :=
  ⟨_, rfl⟩

theorem secretAtPublic_leaks :
    (eval files 10 prog w (env "b" "") secretAtPublic).2.1 = .ok (.err (.int 0)) ∧
    (eval files 10 prog w (env "a" "") secretAtPublic).2.1 = .ok (.ok (.text "1")) := ⟨rfl, rfl⟩

theorem secretAtPublic_untyped {ε : Eff} : ¬ LExprTy prog [] Γ .bot secretAtPublic resPub ε := by
  intro ht
  have h := outcomes ht "b" "" "a" "" 10 (by exact True.intro) (by exact True.intro)
  rw [secretAtPublic_leaks.1, secretAtPublic_leaks.2] at h
  exact not_outRel_pub h

/-! ### Read, then attempt: why the program counter alone is not enough -/

/-- `let y = read(s) in attempt read(t)`. -/
def readThen : Expr :=
  .letR "y" (.text intl) (.read (.var "s")) (attemptRead "t")

unseal Check.flowB Check.flowsB in
theorem readThen_rejected :
    ∃ msg, Check.checkExpr prog [] Γ .bot resPub readThen = .error msg := ⟨_, rfl⟩

/-- The first read succeeds in both runs (`s = a`); the second decides the outcome. -/
theorem readThen_leaks :
    (eval files 10 prog w (env "a" "b") readThen).2.1 = .ok (.err (.int 0)) ∧
    (eval files 10 prog w (env "a" "a") readThen).2.1 = .ok (.ok (.text "1")) := ⟨rfl, rfl⟩

theorem readThen_untyped {ε : Eff} : ¬ LExprTy prog [] Γ .bot readThen resPub ε := by
  intro ht
  have h := outcomes ht "a" "b" "a" "a" 10 (by exact True.intro) (by exact True.intro)
  rw [readThen_leaks.1, readThen_leaks.2] at h
  exact not_outRel_pub h

end Una.F1.DecisionExample
