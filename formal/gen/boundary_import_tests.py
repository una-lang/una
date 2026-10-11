"""Writes formal/UnaFormal/F1/BoundaryImportTests.lean (#562) from `una --json core`.

The package is `formal/gen/boundary_import_tests/`, edition 0.2, with the modules `app` and
`vendor`. The export does not write `una.toml`'s `[boundaries]` yet (edition 0.3, #562's R9), so
the documents are the export with a `boundaries` key added, one ceiling for `vendor` each:

- none: no key, as today's export;
- `read`: `Files[read]`, which `vendor.tiled`'s write exceeds;
- `write`: `Files[read, write]`;
- `empty`: no operation;
- `bad`: `Files[delete]`, an operation `Files` does not have, which is refused.

Usage: una --json core formal/gen/boundary_import_tests > export.json
       python3 formal/gen/boundary_import_tests.py export.json formal/UnaFormal/F1/BoundaryImportTests.lean
"""
import copy, json, sys

CEILINGS = {
    "read": ["Files[read]"],
    "write": ["Files[read, write]"],
    "empty": [],
    "bad": ["Files[delete]"],
}


def main(src, out):
    doc = json.load(open(src, encoding="utf-8"))
    assert doc["complete"] is True
    paths = sorted(m["path"] for m in doc["modules"])
    assert paths == ["app", "vendor"], paths
    texts = {"none": json.dumps(doc, separators=(",", ":"), ensure_ascii=False)}
    for name, ceiling in CEILINGS.items():
        d = copy.deepcopy(doc)
        d["boundaries"] = {"vendor": ceiling}
        texts[name] = json.dumps(d, separators=(",", ":"), ensure_ascii=False)
    for t in texts.values():
        assert '"#' not in t
    defs = "\n".join(f'def {n}Doc : String := r#"{t}"#\n' for n, t in texts.items())
    with open(out, "w", encoding="utf-8") as f:
        f.write(f'''/-
Tests of the reading of boundary modules (ADR 0093, #562), checked when the library builds. The
documents are written by `formal/gen/boundary_import_tests.py` from `una`'s own export of
`formal/gen/boundary_import_tests/`, with a `boundaries` key added (the export does not write one
yet, R9).
-/
import UnaFormal.F1.ImportTests

namespace Una.F1.BoundaryImportTests

open Import ImportTests

{defs}
-- Without boundaries, as today: the call into the library is read as any call.
#guard run noneDoc "app.thumbs" == "accepted"
#guard run noneDoc "app.plain" == "accepted"

-- `Files[read]`: the tiled mode writes, beyond the ceiling, so the call into it is rejected; the
-- plain mode is pure, and a declaration that calls nothing of the library is unchanged.
#guard run readDoc "app.thumbs" == "rejected"
#guard run readDoc "app.plain" == "accepted"
#guard run readDoc "app.own" == "accepted"

-- `Files[read, write]`: the write is within the ceiling.
#guard run writeDoc "app.thumbs" == "accepted"

-- No operation: the write is beyond it; the pure call is within it.
#guard run emptyDoc "app.thumbs" == "rejected"
#guard run emptyDoc "app.plain" == "accepted"

-- A ceiling F1 cannot read: no acceptance stands.
#guard run badDoc "app.thumbs" == "not re-checked"
#guard run badDoc "app.plain" == "not re-checked"

end Una.F1.BoundaryImportTests
''')


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
