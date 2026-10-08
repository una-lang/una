# Summary

Toolchain: una 0.3.3 (editions 0.1, 0.2; z3 5.1.0). Every package is edition 0.2.

| Task | `una check` runs | Passing state reached | Tests / run |
| --- | --- | --- | --- |
| 1 greet | 1 | yes (run 1) | `una run` prints `hello, <user>`; `una test` 0 examples (none asked) |
| 2 orders | 7 | yes (run 7) | `una test` 1/1 passed (exact input and output of the task) |
| 3 stock | 8 | yes (run 8) | `una test` 1/1 passed (stand-in answers `{"count": 7}`, result `Some(7)`) |
| 4 tally | 3 | yes (run 3) | `una test` 3/3 passed; 3 map postconditions of `add_vote` proven |
| 5 sensors | 4 | yes (run 3; run 4 after a change) | `una test` 4/4 passed |

No task reached the limit of 10 runs; there is no GAVE UP.

Main finding: `una-0.3.3/docs` does not exist, although the task and README.md point to docs/guide/*.md. The only reference was README.md plus `una explain` for all 83 codes. Most extra check runs went into discovering undocumented library APIs on purpose (Json variants, `to_json`, `List.append`, `Response`/`Header` fields, the operations of `Http` and `Keys`) through compiler errors. U2007, U2009, U3011 and U2001 list variants, fields, operations and types, which made this possible; U1001 gives no suggestions.

Other doc gaps, detailed in each LOG.md:
- what `key_name` of an HTTP request does (stock);
- how labels apply to the elements of a container (stock);
- loop invariants, and naming an `edit` parameter's prior value in `ensures` (tally);
- the `together for` syntax and its failure-order guarantee, and how an example expects a failure (sensors).

After all tasks were done, I ran `una fmt --check`, `una check` and `una test` once more on all five packages, with no code changed, to confirm the final state: all exit 0. These re-runs are not counted above.
