"""Writes formal/UnaFormal/F1/HandlerImportTests.lean (#524) from `una --json core --with-rejected`.

The package is `formal/gen/handler_import_tests.una` as the module `m` of an edition-0.2 package.
`una check` rejects `knock_pass` (U4002) and `bell`'s handler (U4003); the export lists rejected
declarations under `rejected`, which are put back into the module, in source order, and the
document is marked complete. Nothing else of the export changes.

Usage: una --json core --with-rejected <pkg> > export.json
       python3 formal/gen/handler_import_tests.py export.json formal/UnaFormal/F1/HandlerImportTests.lean
"""
import json, sys

ORDER = ["Shut", "Door", "door", "Bell", "bell", "Gate", "knock_attempt", "knock_use",
         "knock_pass", "ring_it", "through", "installs"]

def name(d):
    return d["value"]["name"] if d["kind"] == "fun" else d.get("name", d.get("value", {}).get("name"))

def main(src, out):
    doc = json.load(open(src, encoding="utf-8"))
    (mod,) = [m for m in doc["modules"] if m["path"] == "m"]
    decls = mod["decls"] + [r["decl"] for r in doc.pop("rejected", []) if r["module"] == mod["path"]]
    assert sorted(map(name, decls)) == sorted(ORDER), sorted(map(name, decls))
    mod["decls"] = sorted(decls, key=lambda d: ORDER.index(name(d)))
    doc["complete"] = True
    text = json.dumps(doc, separators=(",", ":"), ensure_ascii=False)
    assert '"#' not in text
    with open(out, "w", encoding="utf-8") as f:
        f.write(f'''/-
Tests of the reading of user capabilities' handlers (#524), checked when the library builds. The
document is written by `formal/gen/handler_import_tests.py` from `una`'s own export of
`formal/gen/handler_import_tests.una`; each verdict is what `una check` says of the declaration,
or not re-checked where F1 does not read it.
-/
import UnaFormal.F1.ImportTests

namespace Una.F1.HandlerImportTests

open Import ImportTests

/-- `Door.open` (fails `Shut`, accepts `secret`) with the lawful handler `door`, which fails;
`Bell.ring` (accepts `secret`) with the handler `bell`, which prints below its floor (H1);
`Gate.pass_through`, which can fail, with no handler in the document. -/
def handlers : String := r#"{text}"#

-- H5: a failure handled in the secret branch, by `attempt` or `if error use`: accepted, and the
-- handler's operation, a function of the document, is accepted too.
#guard run handlers "m.knock_attempt" == "accepted"
#guard run handlers "m.knock_use" == "accepted"
#guard run handlers "m.door.open" == "accepted"
-- H5: the failure passed out of the secret branch: rejected, as `una check` rejects it (U4002).
#guard run handlers "m.knock_pass" == "rejected"
-- H1: a handler that branches on its parameter, which carries `secret`, and prints: rejected, and
-- what invokes it reaches a rejected function, so it is not re-checked.
#guard run handlers "m.bell.ring" == "rejected"
#guard run handlers "m.ring_it" == "not re-checked"
-- An operation that can fail, whose handler the document does not declare: not re-checked.
#guard run handlers "m.through" == "not re-checked"
-- A declaration that installs a handler (`with`), read since #538: accepted, as `una check` says;
-- the effect-free handler it installs charges it nothing (ruling E).
#guard run handlers "m.installs" == "accepted"

end Una.F1.HandlerImportTests
''')

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
