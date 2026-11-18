# orders log

Design: `summarize(text: Text) -> Text`, pure (no `uses`), `fails OrderError` on text that is not an order.
Parses with `parse_json`, reads each line through `line_of`, and writes the result as an `Object` of a
`Map[Text, Json]` with `to_json()`, which orders the keys.

## Check run 1
- Diagnostics: none (exit 0).
- Consulted: docs: una-for-agents.md §15 (JSON), §7 (failing arms of `when`), §1.6/§3 (text escapes `\{` `\}`).
- Enough: yes.
- Changed: first version.

## una test
- Result: 2 examples, 2 passed (exit 0): the exact input/output pair, and a non-order input that fails Malformed.
- Consulted: nothing. Changed: nothing.
