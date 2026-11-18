# orders LOG

Consulted before writing: docs: una-for-agents.md §15 (Json kind, parse_json, to_json), §7 (`when` arm that fails), §1.6/§3 (text escapes `\{` `\}`), §14 (maps).

## Check run 1
- Diagnostics: none (exit 0).
- Consulted: n/a
- Enough: n/a
- Change: first version: `summary(order: Text) -> Text fails OrderError` (pure, no `uses`), helper `line_of`, output built as `Object(Map[Text, Json]).to_json()`; two examples (exact input/output, and non-JSON input).

## Test run 1
- `una test`: 2 examples, 2 passed, exit 0. Output text is exactly `{"count":2,"names":["a","b"],"total":5}`.

DOC: una-for-agents.md §5 says a pattern binding named like a binding already in scope is U1002, but `let fields = when item / is Object(fields) -> fields` was accepted. Probably fine (the pattern's `fields` is in the arm scope and the outer `let fields` is not yet bound), but the guide does not say when a `let`'s own name enters scope.
