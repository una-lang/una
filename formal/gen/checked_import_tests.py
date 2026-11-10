"""Writes formal/UnaFormal/F1/CheckedImportTests.lean (#1676) from `una --json core --with-rejected`.

The package is `formal/gen/checked_import_tests`, an edition-0.3 package with the module `m`.
`una check` accepts `kept`, a `checked` of a `List[Pos]` whose predicate is on the whole list
(ADR 0103 rules 3 and 4), and `scalar`, a `checked` of an `Int`. It rejects, with U4002 alone,
`outer` (a `checked` of a `secret List[Int]`), `inner` (of a `List[secret Int]`) and `raised` (of
a public list under a `secret` branch), so the export lists them under `rejected`; the importer
reads one complete document, so they are put back into the module, in source order, and the
document is marked complete. Nothing else of the export changes.

Usage: una --json core --with-rejected formal/gen/checked_import_tests > export.json
       python3 formal/gen/checked_import_tests.py export.json formal/UnaFormal/F1/CheckedImportTests.lean
"""
import json, sys

def main(src, out):
    doc = json.load(open(src, encoding="utf-8"))
    (mod,) = [m for m in doc["modules"] if m["path"] == "m"]
    rejected = doc.pop("rejected", [])
    codes = {r["decl"]["value"]["name"]: [x["code"] for x in r["diagnostics"]] for r in rejected}
    assert codes == {"outer": ["U4002"], "inner": ["U4002"], "raised": ["U4002"]}, codes
    order = ["Pos", "Empty", "kept", "scalar", "outer", "inner", "raised"]
    decls = mod["decls"] + [r["decl"] for r in rejected if r["module"] == mod["path"]]
    names = sorted(d["value"]["name"] for d in decls if d["kind"] == "fun")
    assert names == ["inner", "kept", "outer", "raised", "scalar"], names
    def name(d):
        return d["value"]["name"] if d["kind"] == "fun" else d.get("name", d.get("value", {}).get("name"))
    assert sorted(map(name, decls)) == sorted(order), sorted(map(name, decls))
    mod["decls"] = sorted(decls, key=lambda d: order.index(name(d)))
    doc["complete"] = True
    text = json.dumps(doc, separators=(",", ":"), ensure_ascii=False)
    assert '"#' not in text
    with open(out, "w", encoding="utf-8") as f:
        f.write(f'''/-
Tests of the reading of `checked` of a value that is not a scalar (#1676), checked when the
library builds. The document is written by `formal/gen/checked_import_tests.py` from `una`'s own
export of `formal/gen/checked_import_tests`; each verdict matches what `una check` says of the
function.
-/
import UnaFormal.F1.ImportTests

namespace Una.F1.CheckedImportTests

open Import ImportTests

/-- `kept` binds `checked(ps, it.len > 0) else fail NoItems` of a `List[Pos]`; `scalar` gives
`checked(n, it > 0) else fail NoItems` of an `Int`. `outer` gives a `checked` of a
`secret List[Int]`, `inner` of a `List[secret Int]`, and `raised` of a public list under a `secret`
branch, which `una check` rejects (U4002). -/
def checks : String := r#"{text}"#

-- `checked` of a list (ADR 0103 rules 3 and 4): a form F1 does not model, so not re-checked, never
-- rejected; before #1676 it was rejected.
#guard run checks "m.kept" == "not re-checked"
-- `checked` of a scalar: re-checked, accepted, as before.
#guard run checks "m.scalar" == "accepted"
-- The label and program-counter premises come before the restriction: a `checked` of a labeled
-- collection, or of one under a raised program counter, is rejected, as `una check` rejects it
-- (U4002), never refused as a form F1 does not model.
#guard run checks "m.outer" == "rejected"
#guard run checks "m.inner" == "rejected"
#guard run checks "m.raised" == "rejected"

end Una.F1.CheckedImportTests
''')

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
