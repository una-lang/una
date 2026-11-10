"""Writes formal/UnaFormal/F1/UsesOfImportTests.lean (#1676) from `una --json core --with-rejected`.

The package is `formal/gen/uses_of_import_tests`, an edition-0.2 package with the module `m`,
which `una check` accepts. `fold_with` gets `uses of step` inferred and `apply_int` `uses of f`
(ADR 0062 rule 3); `total`, `through` and `named` call them with pure functions and declare no
`uses`; `logged` declares `uses @log`. The export is complete and lists nothing under `rejected`;
nothing of it changes.

Usage: una --json core --with-rejected formal/gen/uses_of_import_tests > export.json
       python3 formal/gen/uses_of_import_tests.py export.json formal/UnaFormal/F1/UsesOfImportTests.lean
"""
import json, sys

def main(src, out):
    doc = json.load(open(src, encoding="utf-8"))
    assert doc["complete"] is True and not doc.get("rejected"), "the export is not the accepted package"
    (mod,) = [m for m in doc["modules"] if m["path"] == "m"]
    funs = {d["value"]["name"]: d["value"] for d in mod["decls"] if d["kind"] == "fun"}
    assert sorted(funs) == ["add", "apply_int", "fold_with", "logged", "make", "named", "through", "total"], sorted(funs)
    assert funs["fold_with"]["uses_of"] == ["step"] and funs["apply_int"]["uses_of"] == ["f"]
    assert all(funs[f]["uses_of"] == [] and funs[f]["uses"] == [] for f in ["total", "through", "named"])
    text = json.dumps(doc, separators=(",", ":"), ensure_ascii=False)
    assert '"#' not in text
    with open(out, "w", encoding="utf-8") as f:
        f.write(f'''/-
Tests of the reading of a call of a function with `uses of` (ADR 0062, #1676), checked when the
library builds. The document is written by `formal/gen/uses_of_import_tests.py` from `una`'s own
export of `formal/gen/uses_of_import_tests`, which `una check` accepts.
-/
import UnaFormal.F1.ImportTests

namespace Una.F1.UsesOfImportTests

open Import ImportTests

/-- `fold_with` has `uses of step` and `apply_int` `uses of f`, both inferred. `total` gives
`fold_with` a lambda with no `uses`, `through` gives `apply_int` the function `make` returns,
`named` gives `fold_with` the function `add` (which Core binds as a lambda), and `logged`
declares `uses @log: Log[write]`. -/
def usesOf : String := r#"{text}"#

-- The functions with `uses of` themselves: re-checked, accepted.
#guard run usesOf "m.fold_with" == "accepted"
#guard run usesOf "m.apply_int" == "accepted"
-- A caller that declares no operation and gives pure functions: F1 has no effect polymorphism, so
-- it is not re-checked, never rejected; before #1676 each was rejected.
#guard run usesOf "m.total" == "not re-checked"
#guard run usesOf "m.through" == "not re-checked"
#guard run usesOf "m.named" == "not re-checked"
-- A caller that declares an operation: checked as before, accepted.
#guard run usesOf "m.logged" == "accepted"

/-- A function type with `uses`, and one without. -/
def effFn : Lean.Json := Lean.Json.mkObj [("kind", "fn"), ("uses", Lean.Json.arr #[Lean.Json.mkObj []])]
def pureFn : Lean.Json := Lean.Json.mkObj [("kind", "fn"), ("uses", Lean.Json.arr #[])]
def varJ (x : String) : Lean.Json := Lean.Json.mkObj [("kind", "var"), ("value", x)]

-- An argument is pure only when every binding of its name is a function type with no `uses`: an
-- effectful binding, a name bound twice with one effectful, an unbound name and a literal are not,
-- so a call given one keeps the declaration checked, and its rejection.
#guard pureFnArg [("g", pureFn)] (varJ "g")
#guard !pureFnArg [("g", effFn)] (varJ "g")
#guard !pureFnArg [("g", pureFn), ("g", effFn)] (varJ "g")
#guard !pureFnArg [] (varJ "g")
#guard !pureFnArg [("g", pureFn)] (Lean.Json.mkObj [("kind", "int"), ("value", "0")])

end Una.F1.UsesOfImportTests
''')

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
