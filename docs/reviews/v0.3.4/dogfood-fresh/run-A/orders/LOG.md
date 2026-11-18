# orders log

Prepared from: una-for-agents.md §15 (JSON: `Json` variants, `parse_json` fails with `JsonError`, `to_json` canonical compact form with keys in order), §3 (text escapes: a literal brace is `\{`), §7 (failures, `if error pass`), §14 (maps). Design: small failing helpers that pull typed fields out of a `Map[Text, Json]`, `map(each: line_of)` with `if error pass` to read the lines, then an `Object` of three fields written with `to_json`.

## Run 1 — `una fmt` + `una --json check`
- Diagnostics: none (exit 0).
- Consulted: n/a.
- Enough to fix: n/a.
- Changed: nothing; first version passed.

## `una test` (after run 1)
- 2 examples: 2 passed, 0 failed (exit 0). The example checks the exact input and output of the task: `{"count":2,"names":["a","b"],"total":5}`.
