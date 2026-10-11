"""Writes formal/UnaFormal/F1/FirstImportTests.lean (#532) from `una --json core --with-rejected`.

The package is `formal/gen/first_import_tests.una` as the module `m` of an edition-0.2 package.
`una check` rejects `top_secret` and `head_read` (U4003), so the export lists them under `rejected`;
the importer reads one complete document, so they are put back into the module, in source order,
and the document is marked complete. Nothing else of the export changes.

Usage: una --json core --with-rejected <pkg> > export.json
       python3 formal/gen/first_import_tests.py export.json formal/UnaFormal/F1/FirstImportTests.lean
"""
import json, sys

ORDER = ["Log", "head_public", "top_secret", "head_read", "presence",
         "text_lt", "text_gt", "text_le", "text_ge", "int_lt"]

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
    with open(out, "w", encoding="utf-8") as f:
        f.write(f'''/-
Tests of the reading of `prelude.first`, of comparisons F1 does not model, and of the
reporting of refinement predicates (#532), checked when the library builds. The document is
written by `formal/gen/first_import_tests.py` from `una`'s own export of
`formal/gen/first_import_tests.una`; each verdict matches what `una check` says of the function.
-/
import UnaFormal.F1.ImportTests

namespace Una.F1.FirstImportTests

open Import ImportTests

/-- `Log.write` accepts `public`. `head_public(xs: List[Int]) = xs.first()`; `top_secret` matches
`first` of a `secret List[Text]` into a public print; `head_read` prints the head of a
`List[secret Text]`; `presence` matches it only for `Some` or `None` into a public print;
`text_lt`, `text_gt`, `text_le` and `text_ge` compare texts, and `int_lt` integers. -/
def firsts : String := r#"{text}"#

-- A public list: accepted.
#guard run firsts "m.head_public" == "accepted"
-- A list with a secret top label, its option matched into a public print: rejected, as `una check`
-- rejects it (U4003).
#guard run firsts "m.top_secret" == "rejected"
-- The head of a list of secret elements read into a public print: rejected (U4003).
#guard run firsts "m.head_read" == "rejected"
-- A list of secret elements matched only for `Some` or `None`: accepted. Reading `first` as
-- `first_where(xs, fn(_) -> true)` would reject it.
#guard run firsts "m.presence" == "accepted"

-- `<`, `>`, `<=` and `>=` on `Text`, which Una orders and F1 does not: not re-checked, never
-- rejected, and never read as the `Int` comparison.
#guard run firsts "m.text_lt" == "not re-checked"
#guard run firsts "m.text_gt" == "not re-checked"
#guard run firsts "m.text_le" == "not re-checked"
#guard run firsts "m.text_ge" == "not re-checked"
-- The same on `Int`: accepted.
#guard run firsts "m.int_lt" == "accepted"

-- The prelude's refinement predicate, which every export carries: outside the flow model, in its
-- own category (`spec/07 §7` rule 6), reported and never dropped.
#guard run firsts "prelude.is_json_number" == "outside the flow model"

end Una.F1.FirstImportTests
''')

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
