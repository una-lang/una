/-
`una-core-check`: the checker of M2 over `una.core/1` documents (`docs/plan/formal-track.md`
§5). For each document given, one line per declaration with code:

    <document> TAB <declaration> TAB accepted | rejected | not re-checked | outside the flow model TAB <reason>

`rejected` on a document `una check` accepted is a disagreement for triage: a potential
soundness finding of the Rust checker, or a reading of the model to revisit. The exit code is 0
when no declaration is rejected and every document is read, 1 otherwise. `outside the flow model`
is a declaration the flow model leaves out by design, a refinement predicate (`spec/07 §7` rule
6), counted apart from `not re-checked` (#532).

With `--rejected` first (ADR 0091), it reads the declarations of each document's `rejected` key,
which `una --json core --with-rejected` exports, instead, one line each:

    <document> TAB <declaration> TAB model-accepts-candidate | model-rejects | not-re-checked TAB <reason>

`model-accepts-candidate` is a candidate for review, never an accepted declaration. Neither
outcome certifies anything: the context omits the declarations with other diagnostics, so a
candidate can be false and a `model-rejects` can hide a real one. The first line of the output
says so. The exit code is 0 when every document is read, 1 otherwise.
-/
import UnaFormal.F1.Import

open Una.F1.Import

/-- The reverse comparison (ADR 0091): each rejected declaration of each document. -/
def reverse (paths : List String) : IO UInt32 := do
  IO.println "# reverse comparison (ADR 0091): leads for review, not verdicts; neither outcome certifies anything"
  let mut code : UInt32 := 0
  for path in paths do
    let text ← IO.FS.readFile path
    match Lean.Json.parse text >>= checkRejected with
    | .error e =>
      IO.println s!"{path}\t-\terror\t{e}"
      code := 1
    | .ok outcomes =>
      for (decl, o) in outcomes do
        match o with
        | .modelAcceptsCandidate note => IO.println s!"{path}\t{decl}\tmodel-accepts-candidate\t{note}"
        | .modelRejects why => IO.println s!"{path}\t{decl}\tmodel-rejects\t{why}"
        | .notRechecked why => IO.println s!"{path}\t{decl}\tnot-re-checked\t{why}"
  return code

def main (args : List String) : IO UInt32 := do
  if args.head? == some "--rejected" then return ← reverse args.tail
  let mut code : UInt32 := 0
  for path in args do
    let text ← IO.FS.readFile path
    match Lean.Json.parse text with
    | .error e =>
      IO.println s!"{path}\t-\terror\t{e}"
      code := 1
    | .ok doc =>
      match checkDocument doc with
      | .error e =>
        IO.println s!"{path}\t-\terror\t{e}"
        code := 1
      | .ok outcomes =>
        for (decl, o) in outcomes do
          match o with
          | .accepted note => IO.println s!"{path}\t{decl}\taccepted\t{note}"
          | .acceptedIfA3 note =>
            IO.println s!"{path}\t{decl}\taccepted, conditional on A3\t{note}"
          | .rejected why =>
            IO.println s!"{path}\t{decl}\trejected\t{why}"
            code := 1
          | .notRechecked why => IO.println s!"{path}\t{decl}\tnot re-checked\t{why}"
          | .outsideFlow why => IO.println s!"{path}\t{decl}\toutside the flow model\t{why}"
  return code
