"""Writes formal/UnaFormal/F1/CheckedImportTests.lean (#1676) from `una --json core --with-rejected`.

The package is `formal/gen/checked_import_tests`, an edition-0.3 package with the module `m`.
`una check` accepts it: `kept` is a `checked` of a `List[Pos]` whose predicate is on the whole
list (ADR 0103 rules 3 and 4), `scalar` a `checked` of an `Int`. The export is complete and
lists nothing under `rejected`; nothing of it changes.

Usage: una --json core --with-rejected formal/gen/checked_import_tests > export.json
       python3 formal/gen/checked_import_tests.py export.json formal/UnaFormal/F1/CheckedImportTests.lean
"""
import json, sys

def main(src, out):
    doc = json.load(open(src, encoding="utf-8"))
    assert doc["complete"] is True and not doc.get("rejected"), "the export is not the accepted package"
    (mod,) = [m for m in doc["modules"] if m["path"] == "m"]
    names = sorted(d["value"]["name"] for d in mod["decls"] if d["kind"] == "fun")
    assert names == ["kept", "scalar"], names
    text = json.dumps(doc, separators=(",", ":"), ensure_ascii=False)
    assert '"#' not in text
    with open(out, "w", encoding="utf-8") as f:
        f.write(f'''/-
Tests of the reading of `checked` of a value that is not a scalar (#1676), checked when the
library builds. The document is written by `formal/gen/checked_import_tests.py` from `una`'s own
export of `formal/gen/checked_import_tests`, which `una check` accepts.
-/
import UnaFormal.F1.ImportTests

namespace Una.F1.CheckedImportTests

open Import ImportTests

/-- `kept` binds `checked(ps, it.len > 0) else fail NoItems` of a `List[Pos]`; `scalar` gives
`checked(n, it > 0) else fail NoItems` of an `Int`. -/
def checks : String := r#"{text}"#

-- `checked` of a list (ADR 0103 rules 3 and 4): a form F1 does not model, so not re-checked, never
-- rejected; before #1676 it was rejected.
#guard run checks "m.kept" == "not re-checked"
-- `checked` of a scalar: re-checked, accepted, as before.
#guard run checks "m.scalar" == "accepted"

end Una.F1.CheckedImportTests
''')

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
