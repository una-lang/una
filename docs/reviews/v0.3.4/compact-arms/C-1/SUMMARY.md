# Summary

Toolchain: una 0.3.3, edition 0.2, Z3 5.1.0. Docs available: README.md and docs/guide/una-compact.md only, plus `una explain` for all 83 codes.

| # | Task | `una check` runs | Failing runs before the first pass | Passing state | Tests / run |
|---|------|-----------------|------------------------------------|---------------|-------------|
| 1 | greet | 2 | 1 | yes | `una run` prints "Hello, <USER>!"; 0 examples (none required) |
| 2 | orders | 4 | 2 | yes | 2 examples pass (exact input/output; malformed input fails) |
| 3 | stock | 4 | 2 | yes | 2 examples pass (stand-in answers `{"count": 7}` gives `Some(7)`; 503 gives `None`) |
| 4 | tally | 2 | 1 | yes | 2 examples pass; 3 postconditions on the map proven |
| 5 | sensors | 2 | 1 | yes | 4 examples pass (first failure in list order, all answer, count answered = 2, earlier failure wins) |
| 6 | report | 3 | 1 (plus 1 failed `una run`: wrong file path) | yes | 2 examples pass; 6 obligations proven; `una run` prints "4 valid readings, sum 56" |
| 7 | allow | 6 | 5 | yes | 2 examples pass against a stand-in Keys handler |

Total: 23 check runs; no task gave up. Runs after the first pass were for extra examples or stricter behavior.

Main documentation gaps (details in each LOG.md under `DOC:`):
- The full guides the compact guide and README link to (una-for-agents.md, syntax-reference.md, getting-started.md) are not in the archive.
- Missing from the compact guide: what goes in verification.lock, `import` syntax, one `uses` line per capability, the leading `|` for a one-variant sum, and putting `=` on its own line before a multi-line body.
- Missing library signatures: `parse_int` fails with ParseError, `Response` has a `headers` field, the Http post/put/delete operations, `Keys.hmac(key_name, data:)` and its labels, the contents of `std.testing`, and how a path for `Files.read` is resolved under a `files:` grant.
