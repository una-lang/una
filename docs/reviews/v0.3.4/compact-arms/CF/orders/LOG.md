# LOG: orders

## Check run 1
- Diagnostics: none (exit 0).
- Consulted: docs: una-for-agents.md §15 (JSON kinds, parse_json, to_json), §7 (a `when` arm that fails), §14 (maps); getting-started.md §4 (building an Object from a Map); una-for-agents.md §3 (escaping braces in text literals as `\{` `\}`).
- Enough: yes.
- Change: initial version: `summary(text)` parses, maps each line object with `line_of`, builds `Object` of count/names/total and writes it with `to_json`; fails with `OrderError`.

## Test run 1
- `una test`: 2 examples, 2 passed (exit 0). The exact output `{"count":2,"names":["a","b"],"total":5}` matched.

## Check run 2 (final verification sweep)
- Diagnostics: none (exit 0); `una test` also exit 0.
- Consulted: (nothing)
- Enough: yes
- Change: none
