"""Writes formal/UnaFormal/F1/PreludeImportTests.lean (#542) from `una --json core --with-rejected`.

The package is `formal/gen/prelude_import_tests.una` as the module `m` of an edition-0.2 package.
`una check` rejects the `_secret` functions (U4001), so the export lists them under `rejected`;
the importer reads one complete document, so they are put back into the module, in source order,
and the document is marked complete. Nothing else of the export changes.

Usage: una --json core --with-rejected <pkg> > export.json
       python3 formal/gen/prelude_import_tests.py export.json formal/UnaFormal/F1/PreludeImportTests.lean
"""
import json, sys

ORDER = ["Oops", "abs_public", "abs_secret", "min_public", "min_secret", "max_public", "max_secret",
         "min_text", "or_else_public", "or_else_secret", "is_ok_public", "is_ok_secret",
         "slice_public", "slice_secret"]

EXPECT = [(n, "rejected" if n.endswith("_secret") else "not re-checked" if n == "min_text" else "accepted")
          for n in ORDER[1:]]

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
    guards = "\n".join(f'#guard run prelude "m.{n}" == "{v}"' for n, v in EXPECT)
    with open(out, "w", encoding="utf-8") as f:
        f.write(f'''/-
Tests of the reading of `abs`, `min`, `max`, `or_else`, `is_ok` and `slice` (#542), checked when
the library builds. The document is written by `formal/gen/prelude_import_tests.py` from `una`'s
own export of `formal/gen/prelude_import_tests.una`; each verdict matches what `una check` says
of the function: the `_public` functions are accepted, the `_secret` ones rejected (U4001), and
`min` on `Text`, which F1 does not order, is not re-checked (#532).
-/
import UnaFormal.F1.ImportTests

namespace Una.F1.PreludeImportTests

open Import ImportTests

def prelude : String := r#"{text}"#

{guards}

end Una.F1.PreludeImportTests
''')

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
