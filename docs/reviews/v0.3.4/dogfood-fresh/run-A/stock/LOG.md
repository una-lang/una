# stock log

Prepared from: una-for-agents.md §17 (HTTP: `Http[get]`, `get(url, headers, key_name)`, internal `Response`, `attempt` on the request, "a `secret` URL, body or header value needs a policy first", strip control characters from header values), §15 (parse, then branch), §9 and syntax-reference.md §8.5 (policies in trusted modules), §7.5 and §10 (handlers, `given`). Design: a trusted module `token` with a policy `api_token_header` (secret Text -> internal Text, removing newlines and carriage returns); `stock_level(item, api_token: secret Text) -> internal Option[internal Int]`; the stand-in is a handler for the host capability `Http`.

DOC: una-for-agents.md §17 names the operations `put` and `delete` but gives no parameter lists for them, nor for `post` beyond its names, and does not show the `Response` and `Header` constructors' field lists in one place; syntax-reference.md has no HTTP section at all (§7.7 host-capability table lacks `Http`). I guessed `put(url, headers, body, key_name)` and `delete(url, headers, key_name)` for the handler, and the checker accepted them.

## Run 1 — `una fmt` + `una --json check`
- U0005 | invalid escape `\r`; write `\\` for a backslash
- Consulted: message only (plus una-for-agents.md §3, the list of text escapes, which indeed has no `\r`).
- Enough to fix: yes.
- Changed: wrote the carriage return as `"\u{D}"` instead of `"\r"` in the policy's `via` function.

## Run 2 — `una fmt` + `una --json check`
- Diagnostics: none (exit 0).
- Consulted: n/a.
- Enough to fix: n/a.
- Changed: nothing.

## `una test` (after run 2)
- 1 example: 1 passed (exit 0).

## Run 3 — `una fmt` + `una --json check` (after strengthening the tests)
- Diagnostics: none (exit 0).
- Consulted: n/a.
- Enough to fix: n/a.
- Changed (before this run): the stand-in now answers 200 only when the request carries an `X-Api-Token` header (401 otherwise), and a second example checks that an answer that is not JSON gives `None`.

## `una test` (after run 3)
- 2 examples: 2 passed, 0 failed (exit 0). `una --json test` shows the trace: one `Http.get` of `https://inventory.example.com/v1/items/widget` with headers `X-Api-Token` (value redacted as internal) and `Accept`; result `Some(7)`.
- Note: `item` is interpolated into the URL path as is; an item with `/` or `?` would change the path. Not required by the task; a caller should pass plain item ids.
