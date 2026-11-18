# Summary

Toolchain: una 0.3.3 (editions 0.1, 0.2; Z3 5.1.0). Docs available in the archive: README.md and docs/guide/una-compact.md only; the other guides the README links to are not in the archive, so `una explain <code>` and compiler messages carried most of the learning.

| # | Task | `una check` runs | First passing run | Passing at the end | `una test` | `una run` |
|---|------|------------------|-------------------|--------------------|------------|-----------|
| 1 | greet | 3 | 3 | yes | 0 examples (none asked) | `Hello, <user>! Your user name has <n> letters.` |
| 2 | orders | 4 | 3 | yes | 1/1 pass | n/a |
| 3 | stock | 2 | 2 | yes | 1/1 pass | n/a |
| 4 | tally | 2 | 2 | yes | 2/2 pass | n/a |
| 5 | sensors | 3 | 2 | yes | 4/4 pass | n/a |
| 6 | report | 7 | 4 | yes | 1/1 pass | `4 valid readings, sum 56` |
| 7 | allow | 3 | 3 | yes | 2/2 pass | n/a |

Total: 24 `una check` runs; no task gave up. Runs after the first passing one were improvements (orders: fail on a non-number qty; sensors: an extra ordering example; report: a file read failure made `main` fail instead of printing a false zero, which took two more runs to find the name `FileError`). A final `una --json check` of all seven packages returned exit 0 for each.

Main documentation gaps (details in the `DOC:` lines of each LOG.md):
- verification.lock contents are only in `una explain U7004`.
- Several `uses` = one `uses` line each; calling a policy by its name after `import`; `kind E = A` with one bare variant is an alias, not a sum.
- Library signatures missing: Json payload types, `Response.headers`, Result variant names, `parse_int` failing `ParseError`, `Keys.hmac(key_name, data:)` and its result label, the std.host failure kind names (`FileError`, `KeyError`).
- The `files:<path>:<access>` grant form and paths relative to the granted directory; `together for` body and result shape; `old(p).len` rather than `old(p.len)`; `when` arms are expressions only.
