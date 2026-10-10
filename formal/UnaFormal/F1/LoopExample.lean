/-
Non-vacuity of M6 (#478, `docs/plan/formal-track-m6.md` §9):

- `sum_runs`: a function that sums a `public` list of `internal` integers in a loop, with an
  `internal` state, is certified by the whole-program check (`certified`), and `t6_host_ground`
  relates its runs on two lists that differ in their elements, whose sums differ.
- `bump`: a function with an `edit` parameter, desugared to a tuple result, and its caller, which
  binds the tuple with `let_tuple`, are certified too.
- `public_count_untyped`: a loop over a list of `internal` length cannot hand its state to a
  `public` binder: the loop's value is raised by the list's top label.
-/
import UnaFormal.F1.Import

namespace Una.F1.LoopExample

open Check

def intl : LExpr := .ofLabel ⟨.internal, []⟩

/-- `sum(xs: List[internal Int]) -> internal Int`: `let edit acc = 0; for x in xs: acc = acc + x`. -/
def sumBody : Expr :=
  .letE "r" (.tuple [.int intl] .bot)
    (.loop (.list (.var "xs")) "x" (.int intl) [("acc", .int intl)] [.int 0]
      (.letR "a" (.int intl) (.prim .add [.var "acc", .var "x"])
        (.letR "t" (.tuple [.int intl] .bot) (.tuple [.var "a"]) (.atom (.var "t")))))
    (.letTuple [("acc2", .int intl)] (.var "r") (.atom (.var "acc2")))

def sumFn : Fun where
  name := "m.sum"
  tparams := []
  eqParams := []
  params := [("xs", .list (.int intl) .bot)]
  result := .int intl
  op := false
  host := {}
  fails := false
  body := sumBody

/-- `bump(edit n: internal Int)`, desugared: it returns the new `n` and `unit`. -/
def bumpFn : Fun where
  name := "m.bump"
  tparams := []
  eqParams := []
  params := [("n", .int intl)]
  result := .tuple [.int intl, .unit .bot] .bot
  op := false
  host := {}
  fails := false
  body := .letR "n2" (.int intl) (.prim .add [.var "n", .int 1])
    (.letR "t" (.tuple [.int intl, .unit .bot] .bot) (.tuple [.var "n2", .unit]) (.atom (.var "t")))

/-- `twice(n: internal Int) -> internal Int`: `let edit m = n; bump(edit m); bump(edit m); m`. -/
def twiceFn : Fun where
  name := "m.twice"
  tparams := []
  eqParams := []
  params := [("n", .int intl)]
  result := .int intl
  op := false
  host := {}
  fails := false
  body :=
    .letR "r1" (.tuple [.int intl, .unit .bot] .bot) (.call "m.bump" [] [.var "n"])
      (.letTuple [("m1", .int intl), ("u1", .unit .bot)] (.var "r1")
        (.letR "r2" (.tuple [.int intl, .unit .bot] .bot) (.call "m.bump" [] [.var "m1"])
          (.letTuple [("m2", .int intl), ("u2", .unit .bot)] (.var "r2") (.atom (.var "m2")))))

def labels : Import.HostLabels :=
  { acceptsReq := ⟨.internal, []⟩, reqResult := ⟨.internal, []⟩, fileState := some Label.bot }

def prog : Program := Import.mkProgram [] Label.bot labels [] [] [sumFn, bumpFn, twiceFn]

unseal flowB flowsB in
/-- **The whole-program check certifies the loop and the `edit` parameter.** -/
theorem certified : prog.LWellFormed ∧ prog.B1WellFormed ∧ prog.HostWF :=
  progOk_sound_nil (by rfl)

unseal flowB flowsB in
theorem sum_checked :
    ∃ ε, checkExpr prog [] [("xs", .list (.int intl) .bot)] .bot (.int intl) sumBody = .ok ε :=
  ⟨_, rfl⟩

def env (a b : Int) : Env := [("xs", .list [.int a, .int b])]

theorem internal_hidden : ¬ Vis prog Label.bot intl := by
  rintro (h | ⟨h, _⟩) <;>
    simp [prog, Import.mkProgram, Program.top, intl, LExpr.ofLabel, Label.le, Label.bot] at h <;>
    revert h <;> decide

/-- **T6 for a loop** (M6): runs over `[1, 2]` and `[3, 4]`, whose elements a `public` observer
does not see, show it the same events and related outcomes. -/
theorem sum_runs : ∃ t ε, LExprTy prog [] [("xs", .list (.int intl) .bot)] .bot sumBody t ε ∧
    ∀ n : Nat, Term (eval Host.unit n prog ⟨(), ()⟩ (env 1 2) sumBody).2.1 →
      Term (eval Host.unit n prog ⟨(), ()⟩ (env 3 4) sumBody).2.1 →
      proj prog Label.bot (eval Host.unit n prog ⟨(), ()⟩ (env 1 2) sumBody).1 =
          proj prog Label.bot (eval Host.unit n prog ⟨(), ()⟩ (env 3 4) sumBody).1 ∧
        OutRel (V prog Label.bot t) (Vis prog Label.bot ε.failLabel)
          (eval Host.unit n prog ⟨(), ()⟩ (env 1 2) sumBody).2.1
          (eval Host.unit n prog ⟨(), ()⟩ (env 3 4) sumBody).2.1 := by
  obtain ⟨hwf, _, hh⟩ := certified
  obtain ⟨ε, hc⟩ := sum_checked
  obtain ⟨t, ht, _⟩ := checkExpr_sound hc
  refine ⟨t, ε, ht, fun n h₁ h₂ => ?_⟩
  have hrel : EnvRel prog Label.bot [("xs", .list (.int intl) .bot)] (env 1 2) (env 3 4) :=
    .cons (.list (.cons (.hidden internal_hidden) (.cons (.hidden internal_hidden) .nil))) .nil
  have hty : ∀ a b : Int, LEnvTy prog (env a b) [("xs", .list (.int intl) .bot)] :=
    fun _ _ => .cons (.list (.cons .int (.cons .int .nil))) .nil
  obtain ⟨hp, ho, _⟩ := t6_host_ground (Γ := [("xs", .list (.int intl) .bot)]) hwf rfl hh
    Host.unit_lawful (fun q hq => by simp at hq; subst hq; exact ⟨rfl, rfl⟩) rfl (WorldRel.refl _)
    (hty 1 2) (hty 3 4) hrel ht n h₁ h₂
  exact ⟨hp, ho⟩

/-- The runs end with different sums, 3 and 7. -/
example : (eval Host.unit 10 prog ⟨(), ()⟩ (env 1 2) sumBody).2.1 = .ok (.int 3) ∧
    (eval Host.unit 10 prog ⟨(), ()⟩ (env 3 4) sumBody).2.1 = .ok (.int 7) := ⟨rfl, rfl⟩

/-- `twice 5` is 7, through two `edit` calls. -/
example : (eval Host.unit 10 prog ⟨(), ()⟩ [("n", .int 5)] twiceFn.body).2.1 = .ok (.int 7) := rfl

/-- A loop over a list of `internal` length, whose state is `public`, handed to a `public` binder. -/
def countBody : Expr :=
  .letE "r" (.tuple [.int .bot] intl)
    (.loop (.list (.var "xs")) "x" (.int .bot) [("c", .int .bot)] [.int 0]
      (.letR "c1" (.int .bot) (.prim .add [.var "c", .int 1])
        (.letR "t" (.tuple [.int .bot] .bot) (.tuple [.var "c1"]) (.atom (.var "t")))))
    (.letTuple [("n", .int .bot)] (.var "r") (.atom (.var "n")))

/-- **The count of an `internal`-length list cannot be `public`.** -/
theorem public_count_untyped {E : List String} {t : LTy} {ε : Eff} :
    ¬ LExprTy prog E [("xs", .list (.int .bot) intl)] .bot countBody t ε := by
  intro h
  cases h with
  | letE _ _ hb _ =>
    cases hb with
    | letTuple hv hflows _ _ =>
      cases hv with
      | var hl =>
        simp [lctxLookup] at hl
        obtain ⟨rfl, rfl⟩ := hl
        cases hflows with
        | cons hf _ =>
          cases hf with
          | int hle =>
            apply internal_hidden
            exact LExpr.le_trans (LExpr.le_join_right _ _) hle
end Una.F1.LoopExample
