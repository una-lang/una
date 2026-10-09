/-
`una-core-check`: the checker of M2 over `una.core/1` documents (`docs/plan/formal-track.md`
§5). For each document given, one line per declaration with code:

    <document> TAB <declaration> TAB accepted | rejected | not re-checked TAB <reason>

`rejected` on a document `una check` accepted is a disagreement for triage: a potential
soundness finding of the Rust checker, or a reading of the model to revisit. The exit code is 0
when no declaration is rejected and every document is read, 1 otherwise.
-/
import UnaFormal.F1.Import

open Una.F1.Import

def main (args : List String) : IO UInt32 := do
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
          | .accepted => IO.println s!"{path}\t{decl}\taccepted\t"
          | .rejected why =>
            IO.println s!"{path}\t{decl}\trejected\t{why}"
            code := 1
          | .notRechecked why => IO.println s!"{path}\t{decl}\tnot re-checked\t{why}"
  return code
