"""Writes formal/UnaFormal/F1/AtImportTests.lean (#545) from `una --json core --with-rejected`.

The package is `formal/gen/at_import_tests.una` as the module `m` of an edition-0.2 package.
`una check` rejects the `_secret` functions (U4001), so the export lists them under `rejected`;
the importer reads one complete document, so they are put back into the module, in source order,
and the document is marked complete. Nothing else of the export changes.

Usage: una --json core --with-rejected <pkg> > export.json
       python3 formal/gen/at_import_tests.py export.json formal/UnaFormal/F1/AtImportTests.lean
"""
import json, sys

ORDER = ["at_public", "at_elem_secret", "at_presence", "at_index_secret"]

EXPECT = [(n, "rejected" if n.endswith("_secret") else "accepted") for n in ORDER]

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
    guards = "\n".join(f'#guard run ats "m.{n}" == "{v}"' for n, v in EXPECT)
    with open(out, "w", encoding="utf-8") as f:
        f.write(f'''/-
Tests of the reading of `at` (#545), checked when
the library builds. The document is written by `formal/gen/at_import_tests.py` from `una`'s
own export of `formal/gen/at_import_tests.una`; each verdict matches what `una check` says
of the function: `at` of a public list at a public index, and a list of secret elements matched
only for `Some` or `None`, are accepted; the element of a list of secret elements, and the
presence of an element at a secret index, are rejected (U4001).
-/
import UnaFormal.F1.ImportTests

namespace Una.F1.AtImportTests

open Import ImportTests

def ats : String := r#"{text}"#

{guards}

end Una.F1.AtImportTests
''')

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
