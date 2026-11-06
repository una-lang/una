"""Writes formal/UnaFormal/F1/QuietImportTests.lean (#1553) from `una --json core --with-rejected`.

The package is `formal/gen/quiet_import_tests.una` as the module `m`, the file `src/m.una`, of an
edition-0.2 package whose `una.toml` names it `quiet_import_tests`, version `0.1.0`. `una check`
rejects `loud` (U4003), so the export lists it under `rejected`; the importer reads one complete
document, so it is put back into the module, in source order, and the document is marked
complete. Nothing else of the export changes. The export wraps the operations of `feed` and
`loud`, and the call of `fan`, in `quiet` (S-42); `public_put` holds none.

Usage: una --json core --with-rejected <pkg> > export.json
       python3 formal/gen/quiet_import_tests.py export.json formal/UnaFormal/F1/QuietImportTests.lean
"""
import json, sys

ORDER = ["Sink", "Log", "feed", "fan", "loud", "public_put"]

def name(d):
    return d["value"]["name"] if d["kind"] == "fun" else d.get("name", d.get("value", {}).get("name"))

def quiets(j):
    if isinstance(j, dict):
        return (j.get("kind") == "quiet") + sum(quiets(v) for v in j.values())
    if isinstance(j, list):
        return sum(quiets(v) for v in j)
    return 0

def main(src, out):
    doc = json.load(open(src, encoding="utf-8"))
    (mod,) = [m for m in doc["modules"] if m["path"] == "m"]
    decls = mod["decls"] + [r["decl"] for r in doc.pop("rejected", []) if r["module"] == mod["path"]]
    assert sorted(map(name, decls)) == sorted(ORDER), sorted(map(name, decls))
    mod["decls"] = sorted(decls, key=lambda d: ORDER.index(name(d)))
    counts = {name(d): quiets(d) for d in mod["decls"]}
    assert counts == {"Sink": 0, "Log": 0, "feed": 1, "fan": 1, "loud": 1, "public_put": 0}, counts
    doc["complete"] = True
    text = json.dumps(doc, separators=(",", ":"), ensure_ascii=False)
    assert '"#' not in text
    with open(out, "w", encoding="utf-8") as f:
        f.write(f'''/-
Tests of the reading of Core's `quiet e` as `e` (#1553, S-39, S-42), checked when the library
builds. The document is written by `formal/gen/quiet_import_tests.py` from `una`'s own export of
`formal/gen/quiet_import_tests.una`; each verdict matches what `una check` says of the function.
-/
import UnaFormal.F1.ImportTests

namespace Una.F1.QuietImportTests

open Import ImportTests

/-- `Sink.put` accepts `secret`, `Log.write` `public`. `feed` puts a `secret` value under a
`secret` branch, `fan` calls `feed` there, `loud` writes under a `secret` branch, and
`public_put` puts under a public one. The export wraps the operations of `feed` and `loud`, and
the call of `fan`, in `quiet`. -/
def quiets : String := r#"{text}"#

-- An operation that accepts `secret` under a `secret` branch, in `quiet`: re-checked, accepted.
#guard run quiets "m.feed" == "accepted"
-- A call under a `secret` branch whose callee performs one, in `quiet`: accepted.
#guard run quiets "m.fan" == "accepted"
-- `quiet` hides nothing from the re-check: a public sink under a `secret` branch is rejected, as
-- `una check` rejects it (U4003).
#guard run quiets "m.loud" == "rejected"
-- No `quiet`: accepted, as before.
#guard run quiets "m.public_put" == "accepted"

end Una.F1.QuietImportTests
''')

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
