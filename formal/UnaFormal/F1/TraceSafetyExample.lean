/-
Non-vacuity of trace safety (ADR 0092, #561, stage 1), in the shape of B06 (una-bench
`tasks/B06-dry-run-prices`): an update journals a failed lookup, and a dry run must journal
nothing. `Catalog.journal` is a user operation whose handler `m.log.journal` the program invokes.

- **(a)** The reference: `update`, `report` and `stop` each condition the journal on
  `not dry`, and `stop` guards it. Checked; a dry run journals nothing, a real one journals.
- **(b)** `stop` journals unguarded: rejected, and its dry run journals.
- **(c)** A lambda that journals: not re-checked in stage 1 (#568 reads it in stage 2).
- **(d)** A journal outside the guard, through a handler installed by `with`: rejected.
- **(e)** One atom, two occurrences: a helper's condition `count < limit` at the caller's
  `count < limit` is one variable, so checked; written `limit > count`, it is another, and
  rejected (the coverage limit).
- **The ghost under a secret program counter**: `WithExample.h3` invokes `Vault.note` only in its
  secret branch. The program is certified (T6) and passes trace safety's checks, and both runs
  project to the same `done`.
-/
import UnaFormal.F1.TraceSafetyProof
import UnaFormal.F1.WithExample

namespace Una.F1.TraceSafetyExample

open Una.F1

def mk (name : String) (params : List (String × LTy)) (body : Expr) (conds : List (String × Cond)) : Fun :=
  { name, tparams := [], eqParams := [], params, result := .unit .bot, op := false,
    host := HostFx.none, fails := false, body, conds }

/-- The journal's handler: it prints the line, so that a performance shows in the trace too. -/
def journal : Fun := mk "m.log.journal" [] (.letR "u" (.unit .bot) (.op (.text "journal")) (.atom .unit)) []

def notDry : Cond := .not (.var "dry")

def jcall : Expr := .letR "u" (.unit .bot) (.invoke "m.log.journal" []) (.atom .unit)

/-- `stop(dry)`: journals unless `dry` (guarded), or always. -/
def stop (guard : Bool) : Fun := mk "m.stop" [("dry", .bool .bot)]
  (if guard then .ite (.var "dry") (.atom .unit) jcall else jcall) [("m.log.journal", notDry)]

/-- `report(dry)`: calls `stop`; its header conditions the journal, or not. -/
def report (cond : Bool) : Fun := mk "m.report" [("dry", .bool .bot)]
  (.letR "r" (.unit .bot) (.call "m.stop" [] [.var "dry"]) (.atom .unit))
  (if cond then [("m.log.journal", notDry)] else [])

/-- `update(dry)`: the entry point, with the scaffold's condition. -/
def update : Fun := mk "m.update" [("dry", .bool .bot)]
  (.letR "r" (.unit .bot) (.call "m.report" [] [.var "dry"]) (.ite (.var "dry") (.atom .unit) jcall))
  [("m.log.journal", notDry)]

def prog (guard cond : Bool) : Program :=
  Import.mkProgram [] Label.bot {} [] [] [journal, stop guard, report cond, update] [] []

def env (dry : Bool) : Env := bindParams update.params [.bool dry] []

def w0 : World Host.unit := ⟨(), (), fun _ => none⟩

/-! ### (a) The reference -/

theorem reference_checked : (prog true true).traceSafeB = true ∧ (prog true true).stage1B = true :=
  ⟨rfl, rfl⟩

/-- A dry run performs no journal: `trace_safety` at `dry = true`. -/
theorem reference_dry : ¬ Performs "m.log.journal" (eval Host.unit 20 (prog true true) w0 (env true) update.body).1 :=
  trace_safety (f := update) (C := notDry) rfl Host.unit_respTyped (by simp [prog, Import.mkProgram])
    rfl w0 20 [.bool true] ⟨trivial, trivial⟩ rfl

/-- A real run journals, in `stop` and after the update: the theorem is not vacuous. -/
theorem reference_real : (eval Host.unit 20 (prog true true) w0 (env false) update.body).1 =
    [.perfFn "m.log.journal", .out "journal", .perfFn "m.log.journal", .out "journal"] := rfl

/-! ### (b) The unguarded journal -/

theorem unguarded_rejected : (prog false true).traceSafeB = false := rfl

theorem unguarded_trace : (eval Host.unit 20 (prog false true) w0 (env true) update.body).1 =
    [.perfFn "m.log.journal", .out "journal"] := rfl

theorem unguarded_journals :
    Performs "m.log.journal" (eval Host.unit 20 (prog false true) w0 (env true) update.body).1 :=
  ⟨.perfFn "m.log.journal", by rw [unguarded_trace]; simp, rfl⟩

/-- A helper without the condition makes its caller's call the refuted site. -/
theorem uncond_rejected : (prog true false).traceSafeB = false := rfl

/-! ### (c) A lambda that journals: not re-checked in stage 1 -/

def updateLam : Fun := mk "m.update" [("dry", .bool .bot)]
  (.ite (.var "dry") (.atom .unit) (.letR "k" (.unit .bot) (.lambda [] jcall) (.atom .unit)))
  [("m.log.journal", notDry)]

theorem lambda_not_rechecked :
    (Import.mkProgram [] Label.bot {} [] [] [journal, updateLam] [] []).stage1B = false := rfl

/-! ### (d) A handler installed outside the guard -/

def updateWith : Fun := mk "m.update" [("dry", .bool .bot)]
  (.withH "Catalog.journal" "m.log.journal"
    (.letR "u" (.unit .bot) (.invokeOp "Catalog.journal" []) (.atom .unit)))
  [("Catalog.journal", notDry)]

theorem with_rejected :
    (Import.mkProgram [] Label.bot {} [] [] [journal, updateWith] [] []).traceSafeB = false := rfl

/-! ### (e) One atom, two occurrences -/

def lim : Cond := .ltInt (.var "count") (.var "limit")

def helper : Fun := mk "m.helper" [("count", .int .bot), ("limit", .int .bot)] jcall [("m.log.journal", lim)]

def outer (c : Cond) : Fun := mk "m.outer" [("count", .int .bot), ("limit", .int .bot)]
  (.letR "r" (.unit .bot) (.call "m.helper" [] [.var "count", .var "limit"]) (.atom .unit))
  [("m.log.journal", c)]

/-- The call's implication identifies the two occurrences: `startOk` holds at `outer`'s call. -/
theorem same_atom_call :
    startOk (Import.mkProgram [] Label.bot {} [] [] [journal, helper, outer lim] [] [])
      [("m.log.journal", lim)] [] "m.helper" (some [.var "count", .var "limit"]) = true := by
  decide

/-- The same condition in other syntax is another atom: the call's implication is refuted. -/
theorem other_syntax_call :
    startOk (Import.mkProgram [] Label.bot {} [] [] [journal, helper, outer (.ltInt (.var "limit") (.var "count"))] [] [])
      [("m.log.journal", .ltInt (.var "limit") (.var "count"))] [] "m.helper"
      (some [.var "count", .var "limit"]) = false := by
  decide

/-! ### A ghost under a secret program counter -/

/-- `WithExample.h3`'s program passes trace safety's checks (it conditions nothing), and is
certified (`WithExample.certified`, `h3_checked`); its secret run performs `Vault.note`, a ghost
no observer sees (`WithExample.h3_runs`). -/
theorem h3_trace_safe : WithExample.prog.traceSafeB = true ∧ WithExample.prog.stage1B = true := ⟨rfl, rfl⟩

end Una.F1.TraceSafetyExample
