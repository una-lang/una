"""Writes formal/UnaFormal/F1/CollectionsImportTests.lean (#983) from `una --json core --with-rejected`.

The package is `formal/gen/collections_import_tests/`, an edition-0.5 package (ADR 0119, as its
amendment of 2026-11-06 targets it) with the modules `m`, which uses each collection of
`std.collections`, and `n`, which declares a kind of its own named `Queue` and does not import
`std.collections`. `una check` accepts the package, so the export is complete and its `rejected`
key is empty; the script checks both and writes the document unchanged.

Usage: una --json core --with-rejected formal/gen/collections_import_tests > export.json
       python3 formal/gen/collections_import_tests.py export.json formal/UnaFormal/F1/CollectionsImportTests.lean
"""
import json, sys

ORDER = {
    "m": ["plain", "stack_depth", "queue_front", "set_has", "hash_map_get", "hash_set_len",
          "priority_first", "bit_set_has", "set_param", "converted", "Ward", "ward_name", "caller",
          "size_of", "generic_at"],
    "n": ["Queue", "head_of"],
}


def name(d):
    return d["value"]["name"] if d["kind"] == "fun" else d.get("name", d.get("value", {}).get("name"))


def main(src, out):
    doc = json.load(open(src, encoding="utf-8"))
    assert doc["edition"] == "0.5" and doc["complete"] is True, (doc["edition"], doc["complete"])
    assert doc.pop("rejected", []) == []
    for path, order in ORDER.items():
        (mod,) = [m for m in doc["modules"] if m["path"] == path]
        assert list(map(name, mod["decls"])) == order, list(map(name, mod["decls"]))
    text = json.dumps(doc, separators=(",", ":"), ensure_ascii=False)
    assert '"#' not in text
    with open(out, "w", encoding="utf-8") as f:
        f.write(f'''/-
Tests of the importer's refusal of ADR 0119's collections (#983), checked when the library builds.
The document is written by `formal/gen/collections_import_tests.py` from `una`'s own export of
`formal/gen/collections_import_tests/`. F1 does not model the collections yet: a declaration that
uses one, in a type, an operation or a function of `std.collections`, or that reaches one, is not
re-checked, never accepted (ADR 0119 implementation plan, step 8). A kind of the program named as
a collection, in a module that does not import it, is the program's own and read as before.
-/
import UnaFormal.F1.ImportTests

namespace Una.F1.CollectionsImportTests

open Import ImportTests

/-- The reason of the verdict of `decl` in `doc`, empty for an outcome without one. -/
def reason (doc : String) (decl : String) : String :=
  match Lean.Json.parse doc with
  | .ok j =>
    match checkDocumentWith (fun rd σ t => t.lsubst rd σ) j {{}} with
    | .ok out =>
      match List.lookup decl out with
      | some (.notRechecked why) => why
      | _ => ""
    | .error e => s!"error: {{e}}"
  | .error e => s!"error: {{e}}"

/-- The modules of `formal/gen/collections_import_tests/`. -/
def colls : String := r#"{text}"#

-- The control: a function without a collection is accepted, as `una check` accepts it.
#guard run colls "m.plain" == "accepted"

-- Each collection, through its type, its operations and the function that makes it: not
-- re-checked, by the refusal of ADR 0119, never accepted.
#guard run colls "m.stack_depth" == "not re-checked"
#guard run colls "m.queue_front" == "not re-checked"
#guard run colls "m.set_has" == "not re-checked"
#guard run colls "m.hash_map_get" == "not re-checked"
#guard run colls "m.hash_set_len" == "not re-checked"
#guard run colls "m.priority_first" == "not re-checked"
#guard run colls "m.bit_set_has" == "not re-checked"
#guard (reason colls "m.stack_depth").endsWith ", ADR 0119)"
#guard (reason colls "m.queue_front").endsWith ", ADR 0119)"
#guard (reason colls "m.set_has").endsWith ", ADR 0119)"
#guard (reason colls "m.hash_map_get").endsWith ", ADR 0119)"
#guard (reason colls "m.hash_set_len").endsWith ", ADR 0119)"
#guard (reason colls "m.priority_first").endsWith ", ADR 0119)"
#guard (reason colls "m.bit_set_has").endsWith ", ADR 0119)"

-- A collection in a parameter's type only, whatever the body: the type is refused.
#guard run colls "m.set_param" == "not re-checked"
#guard (reason colls "m.set_param").endsWith "the collection `std.collections.Set`, ADR 0119)"

-- A collection only in the temporary of a conversion, `xs.to_set().len`.
#guard run colls "m.converted" == "not re-checked"
#guard (reason colls "m.converted").endsWith ", ADR 0119)"

-- A record with a collection field: a function that reads only its other field still takes the
-- record, so it is not re-checked.
#guard run colls "m.ward_name" == "not re-checked"

-- What reaches a collection: a call of a function that uses one, and a call of a generic
-- function at one, are not re-checked. The generic function itself calls no generic function, so
-- its body is checked once for every type parameter, as for any type F1 lacks: its acceptance is
-- of that body, which mentions no collection, and the program the theorem certifies holds no
-- instance at a collection, since `generic_at` is not in it.
#guard run colls "m.caller" == "not re-checked"
#guard run colls "m.generic_at" == "not re-checked"
#guard run colls "m.size_of" == "accepted"

-- The program's own kind `Queue`, in a module that does not import `std.collections`, is a record
-- of the program: the refusal is by the module, not by the name.
#guard run colls "n.head_of" == "accepted"

end Una.F1.CollectionsImportTests
''')


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
