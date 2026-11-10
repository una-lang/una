"""Writes formal/UnaFormal/F1/UsesOfImportTests.lean (#1676) from `una --json core --with-rejected`.

The package is `formal/gen/uses_of_import_tests`, an edition-0.2 package with the module `m`,
which `una check` accepts. `fold_with` gets `uses of step` inferred and `apply_int` `uses of f`
(ADR 0062 rule 3); `total`, `through` and `named` call them with pure functions and declare no
`uses`; `logged` declares `uses @log`. `run_it` has `uses of act` and `fails of act`;
`failing_arg` gives it `risky`, which fails, and declares `fails Oops`; `safe_arg` gives it a
lambda that cannot fail; `other_op` calls `noisy`, which uses `@log`, beside a pure call of
`fold_with`, and declares `uses @log`. The export is complete and lists nothing under `rejected`.

Two declarations are added, as a `una check` with a defect would have exported them (#1676's
review): `failing_undeclared`, `failing_arg` without its `fails`, and `other_undeclared`,
`other_op` without its `uses`. `una check` rejects both forms (U3007, U3003), so its own export
cannot hold them; the re-check must reject them, never refuse them as a form F1 does not model.
Nothing else of the export changes.

Usage: una --json core --with-rejected formal/gen/uses_of_import_tests > export.json
       python3 formal/gen/uses_of_import_tests.py export.json formal/UnaFormal/F1/UsesOfImportTests.lean
"""
import copy, json, sys

def main(src, out):
    doc = json.load(open(src, encoding="utf-8"))
    assert doc["complete"] is True and not doc.get("rejected"), "the export is not the accepted package"
    (mod,) = [m for m in doc["modules"] if m["path"] == "m"]
    funs = {d["value"]["name"]: d["value"] for d in mod["decls"] if d["kind"] == "fun"}
    assert sorted(funs) == ["add", "apply_int", "failing_arg", "fold_with", "logged", "make", "named",
        "noisy", "other_op", "risky", "run_it", "safe_arg", "through", "total"], sorted(funs)
    assert funs["fold_with"]["uses_of"] == ["step"] and funs["apply_int"]["uses_of"] == ["f"]
    assert funs["run_it"]["uses_of"] == ["act"] and funs["run_it"]["fails_of"] == "act"
    assert all(funs[f]["uses_of"] == [] and funs[f]["uses"] == [] for f in ["total", "through", "named", "safe_arg"])
    assert funs["failing_arg"]["fails"] == "Oops" and funs["other_op"]["uses"] != []
    by_name = {d["value"]["name"]: d for d in mod["decls"] if d["kind"] == "fun"}
    failing = copy.deepcopy(by_name["failing_arg"])
    failing["value"].update(name="failing_undeclared", fails=None, error=None)
    other = copy.deepcopy(by_name["other_op"])
    other["value"].update(name="other_undeclared", uses=[])
    mod["decls"] += [failing, other]
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
declares `uses @log: Log[write]`. `run_it` has `uses of act` and `fails of act`; `failing_arg`,
`safe_arg` and `other_op` are described in the generator, as are `failing_undeclared` and
`other_undeclared`, which no correct `una check` exports. -/
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
-- `fails of` too: a lambda with no `uses` and no `fails`, given to `run_it`, is read per call,
-- not re-checked. `risky`, which fails and declares no `uses`, is read as performing nothing and
-- still failing, so `failing_arg`, which declares `fails Oops`, is not re-checked either.
#guard run usesOf "m.safe_arg" == "not re-checked"
#guard run usesOf "m.failing_arg" == "not re-checked"
-- A caller that declares the other callee's operation: checked as before, accepted.
#guard run usesOf "m.other_op" == "accepted"
-- The two bypasses of #1676's review stay rejected. A failing argument with no `fails` declared:
-- the call is still read as failing, so the declaration may fail undeclared.
#guard run usesOf "m.failing_undeclared" == "rejected"
-- An operation of another callee beside a pure call: reading the pure call as performing nothing
-- leaves `noisy`'s operation, so the rejection stands.
#guard run usesOf "m.other_undeclared" == "rejected"

/-- A function type with `uses`, one with neither `uses` nor `fails`, and one with `fails`. -/
def effFn : Lean.Json := Lean.Json.mkObj [("kind", "fn"), ("uses", Lean.Json.arr #[Lean.Json.mkObj []])]
def pureFn : Lean.Json := Lean.Json.mkObj [("kind", "fn"), ("uses", Lean.Json.arr #[])]
def failFn : Lean.Json :=
  Lean.Json.mkObj [("kind", "fn"), ("uses", Lean.Json.arr #[]), ("fails", "Oops")]
def varJ (x : String) : Lean.Json := Lean.Json.mkObj [("kind", "var"), ("value", x)]

-- An argument has no `uses` (no `fails`) only when every binding of its name is a function type
-- with none: an effectful (failing) binding, a name bound twice with one effectful, an unbound name
-- and a literal are not, so a call given one keeps that part of its reading, and its rejection.
#guard fnArgWith noUses [("g", pureFn)] (varJ "g")
#guard !fnArgWith noUses [("g", effFn)] (varJ "g")
#guard !fnArgWith noUses [("g", pureFn), ("g", effFn)] (varJ "g")
#guard !fnArgWith noUses [] (varJ "g")
#guard !fnArgWith noUses [("g", pureFn)] (Lean.Json.mkObj [("kind", "int"), ("value", "0")])
#guard fnArgWith noFails [("g", pureFn)] (varJ "g")
#guard !fnArgWith noFails [("g", failFn)] (varJ "g")
#guard fnArgWith noUses [("g", failFn)] (varJ "g")

end Una.F1.UsesOfImportTests
''')

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
