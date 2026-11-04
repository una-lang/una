"""Writes formal/UnaFormal/F1/LoopImportTests.lean (#946) from `una --json core --with-rejected`.

The package is `formal/gen/loop_import_tests.una` as the module `m`, the file `src/m.una`, of an
edition-0.4 package (ADR 0116) whose manifest `ita.una` is exactly these three lines and nothing
after them:

    ita loop_import_tests
      version 0.1.0
      edition 0.4

The export gives the arguments the elaborator synthesizes the span of the manifest's last line
plus one, `ita.una:4:1`, so another manifest gives another document; `una.toml` is U7002 for
edition 0.4. The package name does not reach the export, and no `ita.lock` is needed (`una check`
reports U7009 for its absence, which does not change the export). `una check` rejects `when_secret`, `under_secret` and `ever_secret` (U4002),
`after_skip` (U4001) and `loud_until` (U4003), so the export lists them under `rejected`; the importer reads one complete document, so they are
put back into the module, in source order, and the document is marked complete. Nothing else of
the export changes.

Usage: una --json core --with-rejected <pkg> > export.json
       python3 formal/gen/loop_import_tests.py export.json formal/UnaFormal/F1/LoopImportTests.lean
"""
import json, sys

ORDER = ["Log", "Order", "total", "first_square", "pairs", "kinds", "when_secret", "under_secret",
         "after_skip", "public_exits", "halves", "alive", "secret_hits", "secret_until", "loud_until",
         "ever_secret"]

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
Tests of the reading and re-check of ADR 0116's exits and loops (#946), checked when the library
builds. The document is written by `formal/gen/loop_import_tests.py` from `una`'s own export of
`formal/gen/loop_import_tests.una`, whose functions are those of the conformance cases
`runtime/loop_stop_and_skip`, `security/loop_exit_flow`, `runtime/until_runs` and
`runtime/for_ever_on_stop`; each verdict agrees with what `una check` says of the function, or is
"not re-checked", never wrong.
-/
import UnaFormal.F1.ImportTests

namespace Una.F1.LoopImportTests

open Import ImportTests

/-- The functions of `formal/gen/loop_import_tests.una`. -/
def loops : String := r#"{text}"#

-- `stop` and `skip` decided by public data, over lists and ranges, nested loops and blocks: the
-- model types them (T-Exit) and the re-check accepts them, as `una check` does.
#guard run loops "m.total" == "accepted"
#guard run loops "m.first_square" == "accepted"
#guard run loops "m.pairs" == "accepted"
#guard run loops "m.kinds" == "accepted"
#guard run loops "m.public_exits" == "accepted"
-- A `stop` decided by secret data: rejected, as `una check` rejects it (U4002, IF2).
#guard run loops "m.when_secret" == "rejected"
#guard run loops "m.under_secret" == "rejected"
-- A `skip` under a secret branch: IF3 raises the rest of the body, which the model's T-Exit does
-- not, so it is not re-checked (`una check` rejects this one, U4001); never accepted.
#guard run loops "m.after_skip" == "not re-checked"
-- `for until` and `for ever`, with `stop` and `on stop`: the model types and runs them (T-Until,
-- T-Ever, T1, T6, `Until.lean`), and the re-check accepts them, as `una check` does.
#guard run loops "m.halves" == "accepted"
#guard run loops "m.alive" == "accepted"
#guard run loops "m.secret_hits" == "accepted"
-- A `for until` on a secret condition runs its body under it (IF1): accepted when the body shows
-- nothing, rejected when it writes a public line (U4003), as `una check` says.
#guard run loops "m.secret_until" == "accepted"
#guard run loops "m.loud_until" == "rejected"
-- A `stop` in a `for ever` under a secret branch: rejected (U4002, IF2).
#guard run loops "m.ever_secret" == "rejected"

end Una.F1.LoopImportTests
''')

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
