"""Writes formal/UnaFormal/F1/FixedImportTests.lean (#548) from `una --json core --with-rejected`.

The package is `formal/gen/fixed_import_tests.una` as the module `m` of an edition-0.2 package.
`una check` rejects the `_secret` functions (U4001), so the export lists them under `rejected`;
the importer reads one complete document, so they are put back into the module, in source order,
and the document is marked complete. Nothing else of the export changes.

Usage: una --json core --with-rejected <pkg> > export.json
       python3 formal/gen/fixed_import_tests.py export.json formal/UnaFormal/F1/FixedImportTests.lean
"""
import json, sys

ORDER = ["add_public", "add_secret", "nested", "narrow_public", "narrow_secret", "widen",
         "div_public", "wrap", "unknown_binder"]

VERDICT = {"add_secret": "rejected", "narrow_secret": "rejected", "wrap": "not re-checked",
           "unknown_binder": "not re-checked"}
EXPECT = [(n, VERDICT.get(n, "accepted")) for n in ORDER]

def name(d):
    return d["value"]["name"] if d["kind"] == "fun" else d["name"]

def main(src, out):
    doc = json.load(open(src, encoding="utf-8"))
    (mod,) = [m for m in doc["modules"] if m["path"] == "m"]
    decls = mod["decls"] + [r["decl"] for r in doc.pop("rejected", []) if r["module"] == mod["path"]]
    assert sorted(map(name, decls)) == sorted(ORDER), sorted(map(name, decls))
    mod["decls"] = sorted(decls, key=lambda d: ORDER.index(name(d)))
    doc["complete"] = True
    text = json.dumps(doc, separators=(",", ":"), ensure_ascii=False)
    assert '"#' not in text
    guards = "\n".join(f'#guard run fixeds "m.{n}" == "{v}"' for n, v in EXPECT)
    with open(out, "w", encoding="utf-8") as f:
        f.write(f'''/-
Tests of the reading of fixed-width integers (#548), checked when the library builds. The
document is written by `formal/gen/fixed_import_tests.py` from `una`'s own export of
`formal/gen/fixed_import_tests.una`; each verdict matches what `una check` says of the function:
arithmetic and conversions on public values are accepted, on a secret rejected (U4001). A
wrapping method, and arithmetic on a pattern's binder, whose type the reading does not know, are
not re-checked, never read as unbounded `Int`.
-/
import UnaFormal.F1.ImportTests

namespace Una.F1.FixedImportTests

open Import ImportTests

def fixeds : String := r#"{text}"#

{guards}

end Una.F1.FixedImportTests
''')

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
