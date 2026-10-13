"""Writes formal/UnaFormal/F1/HmacImportTests.lean (#587) from `una --json core --with-rejected`.

The packages are `formal/gen/hmac_import_tests/{plain,branch,interp,with}/`, edition 0.2, each with the
modules `main` and the trusted `release`, from the conformance cases `via_hmac_only_public_failure`,
`via_hmac_under_branch` and `via_hmac_interpolated_key` of ADR 0098 (#497); `with` adds a handler
of `Keys` that delegates under another key name, installed around `passes` by a `with`, as
`via_hmac_handler_in_with` does. Until #497 merges, `una check` rejects `main.passes` in all four
(U4002); the export lists it under `rejected`, which
is put back into its module, and the document is marked complete.

Usage: una --json core --with-rejected <pkg> > <name>.json   (for plain, branch, interp and with)
       python3 formal/gen/hmac_import_tests.py plain.json branch.json interp.json with.json \
         formal/UnaFormal/F1/HmacImportTests.lean
"""
import json, sys


def text(src):
    doc = json.load(open(src, encoding="utf-8"))
    for r in doc.pop("rejected", []):
        (mod,) = [m for m in doc["modules"] if m["path"] == r["module"]]
        mod["decls"].append(r["decl"])
    doc["complete"] = True
    paths = sorted(m["path"] for m in doc["modules"] if m["path"] != "prelude")
    assert paths == ["main", "release"], paths
    t = json.dumps(doc, separators=(",", ":"), ensure_ascii=False)
    assert '"#' not in t
    return t


def main(plain, branch, interp, with_, out):
    defs = "\n".join(f'def {n} : String := r#"{text(s)}"#\n'
                     for n, s in [("plain", plain), ("branch", branch), ("interp", interp),
                                  ("withHandler", with_)])
    with open(out, "w", encoding="utf-8") as f:
        f.write(f'''/-
Tests of the reading of `Keys.hmac` and of ADR 0098's `via` functions (#587), checked when the
library builds. The documents are written by `formal/gen/hmac_import_tests.py` from `una`'s own
export of `formal/gen/hmac_import_tests/`, with the declarations `una check` rejects put back. Each
verdict is ADR 0098's (#497): a `via` function of its shape fails at the program counter, and one
outside it leaves the program outside B1, so nothing in it is re-checked.
-/
import UnaFormal.F1.ImportTests

namespace Una.F1.HmacImportTests

open Import ImportTests

{defs}
-- ADR 0098's shape: one hmac call with a literal key name, its failure passed on. The policy's
-- failure is public, so passing it on in `passes` is accepted, as #497 accepts it.
#guard run plain "main.passes" == "accepted"
-- A `via` function itself is not re-checked: no label rule checks it, B1 does.
#guard run plain "release.hmac_it" == "not re-checked"

-- The hmac calls under a branch on the secret: outside the shape, so outside B1 ("impure via").
-- Never accepted.
#guard run branch "main.passes" == "not re-checked"

-- A key name interpolated from the secret: outside the shape too.
#guard run interp "main.passes" == "not re-checked"

-- A handler of `Keys` in the document may run in place of the host's: its `Keys.hmac` calls are
-- not read, so neither the `via` function nor what calls the policy is re-checked.
#guard run withHandler "main.passes" == "not re-checked"
#guard run withHandler "main.swapped" == "not re-checked"

end Una.F1.HmacImportTests
''')


if __name__ == "__main__":
    main(*sys.argv[1:6])
