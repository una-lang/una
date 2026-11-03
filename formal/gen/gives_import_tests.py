"""Writes formal/UnaFormal/F1/GivesImportTests.lean (#1518) from `una --json core`.

The package is `formal/gen/gives_import_tests/`, edition 0.3, with the module `m`: each operation
a binding labels with `gives` (ADR 0094 rule 1), declared, inferred or in a handler's `uses`,
beside its unlabeled twin. The documents are the export itself, `gives`, and the export with
every `gives` key of a `uses` entry removed, `stripped`: the document an export before #1518
wrote for the same package, which shows what the key changes.

Usage: una --json core formal/gen/gives_import_tests > export.json
       python3 formal/gen/gives_import_tests.py export.json formal/UnaFormal/F1/GivesImportTests.lean
"""
import json, sys


def strip(node):
    """Removes `gives` from every `uses` entry, wherever it is; returns how many it removed."""
    count = 0
    if isinstance(node, dict):
        if "cap" in node and "capability" in node and "ops" in node and "gives" in node:
            del node["gives"]
            count += 1
        for value in node.values():
            count += strip(value)
    elif isinstance(node, list):
        for value in node:
            count += strip(value)
    return count


def text(doc):
    t = json.dumps(doc, separators=(",", ":"), ensure_ascii=False)
    assert '"#' not in t
    return t


def main(src, out):
    doc = json.load(open(src, encoding="utf-8"))
    assert doc["complete"] is True
    paths = sorted(m["path"] for m in doc["modules"] if m["path"] != "prelude")
    assert paths == ["m"], paths
    gives = text(doc)
    removed = strip(doc)
    # noted, note_of, noted_through, env_seen, read_seen and the handler `forward`.
    assert removed == 6, removed
    stripped = text(doc)
    with open(out, "w", encoding="utf-8") as f:
        f.write(f'''/-
Tests of the reading of operation labels (ADR 0094 rule 8, #1518), checked when the library
builds. The documents are written by `formal/gen/gives_import_tests.py` from `una`'s own export of
`formal/gen/gives_import_tests/`. F1 does not model a label on an operation yet (#563): it would
read the answer and the decision on the failure of `op gives l` at the capability's declared
labels, below `l`, which the host premise H5 contradicts. So a declaration that uses a labeled
operation, or reaches one, is not re-checked, never accepted; its unlabeled twin keeps its verdict.
-/
import UnaFormal.F1.ImportTests

namespace Una.F1.GivesImportTests

open Import ImportTests

/-- The reason of `decl`'s verdict in `doc`. -/
def why (doc : String) (decl : String) : String :=
  match Lean.Json.parse doc with
  | .ok j =>
    match checkDocumentWith (fun rd σ t => t.lsubst rd σ) j {{}} with
    | .ok out => match List.lookup decl out with
      | some (.notRechecked w) => w
      | some o => verdict o
      | none => "missing"
    | .error e => s!"error: {{e}}"
  | .error e => s!"error: {{e}}"

/-- `una`'s export of the package, with its `gives` keys. -/
def gives : String := r#"{gives}"#

/-- The same export with every `gives` key removed, as an export before #1518 wrote it. -/
def stripped : String := r#"{stripped}"#

def labeled : String := "F1 restriction (a labeled operation, ADR 0094 rule 8)"

-- A user operation the header labels: not re-checked; its unlabeled twin is accepted.
#guard why gives "m.noted" == labeled
#guard run gives "m.noted_plain" == "accepted"
-- A private helper whose label is inferred (ADR 0094 S2), and its labeled caller.
#guard why gives "m.note_of" == labeled
#guard why gives "m.noted_through" == labeled
-- An unlabeled binding passed to a callee that reads it labeled: the caller is not re-checked
-- either, since what it calls is not.
#guard run gives "m.passes_on" == "not re-checked"
-- Host operations the binding labels; their unlabeled twins keep their verdict.
#guard why gives "m.env_seen" == labeled
#guard run gives "m.env_plain" == "accepted"
#guard why gives "m.read_seen" == labeled
#guard run gives "m.read_plain" == "accepted"
-- A handler whose own `uses` label an operation: its synthesized operation is not re-checked,
-- and neither is what invokes it.
#guard why gives "m.forward.write" == labeled
#guard run gives "m.logs" == "not re-checked"

-- Without the key, the re-check could not tell a labeled operation apart and accepted these
-- declarations under H1 to H3 alone: the overclaim #1518 removes.
#guard run stripped "m.noted" == "accepted"
#guard run stripped "m.note_of" == "accepted"
#guard run stripped "m.noted_through" == "accepted"
#guard run stripped "m.passes_on" == "accepted"
#guard run stripped "m.env_seen" == "accepted"
#guard run stripped "m.read_seen" == "accepted"
#guard run stripped "m.forward.write" == "accepted"
#guard run stripped "m.logs" == "accepted"

-- A `gives` that is not a list fails closed.
#guard why (gives.replace "\\"gives\\":[" "\\"gives\\":\\"bad\\",\\"was\\":[") "m.noted" ==
  "F1 restriction (a malformed `gives`, ADR 0094 rule 8)"

end Una.F1.GivesImportTests
''')


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
