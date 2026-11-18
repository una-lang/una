# orders log

Before writing: used the prelude signatures embedded in the `una` binary (Json, parse_json, to_json, empty_map, append) and `una explain U0007` (JSON in text literals: `{` and `}` must be escaped as `\{` `\}`), `una explain U3007` (if error handling), `una explain U2019` (`let edit`).
DOC: Map methods (`get(key:)`, `set(key:, value:)` as an `edit` method) are documented nowhere available; I only saw them in the README "A taste" snippet.
DOC: README and U4001 mention "the guide's section on labels" and spec sections (`spec/23 §4` for canonical JSON) that are not in the archive; the key order of `to_json` (sorted keys, no spaces) had to be confirmed by running the example.

## Check run 1
- Diagnostics: none (exit 0).
- Consulted: una explain U0007, una explain U3007, una explain U2019; prelude strings in binary (before writing).
- Enough: yes.
- Changed: initial version (`summarize` parses JSON, walks `lines`, builds Map[Text, Json] and `to_json`s it; fails `OrderError.Malformed`).

## Test run 1
- `una test`: 1 example, 1 passed, exit 0.
- Sanity check (copy under tmp/): changing the expected total to 6 makes the example fail and shows the actual `{"count":2,"names":["a","b"],"total":5}`, so the example really checks the output.
