"""Writes formal/UnaFormal/F1/ServerImportTests.lean (#1012) from `una --json core --with-rejected`.

The package is `formal/gen/server_import_tests/`, an edition-0.4 package (ADR 0123) with the
modules `m`, which uses each construct of the server (its kinds, `ReplyRight`, the operations
`next`, `reply` and `close` of `HttpServer`, a grant of it, a handler of it, and the function of a
`serve` section), and `n`, which declares a kind of its own named `Request` and imports nothing
of `std.host`. `una check` accepts the package, so the export is complete and its `rejected` key
is empty; the script checks both and writes the document unchanged.

It also checks that the importer's `serverKinds` (`formal/UnaFormal/F1/Import.lean`) is the
checker's `SERVER_CORE_KINDS` (`crates/una-check/src/types/elab.rs`), the kinds Core declares in
`prelude` for a package that imports a name of the server. The gate that keeps them, and
`serverTestingNames` with `STD_TESTING_SERVER_NAMES`, in step on every change is the test
`server_kinds::the_formal_importer_refuses_every_name_of_the_server` of `una-check`.

Usage: una --json core --with-rejected formal/gen/server_import_tests > export.json
       python3 formal/gen/server_import_tests.py export.json formal/UnaFormal/F1/ServerImportTests.lean
"""
import json, os, re, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

ORDER = {
    "m": ["plain", "header_value", "handle", "created", "ok_status", "method_of", "param_name",
          "why", "refused", "shown", "same", "serve_one", "answer", "stop_serving", "granted",
          "created_status", "caller", "closed"],
    "n": ["Request", "path_of"],
}


def name(d):
    return d["value"]["name"] if d["kind"] == "fun" else d.get("name", d.get("value", {}).get("name"))


def quoted_list(text, pattern):
    m = re.search(pattern, text, re.S)
    assert m, pattern
    return re.findall(r'"([A-Za-z]+)"', m.group(1))


def main(src, out):
    lean = open(os.path.join(ROOT, "formal/UnaFormal/F1/Import.lean"), encoding="utf-8").read()
    rust = open(os.path.join(ROOT, "crates/una-check/src/types/elab.rs"), encoding="utf-8").read()
    kinds = quoted_list(lean, r"def serverKinds : List String := \[(.*?)\]")
    core = quoted_list(rust, r"pub\(crate\) const SERVER_CORE_KINDS: \[&str; \d+\] = \[(.*?)\];")
    assert kinds == core, (kinds, core)
    doc = json.load(open(src, encoding="utf-8"))
    assert doc["edition"] == "0.4" and doc["complete"] is True, (doc["edition"], doc["complete"])
    assert doc.pop("rejected", []) == []
    for path, order in ORDER.items():
        (mod,) = [m for m in doc["modules"] if m["path"] == path]
        assert list(map(name, mod["decls"])) == order, list(map(name, mod["decls"]))
    (prelude,) = [m for m in doc["modules"] if m["path"] == "prelude"]
    declared = [name(d) for d in prelude["decls"] if d["kind"] == "data"]
    assert all(k in declared for k in kinds), (kinds, declared)
    text = json.dumps(doc, separators=(",", ":"), ensure_ascii=False)
    assert '"#' not in text
    with open(out, "w", encoding="utf-8") as f:
        f.write(f'''/-
Tests of the importer's refusal of ADR 0123's server (#1012), checked when the library builds.
The document is written by `formal/gen/server_import_tests.py` from `una`'s own export of
`formal/gen/server_import_tests/`. F1 does not model `next` as an input stream, `reply` as an
effect or the reply rights of `HttpServer` yet (ADR 0123 S10): a declaration that holds a kind of
the server, a `ReplyRight`, an operation, a grant or a handler of `HttpServer`, or that reaches
one, is not re-checked, never accepted. A kind of the program named as one of the server's, in a
module of its own, is the program's and read as before.
-/
import UnaFormal.F1.ImportTests

namespace Una.F1.ServerImportTests

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

/-- Whether `doc` declares the kind `prelude.k`. -/
def declaresPrelude (doc : String) (k : String) : Bool :=
  match Lean.Json.parse doc with
  | .ok j =>
    match j.getObjValD "modules" with
    | .arr ms => ms.any fun m =>
        m.getObjValD "path" == .str "prelude" &&
          match m.getObjValD "decls" with
          | .arr ds => ds.any fun d =>
              d.getObjValD "kind" == .str "data" &&
                (d.getObjValD "name" == .str k || (d.getObjValD "value").getObjValD "name" == .str k)
          | _ => false
    | _ => false
  | .error _ => false

/-- The modules of `formal/gen/server_import_tests/`. -/
def srv : String := r#"{text}"#

-- `serverKinds` names kinds Core declares for a package of the server, none of them stale.
#guard serverKinds.all (declaresPrelude srv)

-- The controls: a function without the server is accepted, and so is one of `Http`'s `Header`,
-- which a server's request holds too and F1 models.
#guard run srv "m.plain" == "accepted"
#guard run srv "m.header_value" == "accepted"

-- The function of a `serve` section (`spec/31 §7`), which the host's driver calls with each
-- request: not re-checked, as the driver's `next` and `reply` are not in the model.
#guard run srv "m.handle" == "not re-checked"
#guard (reason srv "m.handle").endsWith ", ADR 0123)"

-- Each kind of the server and `ReplyRight`, in a parameter, a result, a constructor or a pattern.
#guard run srv "m.created" == "not re-checked"
#guard run srv "m.method_of" == "not re-checked"
#guard run srv "m.param_name" == "not re-checked"
#guard run srv "m.why" == "not re-checked"
#guard run srv "m.shown" == "not re-checked"
#guard run srv "m.same" == "not re-checked"
#guard (reason srv "m.created").endsWith ", ADR 0123)"
#guard (reason srv "m.method_of").endsWith ", ADR 0123)"
#guard (reason srv "m.param_name").endsWith ", ADR 0123)"
#guard (reason srv "m.why").endsWith ", ADR 0123)"
#guard (reason srv "m.shown") == "F1 restriction (the reply right `ReplyRight`, ADR 0123)"

-- `TestRequestError`, the server's kind of `std.testing` (#1012, slice 2), refused by its module.
#guard run srv "m.refused" == "not re-checked"
#guard (reason srv "m.refused") == "F1 restriction (the server's `std.testing.TestRequestError`, ADR 0123)"
#guard serverTestingNames.contains "TestRequestError"
#guard (reason srv "m.same").endsWith ", ADR 0123)"

-- `Status` is an `Int` with a refinement in Core, no construct of the server.
#guard run srv "m.ok_status" == "accepted"

-- `next`, the input stream, and `reply`, the effect (ADR 0123 S10); `close`, with no kind of the
-- server in the declaration; and a grant of `HttpServer` that the body does not use.
#guard run srv "m.serve_one" == "not re-checked"
#guard run srv "m.answer" == "not re-checked"
#guard run srv "m.stop_serving" == "not re-checked"
#guard run srv "m.granted" == "not re-checked"
#guard (reason srv "m.serve_one").endsWith ", ADR 0123)"
#guard (reason srv "m.answer").endsWith ", ADR 0123)"
#guard (reason srv "m.stop_serving") == "F1 restriction (the capability `HttpServer`, ADR 0123)"
#guard (reason srv "m.granted") == "F1 restriction (the capability `HttpServer`, ADR 0123)"

-- A `Reply` only in a temporary, and what reaches it by a call: not re-checked. The importer
-- before #1012 accepted these two, `created` and `param_name`: the server's kinds were records.
#guard run srv "m.created_status" == "not re-checked"
#guard (reason srv "m.created_status").endsWith ", ADR 0123)"
#guard run srv "m.caller" == "not re-checked"

-- A handler of `HttpServer`, as a test installs it: not re-checked, and its operations are not
-- read as functions of the program, since the document does not declare the capability.
#guard run srv "m.closed" == "not re-checked"
#guard run srv "m.closed.next" == "missing"

-- The program's own kind `Request`, in a module that imports nothing of the server, is a record
-- of the program: the refusal is by the module `prelude`, not by the name alone.
#guard run srv "n.path_of" == "accepted"

end Una.F1.ServerImportTests
''')


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
