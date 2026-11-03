/-
Non-vacuity of ADR 0116's slice of F1 (#946): exits, `for until`, `for ever` with `on stop`, and
the host's stop schedule (`Host.stopAt`).

- `halve`: a `for until` loop over an `internal` state, whose condition is `internal`, is certified
  by the whole-program check (`certified`), and `t6_host_ground` relates its runs on two inputs
  whose iteration counts differ (`halve_runs`, `halve_counts`).
- `tick`: a `for ever` loop with `stop when` and `on stop` ends by its `stop` on a host that never
  stops it, and at the schedule's stop point on a host that does (`tick_stops`, `tick_host_stop`).
- `ticks`: a `for ever` loop that adds an `internal` value to its state, on a host whose schedule
  stops it at its second iteration: `t6_host_ground` relates its runs on two values
  (`ticks_runs`), whose sums differ.
- A `stop` decided by `internal` data is rejected (U4002, IF2), and a `skip` under an `internal`
  branch, which IF3 allows and F1's T-Exit does not, is a restriction, never accepted.
- The host-driven step (ADR 0116 rules 1 to 4, `spec/26 §2`): under a fixed stop schedule, the
  driver delivers a finite list of ticks, chosen by the host, so its run is the Core program
  `driveExpr`: `init` once, `step` per tick, then `on_stop`. `drive_runs` relates two runs whose
  ticks differ in times the observer does not see, as IF7 labels them; `drive_checked` is the
  re-check's verdict on it.
-/
import UnaFormal.F1.Import

namespace Una.F1.LongRunExample

open Check

def intl : LExpr := .ofLabel ⟨.internal, []⟩

/-! ### `for until` -/

/-- `halve(n: internal Int) -> internal Int`:
`let edit left = n; let edit count = 0; for until left == 0: left = left / 2; count = count + 1`,
returning `count`. The measure `left` is the verifier's, not part of Core's syntax. -/
def halveBody : Expr :=
  .letE "r" (.tuple [.int intl, .int intl] .bot)
    (.loopUntil
      (.letR "c" (.bool intl) (.prim .eqInt [.var "left", .int 0]) (.atom (.var "c")))
      [("left", .int intl), ("count", .int intl)] [.var "n", .int 0]
      (.letR "l2" (.int intl) (.prim .div [.var "left", .int 2])
        (.letR "c2" (.int intl) (.prim .add [.var "count", .int 1])
          (.letR "t" (.tuple [.int intl, .int intl] .bot) (.tuple [.var "l2", .var "c2"])
            (.atom (.var "t"))))))
    (.letTuple [("l", .int intl), ("k", .int intl)] (.var "r") (.atom (.var "k")))

def halveFn : Fun where
  name := "m.halve"
  tparams := []
  eqParams := []
  params := [("n", .int intl)]
  result := .int intl
  op := false
  host := {}
  fails := false
  body := halveBody

/-! ### `for ever` with `stop when` and `on stop` -/

/-- `tick() -> Int`: `let edit n = 0; for ever: stop when n == 3; n = n + 1` with
`on stop: n = n + 100`, returning `n`. -/
def tickBody : Expr :=
  .letE "r" (.tuple [.int .bot] .bot)
    (.loopEver 7 [("n", .int .bot)] [.int 0]
      (.letR "s" (.bool .bot) (.prim .eqInt [.var "n", .int 3])
        (.ite (.var "s") (.exit true [.var "n"])
          (.letR "n2" (.int .bot) (.prim .add [.var "n", .int 1])
            (.letR "t" (.tuple [.int .bot] .bot) (.tuple [.var "n2"]) (.atom (.var "t"))))))
      [("m", .int .bot)]
      (.letR "m2" (.int .bot) (.prim .add [.var "m", .int 100])
        (.letR "t" (.tuple [.int .bot] .bot) (.tuple [.var "m2"]) (.atom (.var "t")))))
    (.letTuple [("v", .int .bot)] (.var "r") (.atom (.var "v")))

def tickFn : Fun where
  name := "m.tick"
  tparams := []
  eqParams := []
  params := []
  result := .int .bot
  op := false
  host := {}
  fails := false
  body := tickBody

/-- `ticks(x: internal Int) -> internal Int`: `let edit i = 0; let edit acc = 0; for ever:
stop when i == 5; i = i + 1; acc = acc + x`, with no `on stop`, returning `acc`. -/
def ticksBody : Expr :=
  .letE "r" (.tuple [.int .bot, .int intl] .bot)
    (.loopEver 9 [("i", .int .bot), ("acc", .int intl)] [.int 0, .int 0]
      (.letR "s" (.bool .bot) (.prim .eqInt [.var "i", .int 5])
        (.ite (.var "s") (.exit true [.var "i", .var "acc"])
          (.letR "i2" (.int .bot) (.prim .add [.var "i", .int 1])
            (.letR "a2" (.int intl) (.prim .add [.var "acc", .var "x"])
              (.letR "t" (.tuple [.int .bot, .int intl] .bot) (.tuple [.var "i2", .var "a2"])
                (.atom (.var "t")))))))
      [("i", .int .bot), ("acc", .int intl)]
      (.letR "t" (.tuple [.int .bot, .int intl] .bot) (.tuple [.var "i", .var "acc"])
        (.atom (.var "t"))))
    (.letTuple [("j", .int .bot), ("a", .int intl)] (.var "r") (.atom (.var "a")))

def ticksFn : Fun where
  name := "m.ticks"
  tparams := []
  eqParams := []
  params := [("x", .int intl)]
  result := .int intl
  op := false
  host := {}
  fails := false
  body := ticksBody

/-! ### The host-driven step -/

/-- The driver's loop (ADR 0116 rules 3 and 4) under a fixed stop schedule and fixed tick times:
the host delivers the list `%ticks` of ticks before its stop, so the `for ever` of rule 4 runs
one iteration per tick, and the loop is a Core `loop` over that list. `init` gives the state,
`step(edit state, input)`, desugared to the new state and `Unit`, gives the next one, and
`on_stop` runs on the last; the state is its value. `S` is the state's type and `T` the type of
`step`'s `input`, which IF7 labels at least the step's timing label. -/
def driveExpr (S T : LTy) : Expr :=
  .letR "%s0" S (.call "m.init" [] [])
    (.letE "%r" (.tuple [S] .bot)
      (.loop (.list (.var "%ticks")) "%tick" T [("%s", S)] [.var "%s0"]
        (.letR "%q" (.tuple [S, .unit .bot] .bot) (.call "m.step" [] [.var "%s", .var "%tick"])
          (.letTuple [("%s2", S), ("%u", .unit .bot)] (.var "%q")
            (.letR "%t" (.tuple [S] .bot) (.tuple [.var "%s2"]) (.atom (.var "%t"))))))
      (.letTuple [("%s3", S)] (.var "%r")
        (.letR "%v" (.unit .bot) (.call "m.on_stop" [] [.var "%s3"]) (.atom (.var "%s3")))))

/-- `init() -> internal Int`: `0`. -/
def initFn : Fun where
  name := "m.init"
  tparams := []
  eqParams := []
  params := []
  result := .int intl
  op := false
  host := {}
  fails := false
  body := .atom (.int 0)

/-- `step(edit state: internal Int, input: internal Int)`: `state = state + input`, the tick's
time standing for the `Tick` (an `internal` scalar: IF7 labels a step's input at least its timing
label, and the fields of `Tick` are scalars). -/
def stepFn : Fun where
  name := "m.step"
  tparams := []
  eqParams := []
  params := [("state", .int intl), ("input", .int intl)]
  result := .tuple [.int intl, .unit .bot] .bot
  op := false
  host := {}
  fails := false
  body := .letR "s2" (.int intl) (.prim .add [.var "state", .var "input"])
    (.letR "t" (.tuple [.int intl, .unit .bot] .bot) (.tuple [.var "s2", .unit]) (.atom (.var "t")))

/-- `on_stop(state: internal Int)`: prints `"stopped"`, a `public` line. -/
def onStopFn : Fun where
  name := "m.on_stop"
  tparams := []
  eqParams := []
  params := [("state", .int intl)]
  result := .unit .bot
  op := true
  host := {}
  fails := false
  body := .letR "u" (.unit .bot) (.op (.text "stopped")) (.atom (.var "u"))

def labels : Import.HostLabels :=
  { acceptsReq := ⟨.internal, []⟩, reqResult := ⟨.internal, []⟩, fileState := some Label.bot }

def prog : Program :=
  Import.mkProgram [] Label.bot labels [] [] [halveFn, tickFn, ticksFn, initFn, stepFn, onStopFn]

unseal flowB flowsB in
/-- **The whole-program check certifies the three loops.** -/
theorem certified : prog.LWellFormed ∧ prog.B1WellFormed ∧ prog.HostWF :=
  progOk_sound_nil (by rfl)

def w₀ : World Host.unit := ⟨(), (), fun _ => none⟩

theorem internal_hidden : ¬ Vis prog Label.bot intl := by
  rintro (h | ⟨h, _⟩) <;>
    simp [prog, Import.mkProgram, Program.top, intl, LExpr.ofLabel, Label.le, Label.bot] at h <;>
    revert h <;> decide

unseal flowB flowsB in
theorem halve_checked :
    ∃ ε, checkExpr prog [] [("n", .int intl)] .bot (.int intl) halveBody = .ok ε :=
  ⟨_, rfl⟩

/-- **T6 for a `for until` loop** (ADR 0116 IF1): runs on `8` and on `1`, which a `public`
observer does not see, show it the same events and related outcomes, although their iteration
counts differ (`halve_counts`). -/
theorem halve_runs : ∃ t ε, LExprTy prog [] [("n", .int intl)] .bot halveBody t ε ∧
    ∀ n : Nat, Term (eval Host.unit n prog w₀ [("n", .int 8)] halveBody).2.1 →
      Term (eval Host.unit n prog w₀ [("n", .int 1)] halveBody).2.1 →
      proj prog Label.bot (eval Host.unit n prog w₀ [("n", .int 8)] halveBody).1 =
          proj prog Label.bot (eval Host.unit n prog w₀ [("n", .int 1)] halveBody).1 ∧
        OutRel prog Label.bot (V prog Label.bot t) (Vis prog Label.bot ε.failLabel)
          (eval Host.unit n prog w₀ [("n", .int 8)] halveBody).2.1
          (eval Host.unit n prog w₀ [("n", .int 1)] halveBody).2.1 := by
  obtain ⟨hwf, _, hh⟩ := certified
  obtain ⟨ε, hc⟩ := halve_checked
  obtain ⟨t, ht, _⟩ := checkExpr_sound hc
  refine ⟨t, ε, ht, fun n h₁ h₂ => ?_⟩
  have hrel : EnvRel prog Label.bot [("n", .int intl)] [("n", .int 8)] [("n", .int 1)] :=
    .cons (.hidden internal_hidden) .nil
  have hty : ∀ a : Int, LEnvTy prog [("n", .int a)] [("n", .int intl)] := fun _ => .cons .int .nil
  obtain ⟨hp, ho, _⟩ := t6_host_ground (Γ := [("n", .int intl)]) hwf rfl hh
    Host.unit_lawful (fun q hq => by simp at hq; subst hq; simp [LTy.InPlace, intl, LExpr.ofLabel]) rfl (WorldRel.refl _)
    (hty 8) (hty 1) hrel ht n h₁ h₂
  exact ⟨hp, ho⟩

/-- The runs end with different counts: `8` halves 4 times to `0`, and `1` once. -/
theorem halve_counts : (eval Host.unit 10 prog w₀ [("n", .int 8)] halveBody).2.1 = .ok (.int 4) ∧
    (eval Host.unit 10 prog w₀ [("n", .int 1)] halveBody).2.1 = .ok (.int 1) := ⟨rfl, rfl⟩

/-! ### `for ever` and the host's stop schedule -/

/-- A host whose stop schedule stops every `for ever` loop at the start of its iteration `k`. -/
def stopping (k : Nat) : Host := { Host.unit with stopAt := fun _ i => i == k }

theorem stopping_lawful (k : Nat) : (stopping k).Lawful :=
  ⟨fun _ _ => Iff.rfl, fun _ _ _ _ => rfl, fun _ _ _ _ _ h => nomatch h⟩

def u₀ (k : Nat) : World (stopping k) := ⟨(), (), fun _ => none⟩

/-- **`stop` ends a `for ever`, then `on stop` runs** (ADR 0116 rules 6 and 10): on a host that
never stops it, `tick` stops when `n` is `3`, and `on stop` adds `100`. -/
theorem tick_stops : (eval Host.unit 10 prog w₀ [] tickBody).2.1 = .ok (.int 103) := rfl

/-- **A host stop ends a `for ever` at its stop point** (ADR 0116 rules 13 and 14): on a host
whose schedule stops it at the start of iteration `1`, `tick` ends there with `n = 1`, and
`on stop` runs on that state. -/
theorem tick_host_stop : (eval (stopping 1) 10 prog (u₀ 1) [] tickBody).2.1 = .ok (.int 101) := rfl

unseal flowB flowsB in
theorem ticks_checked :
    ∃ ε, checkExpr prog [] [("x", .int intl)] .bot (.int intl) ticksBody = .ok ε :=
  ⟨_, rfl⟩

/-- **T6 for a `for ever` loop under a fixed stop schedule** (ADR 0116, "Where this sits in the
model"): on a host whose schedule stops the loop at its second iteration, runs on `5` and `7`,
which a `public` observer does not see, show it the same events and related outcomes. -/
theorem ticks_runs : ∃ t ε, LExprTy prog [] [("x", .int intl)] .bot ticksBody t ε ∧
    ∀ n : Nat, Term (eval (stopping 2) n prog (u₀ 2) [("x", .int 5)] ticksBody).2.1 →
      Term (eval (stopping 2) n prog (u₀ 2) [("x", .int 7)] ticksBody).2.1 →
      proj prog Label.bot (eval (stopping 2) n prog (u₀ 2) [("x", .int 5)] ticksBody).1 =
          proj prog Label.bot (eval (stopping 2) n prog (u₀ 2) [("x", .int 7)] ticksBody).1 ∧
        OutRel prog Label.bot (V prog Label.bot t) (Vis prog Label.bot ε.failLabel)
          (eval (stopping 2) n prog (u₀ 2) [("x", .int 5)] ticksBody).2.1
          (eval (stopping 2) n prog (u₀ 2) [("x", .int 7)] ticksBody).2.1 := by
  obtain ⟨hwf, _, hh⟩ := certified
  obtain ⟨ε, hc⟩ := ticks_checked
  obtain ⟨t, ht, _⟩ := checkExpr_sound hc
  refine ⟨t, ε, ht, fun n h₁ h₂ => ?_⟩
  have hrel : EnvRel prog Label.bot [("x", .int intl)] [("x", .int 5)] [("x", .int 7)] :=
    .cons (.hidden internal_hidden) .nil
  have hty : ∀ a : Int, LEnvTy prog [("x", .int a)] [("x", .int intl)] := fun _ => .cons .int .nil
  obtain ⟨hp, ho, _⟩ := t6_host_ground (Γ := [("x", .int intl)]) hwf rfl hh
    (stopping_lawful 2) (fun q hq => by simp at hq; subst hq; simp [LTy.InPlace, intl, LExpr.ofLabel]) rfl
    (WorldRel.refl _) (hty 5) (hty 7) hrel ht n h₁ h₂
  exact ⟨hp, ho⟩

/-- The runs end at the schedule's stop point, before their own `stop`, with different sums. -/
theorem ticks_sums :
    (eval (stopping 2) 10 prog (u₀ 2) [("x", .int 5)] ticksBody).2.1 = .ok (.int 10) ∧
    (eval (stopping 2) 10 prog (u₀ 2) [("x", .int 7)] ticksBody).2.1 = .ok (.int 14) := ⟨rfl, rfl⟩

/-! ### The driver's runs -/

def Γdrive : LCtx := [("%ticks", .list (.int intl) .bot)]

unseal flowB flowsB in
/-- The re-check accepts the driver's program for this module. -/
theorem drive_checked :
    ∃ ε, checkExpr prog [] Γdrive .bot (.int intl) (driveExpr (.int intl) (.int intl)) = .ok ε :=
  ⟨_, rfl⟩

def ticks (xs : List Int) : Env := [("%ticks", .list (xs.map Val.int))]

/-- **T6 for the host-driven step** (ADR 0116 rules 1 to 4, IF7): under one stop schedule, which
delivers two ticks, runs whose ticks came at different times, `16, 32` and `17, 40`, which the
observer does not see since IF7 labels them, show it the same events, `on_stop`'s line included,
and related outcomes. -/
theorem drive_runs : ∃ t ε, LExprTy prog [] Γdrive .bot (driveExpr (.int intl) (.int intl)) t ε ∧
    ∀ n : Nat, Term (eval Host.unit n prog w₀ (ticks [16, 32]) (driveExpr (.int intl) (.int intl))).2.1 →
      Term (eval Host.unit n prog w₀ (ticks [17, 40]) (driveExpr (.int intl) (.int intl))).2.1 →
      proj prog Label.bot (eval Host.unit n prog w₀ (ticks [16, 32]) (driveExpr (.int intl) (.int intl))).1 =
          proj prog Label.bot (eval Host.unit n prog w₀ (ticks [17, 40]) (driveExpr (.int intl) (.int intl))).1 ∧
        OutRel prog Label.bot (V prog Label.bot t) (Vis prog Label.bot ε.failLabel)
          (eval Host.unit n prog w₀ (ticks [16, 32]) (driveExpr (.int intl) (.int intl))).2.1
          (eval Host.unit n prog w₀ (ticks [17, 40]) (driveExpr (.int intl) (.int intl))).2.1 := by
  obtain ⟨hwf, _, hh⟩ := certified
  obtain ⟨ε, hc⟩ := drive_checked
  obtain ⟨t, ht, _⟩ := checkExpr_sound hc
  refine ⟨t, ε, ht, fun n h₁ h₂ => ?_⟩
  have hrel : EnvRel prog Label.bot Γdrive (ticks [16, 32]) (ticks [17, 40]) :=
    .cons (.list (.cons (.hidden internal_hidden) (.cons (.hidden internal_hidden) .nil))) .nil
  have hty : ∀ a b : Int, LEnvTy prog (ticks [a, b]) Γdrive :=
    fun _ _ => .cons (.list (.cons .int (.cons .int .nil))) .nil
  obtain ⟨hp, ho, _⟩ := t6_host_ground (Γ := Γdrive) hwf rfl hh Host.unit_lawful
    (fun q hq => by simp [Γdrive] at hq; subst hq; simp [LTy.InPlace, intl, LExpr.ofLabel, LExpr.bot])
    rfl (WorldRel.refl _) (hty 16 32) (hty 17 40) hrel ht n h₁ h₂
  exact ⟨hp, ho⟩

/-- The driver's runs end with the state the steps left, `48` and `57`, and print `on_stop`'s
line once. -/
theorem drive_values :
    (eval Host.unit 10 prog w₀ (ticks [16, 32]) (driveExpr (.int intl) (.int intl))).2.1 = .ok (.int 48) ∧
    (eval Host.unit 10 prog w₀ (ticks [17, 40]) (driveExpr (.int intl) (.int intl))).2.1 = .ok (.int 57) ∧
    (eval Host.unit 10 prog w₀ (ticks [16, 32]) (driveExpr (.int intl) (.int intl))).1 =
      [.out "stopped"] := ⟨rfl, rfl, rfl⟩

/-! ### Exits decided by `internal` data -/

def why : Check.Result Eff → String
  | .ok _ => "ok"
  | .error m => m.msg

def isRestriction : Check.Result Eff → Bool
  | .error (.restriction _) => true
  | _ => false

/-- `for ever: if b: stop; ...` with `b` an `internal` `Bool`: whether later iterations run
depends on `b`. -/
def stopSecret : Expr :=
  .loopEver 11 [("n", .int intl)] [.int 0]
    (.ite (.var "b") (.exit true [.var "n"])
      (.letR "n2" (.int intl) (.prim .add [.var "n", .int 1])
        (.letR "t" (.tuple [.int intl] .bot) (.tuple [.var "n2"]) (.atom (.var "t")))))
    [("m", .int intl)]
    (.letR "t" (.tuple [.int intl] .bot) (.tuple [.var "m"]) (.atom (.var "t")))

/-- The same loop with `skip` for `stop`: IF3 allows it, raising the rest of the body, which
F1's T-Exit does not model. -/
def skipSecret : Expr :=
  match stopSecret with
  | .loopEver s st i (.ite c (.exit _ vs) e) os o => .loopEver s st i (.ite c (.exit false vs) e) os o
  | e => e

def Γb : LCtx := [("b", .bool intl)]

unseal flowB flowsB in
#guard why (checkExpr prog [] Γb .bot (.tuple [.int intl] .bot) stopSecret) ==
  "a `stop` decided above what decides whether its iteration runs (U4002)"

unseal flowB flowsB in
#guard isRestriction (checkExpr prog [] Γb .bot (.tuple [.int intl] .bot) skipSecret)

end Una.F1.LongRunExample
