/-
Tests of the marker names (#676), checked when the library builds. Only a function's marker
parameters, those of the operations it invokes, are named with `@`: a `let`, a case's binder, a
lambda's parameter or another parameter so named fails `Fun.markersOk`, and with it the
whole-program check, so no such program has a certificate. A document that names a binder so is
not read (`Import.namesMarker`).
-/
import UnaFormal.F1.CheckProgramExample
import UnaFormal.F1.AtImportTests

namespace Una.F1.MarkerNamesTests

open Import ImportTests Check

/-- `f(x: Int) -> Int = x`. -/
def f : Fun where
  name := "f"
  tparams := []
  eqParams := []
  params := [("x", .int .bot)]
  result := .int .bot
  op := false
  host := HostFx.none
  fails := false
  body := .atom (.var "x")

/-- `f` with the body `body`. -/
def fb (body : Expr) : Fun := { f with body }

/-- `x`, the body of `f`. -/
def x : Expr := .atom (.var "x")

-- A plain function, and one whose marker parameter is that of an operation it invokes.
#guard f.markersOk
#guard ({ f with params := f.params ++ [("@o", .text .bot)], invokes := ["o"] } : Fun).markersOk

-- A parameter named with `@` that is no invoked operation's marker.
#guard !({ f with params := [("@x", .int .bot)] } : Fun).markersOk
#guard !({ f with params := f.params ++ [("@o", .text .bot)], invokes := ["p"] } : Fun).markersOk

-- A `let`, a case's binder, an error's binder, a tuple's or a loop's binder, a lambda's
-- parameter.
#guard !(fb (.letR "@y" (.int .bot) (.atom (.var "x")) x)).markersOk
#guard !(fb (.letE "@y" (.int .bot) x x)).markersOk
#guard !(fb (.caseOption (.var "x") x "@y" x)).markersOk
#guard !(fb (.caseResult (.var "x") "y" x "@z" x)).markersOk
#guard !(fb (.caseSum (.var "x") 0 ["@y"] x x)).markersOk
#guard !(fb (.handle x "@e" x)).markersOk
#guard !(fb (.handleK "K" x "@e" x)).markersOk
#guard !(fb (.letTuple [("@y", .int .bot)] (.var "x") x)).markersOk
#guard !(fb (.loop (.list (.var "x")) "@y" (.int .bot) [] [] x)).markersOk
#guard !(fb (.loop (.list (.var "x")) "y" (.int .bot) [("@s", .int .bot)] [.var "x"] x)).markersOk
#guard !(fb (.letR "g" (.int .bot) (.lambda [("@y", .int .bot)] x) x)).markersOk
-- Nested: in a lambda's body, in the body of a `withH` scope, in an `attempt`.
#guard !(fb (.letR "g" (.int .bot) (.lambda [("y", .int .bot)] (.letR "@z" (.int .bot)
  (.atom (.var "y")) x)) x)).markersOk
#guard !(fb (.withH "o" "h" (.letR "@y" (.int .bot) (.atom (.var "x")) x))).markersOk
#guard !(fb (.attempt (.letR "@y" (.int .bot) (.atom (.var "x")) x))).markersOk
-- The same shapes with plain names pass, and `withH` itself, which binds the marker, is no
-- binder this check refuses.
#guard (fb (.letR "y" (.int .bot) (.lambda [("z", .int .bot)] x) x)).markersOk
#guard (fb (.caseResult (.var "x") "y" x "z" x)).markersOk
#guard (fb (.withH "o" "h" x)).markersOk

/-- `caller` of `CheckProgramExample`, with its `let` named `@b`. -/
def markedCaller : Fun :=
  { CheckProgramExample.caller with
    body := .letR "@b" (.bool .bot) (.call "rev" [] [.var "s"]) (.atom (.var "@b")) }

-- The whole-program check refuses a program with such a binder, which it accepts without it.
#guard progOk [] CheckProgramExample.prog3
#guard !progOk [] { CheckProgramExample.prog3 with
  funs := [ReleaseExample.check, ReleaseExample.rev, markedCaller] }

/-- The reason of the verdict of `decl` in `doc`, when it is not re-checked. -/
def reason (doc : String) (decl : String) : String :=
  match Lean.Json.parse doc with
  | .ok j =>
    match checkDocumentWith (fun rd σ t => t.lsubst rd σ) j {} with
    | .ok out =>
      match List.lookup decl out with
      | some (.notRechecked r) => r
      | some o => verdict o
      | none => "missing"
    | .error e => s!"error: {e}"
  | .error e => s!"error: {e}"

/-- The document of `AtImportTests`, with its `let` named `o` named `@o`. -/
def marked : String :=
  AtImportTests.ats.replace "\"kind\":\"let\",\"name\":\"o\"" "\"kind\":\"let\",\"name\":\"@o\""

-- The document is read as written, and not read once a binder is named with `@`.
#guard marked != AtImportTests.ats
#guard run AtImportTests.ats "m.at_presence" == "accepted"
#guard reason marked "m.at_presence" == "a name starting with `@`"

end Una.F1.MarkerNamesTests
