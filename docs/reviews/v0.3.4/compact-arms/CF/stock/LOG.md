# LOG: stock

## Check run 1
- Diagnostics: none (exit 0).
- Consulted: docs: una-for-agents.md §17 (Http operations, labels, `attempt`, stand-in handler with param `answer`, header values), syntax-reference.md §7.7 (Http, secret header token through a policy to internal), §8.5 (policies in trusted modules).
- Enough: yes.
- Change: initial version: trusted policy `api_token_header` (secret Text -> internal Text); `stock_level(item, token: secret Text) -> internal Option[internal Int]` with `attempt @http.get(...)`, parses `count`; stand-in handler answers 200 only when an X-Api-Token header is present.

## Test run 1
- `una test`: 2 examples, 2 passed (exit 0), including `{"count": 7}` -> `Some(7)`.

## Check run 2
- Diagnostics: none (exit 0).
- Consulted: docs: una-for-agents.md §17 "Header values" (any control character other than tab aborts with U6004).
- Enough: yes.
- Change: also remove carriage returns (`\u{D}`) from the token before it becomes a header value.
- DOC: una-for-agents.md §17 only shows removing `\n`; it says every control character except tab aborts, but gives no prelude way to strip all control characters, so callers must chain one `replace` per character.

## Test run 2
- `una test`: 2 examples, 2 passed (exit 0).

## Check run 3 (final verification sweep)
- Diagnostics: none (exit 0); `una test` also exit 0.
- Consulted: (nothing)
- Enough: yes
- Change: none
