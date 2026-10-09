/-
Tests of the instance search of the re-check (`Import.lean`), checked when the library builds:
a function whose instances the search does not finish, or that a node other than a call
instantiates, is never reported accepted (ADR 0008, "unknown is not proven").
-/
import UnaFormal.F1.Import

namespace Una.F1.ImportTests

open Import

def verdict : Outcome → String
  | .accepted _ => "accepted"
  | .rejected _ => "rejected"
  | .notRechecked _ => "not re-checked"

/-- The verdict of `decl` in `doc` under `caps`. -/
def run (doc : String) (decl : String) (caps : Caps := {}) : String :=
  match Lean.Json.parse doc with
  | .ok j =>
    match checkDocumentWith (fun rd σ t => t.lsubst rd σ) j caps with
    | .ok out => ((List.lookup decl out).map verdict).getD "missing"
    | .error e => s!"error: {e}"
  | .error e => s!"error: {e}"

/-- `h[U](x: U)`; `g[T](x: T)` calls `h[T]`, so `g` is checked at its instances; `main` calls
`g[Int]` and `g[Bool]`: two instances of `g`. -/
def fanout : String := r#"{"schema": "una.core/1", "edition": "0.2", "complete": true, "modules": [{"path": "m", "decls": [{"kind": "fun", "value": {"name": "h", "type_params": ["U"], "constraints": [{"kind": "none"}], "params": [["x", {"kind": "base", "base": {"kind": "param", "value": "U"}, "label": {"level": "public", "categories": [], "symbols": []}}]], "result": {"kind": "base", "base": {"kind": "unit"}, "label": {"level": "public", "categories": [], "symbols": []}}, "uses": [], "fails": null, "error": null, "body": {"kind": "atom", "atom": {"kind": "unit"}}}}, {"kind": "fun", "value": {"name": "g", "type_params": ["T"], "constraints": [{"kind": "none"}], "params": [["x", {"kind": "base", "base": {"kind": "param", "value": "T"}, "label": {"level": "public", "categories": [], "symbols": []}}]], "result": {"kind": "base", "base": {"kind": "unit"}, "label": {"level": "public", "categories": [], "symbols": []}}, "uses": [], "fails": null, "error": null, "body": {"kind": "let", "name": "r", "ty": {"kind": "base", "base": {"kind": "unit"}, "label": {"level": "public", "categories": [], "symbols": []}}, "value": {"kind": "rhs", "value": {"kind": "call", "function": {"module": "m", "name": "h"}, "type_args": [{"kind": "base", "base": {"kind": "param", "value": "T"}, "label": {"level": "public", "categories": [], "symbols": []}}], "args": [{"kind": "var", "value": "x"}]}}, "body": {"kind": "atom", "atom": {"kind": "var", "value": "r"}}}}}, {"kind": "fun", "value": {"name": "main", "type_params": [], "constraints": [], "params": [], "result": {"kind": "base", "base": {"kind": "unit"}, "label": {"level": "public", "categories": [], "symbols": []}}, "uses": [], "fails": null, "error": null, "body": {"kind": "let", "name": "a", "ty": {"kind": "base", "base": {"kind": "unit"}, "label": {"level": "public", "categories": [], "symbols": []}}, "value": {"kind": "rhs", "value": {"kind": "call", "function": {"module": "m", "name": "g"}, "type_args": [{"kind": "base", "base": {"kind": "int"}, "label": {"level": "public", "categories": [], "symbols": []}}], "args": [{"kind": "int", "value": "1"}]}}, "body": {"kind": "let", "name": "b", "ty": {"kind": "base", "base": {"kind": "unit"}, "label": {"level": "public", "categories": [], "symbols": []}}, "value": {"kind": "rhs", "value": {"kind": "call", "function": {"module": "m", "name": "g"}, "type_args": [{"kind": "base", "base": {"kind": "bool"}, "label": {"level": "public", "categories": [], "symbols": []}}], "args": [{"kind": "bool", "value": true}]}}, "body": {"kind": "atom", "atom": {"kind": "var", "value": "b"}}}}}}]}], "library": [], "host": []}"#

/-- `fanout` with `main`'s call of `g[Bool]` written as a `spawn` node, a form F1 does not read
that carries type arguments. -/
def otherNode : String := r#"{"schema": "una.core/1", "edition": "0.2", "complete": true, "modules": [{"path": "m", "decls": [{"kind": "fun", "value": {"name": "h", "type_params": ["U"], "constraints": [{"kind": "none"}], "params": [["x", {"kind": "base", "base": {"kind": "param", "value": "U"}, "label": {"level": "public", "categories": [], "symbols": []}}]], "result": {"kind": "base", "base": {"kind": "unit"}, "label": {"level": "public", "categories": [], "symbols": []}}, "uses": [], "fails": null, "error": null, "body": {"kind": "atom", "atom": {"kind": "unit"}}}}, {"kind": "fun", "value": {"name": "g", "type_params": ["T"], "constraints": [{"kind": "none"}], "params": [["x", {"kind": "base", "base": {"kind": "param", "value": "T"}, "label": {"level": "public", "categories": [], "symbols": []}}]], "result": {"kind": "base", "base": {"kind": "unit"}, "label": {"level": "public", "categories": [], "symbols": []}}, "uses": [], "fails": null, "error": null, "body": {"kind": "let", "name": "r", "ty": {"kind": "base", "base": {"kind": "unit"}, "label": {"level": "public", "categories": [], "symbols": []}}, "value": {"kind": "rhs", "value": {"kind": "call", "function": {"module": "m", "name": "h"}, "type_args": [{"kind": "base", "base": {"kind": "param", "value": "T"}, "label": {"level": "public", "categories": [], "symbols": []}}], "args": [{"kind": "var", "value": "x"}]}}, "body": {"kind": "atom", "atom": {"kind": "var", "value": "r"}}}}}, {"kind": "fun", "value": {"name": "main", "type_params": [], "constraints": [], "params": [], "result": {"kind": "base", "base": {"kind": "unit"}, "label": {"level": "public", "categories": [], "symbols": []}}, "uses": [], "fails": null, "error": null, "body": {"kind": "let", "name": "a", "ty": {"kind": "base", "base": {"kind": "unit"}, "label": {"level": "public", "categories": [], "symbols": []}}, "value": {"kind": "rhs", "value": {"kind": "call", "function": {"module": "m", "name": "g"}, "type_args": [{"kind": "base", "base": {"kind": "int"}, "label": {"level": "public", "categories": [], "symbols": []}}], "args": [{"kind": "int", "value": "1"}]}}, "body": {"kind": "let", "name": "b", "ty": {"kind": "base", "base": {"kind": "unit"}, "label": {"level": "public", "categories": [], "symbols": []}}, "value": {"kind": "rhs", "value": {"kind": "spawn", "function": {"module": "m", "name": "g"}, "type_args": [{"kind": "base", "base": {"kind": "bool"}, "label": {"level": "public", "categories": [], "symbols": []}}], "args": [{"kind": "bool", "value": true}]}}, "body": {"kind": "atom", "atom": {"kind": "var", "value": "b"}}}}}}]}], "library": [], "host": []}"#

-- With room, both instances are checked and pass.
#guard run fanout "m.g" == "accepted"
-- A search out of fuel with an instance left in its queue: unknown, not accepted.
#guard run fanout "m.g" { fuel := 1 } == "not re-checked"
-- More instances than the count cap: unknown.
#guard run fanout "m.g" { count := 1 } == "not re-checked"
-- A non-call node with type arguments makes the instances of what it names unknown.
#guard run otherNode "m.g" == "not re-checked"
-- The function that is checked generically is not affected.
#guard run otherNode "m.h" == "accepted"

end Una.F1.ImportTests
