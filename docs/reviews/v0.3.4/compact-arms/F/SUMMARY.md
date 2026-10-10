# Summary

Toolchain: una 0.3.3, Z3 5.1.0. Every package is edition 0.2; `una check` exits 0 for all seven.

| Task | `una check` runs | Passing state | `una test` | `una run` |
| --- | --- | --- | --- | --- |
| 1. greet | 1 | yes (run 1) | 0 examples, exit 0 | prints `Hello, <USER>! Welcome to Una.` |
| 2. orders | 1 | yes (run 1) | 2/2 passed | - |
| 3. stock | 1 | yes (run 1) | 3/3 passed | - |
| 4. tally | 1 | yes (run 1) | 3/3 passed | - |
| 5. sensors | 3 | yes (all 3 passed; runs 2-3 added and fixed an example) | 4/4 passed (one failing run from a wrong `expect` I wrote) | - |
| 6. report | 3 | yes (run 2; run 3 after changing the read path) | 2/2 passed | prints `4 valid readings, sum 56` |
| 7. allow | 1 | yes (run 1) | 2/2 passed | - |

No task gave up. Only one compiler diagnostic was hit in total (U4001 twice, in report), fixed with `una explain U4001`. The one runtime failure (U6001 `NotFound` in report) came from an undocumented rule: a `Files.read` path is relative to the granted directory.

Documentation notes (the `DOC:` lines in each LOG.md):
- report: how a `Files.read` path resolves against a `files:` grant is not documented.
- allow: the variants of `KeyError` and `FileError` are not documented; `Keys.hmac`'s `-> internal Hashed` versus "carries the label of `data`".
- stock: no example of a policy that turns a `secret` token into an `internal` header value; URL interpolation encoding is not described.
- greet: the full Env-to-Console policy example lives only in `una explain U4003`.
- orders: when a `let`'s own name enters scope (a pattern of the same name inside its `when` was accepted).
- tally: whether several `ensures` clauses are allowed is not stated (they are).
- sensors: no documented example of a handler operation that fails.
