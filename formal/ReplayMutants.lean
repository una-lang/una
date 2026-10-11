/-
`replay-mutants`: runs the re-check of each document given with labeled instantiation and with
its two mutants (`CheckTests.lean`), and prints, per declaration, the outcome under each:

    <document> TAB <declaration> TAB <lsubst> TAB <mutant-label> TAB <mutant-deep>

A replay catches a mutant when the declaration is rejected under `LTy.lsubst` and not under it.
The exit code is 1 when a mutant is caught by no document, 0 otherwise.
-/
import UnaFormal.F1.Import
import UnaFormal.F1.CheckTests

open Una.F1 Una.F1.Import Una.F1.CheckTests

def verdict : Outcome → String
  | .accepted _ => "accepted"
  | .acceptedIfA3 _ => "accepted, conditional on A3"
  | .rejected _ => "rejected"
  | .notRechecked _ => "not re-checked"
  | .outsideFlow _ => "outside the flow model"

def main (args : List String) : IO UInt32 := do
  let mut caughtLabel := false
  let mut caughtDeep := false
  for path in args do
    let text ← IO.FS.readFile path
    let some doc := (Lean.Json.parse text).toOption
      | IO.println s!"{path}\t-\terror"; return 1
    let run := fun (s : Subst) => (checkDocumentWith s doc).toOption.getD []
    let right := run (fun rd σ t => t.lsubst rd σ)
    let label := run mutantLabel
    let deep := run mutantDeep
    for (decl, o) in right do
      let ol := (List.lookup decl label).map verdict |>.getD "-"
      let od := (List.lookup decl deep).map verdict |>.getD "-"
      if verdict o == "rejected" && ol != "rejected" then caughtLabel := true
      if verdict o == "rejected" && od != "rejected" then caughtDeep := true
      IO.println s!"{path}\t{decl}\t{verdict o}\t{ol}\t{od}"
  IO.println s!"mutant-label caught: {caughtLabel}; mutant-deep caught: {caughtDeep}"
  return if caughtLabel && caughtDeep then 0 else 1
