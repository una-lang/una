# orders log

Consulted before writing: docs: una-for-agents.md §15 (JSON: Json kind, parse_json, to_json), §3 (text escapes `\{` `\}`), §7 (failures, handling clauses), §11 (let edit, for), §14 (maps).

## Check run 1
- Diagnostics: none, exit 0.
- Consulted: n/a (written up front from docs: una-for-agents.md §15, §3, §7).
- Enough to fix: n/a.
- Change: first version: `summarize(text) -> Text fails OrderError` parses, walks `lines` with a for loop, builds an Object map and writes it with `to_json()`.

## Test run 1
- `una --json test`: 2 examples (the exact input/output of the task, and a non-JSON input failing NotJson), passed 2, failed 0, exit 0.
- Note: the output is `{"count":2,"names":["a","b"],"total":5}`, keys in ascending order as `to_json` writes them.
DOC: una-for-agents.md §15 does not say whether a `when` arm may `fail` while binding a `let` (`let x = when j ... else -> fail V`); it worked, but the guide shows no example of it.
