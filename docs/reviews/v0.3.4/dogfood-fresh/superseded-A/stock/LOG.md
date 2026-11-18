# stock log

Consulted before writing: docs: una-for-agents.md §17 (HTTP: operations, Header, Response, labels, "a secret ... header value needs a policy first", control characters), §9 and syntax-reference.md §8.5 (policies in trusted modules), §15 (JSON), syntax-reference.md §7.5 and §10 (handlers, examples with `given`).

## Check run 1
- U0005: "invalid escape `\r`; write `\\` for a backslash" (src/token.una)
- U1002: "`body` is already bound in this scope; a local cannot rebind another" (x2, handler parameter `body` vs op parameter `body` of post/put)
- Consulted: message only (plus docs: una-for-agents.md §3 for the list of escapes: `\u{H}` exists, `\r` does not).
- Enough to fix: yes.
- Change: `"\r"` became `"\u{D}"`; the handler parameter `body` was renamed `answer`.
DOC: una-for-agents.md §17 does not list the parameter names of `put` and `delete`, nor that a handler for `Http` must implement all four operations; I inferred `put(url, headers, body, key_name)` and `delete(url, headers, key_name)` and the compiler accepted them.
DOC: una-for-agents.md §3 lists the text escapes but no `\r`; a carriage return needs `\u{D}`. Worth a line, since §17 warns about control characters in header values.

## Check run 2
- Diagnostics: none, exit 0.
- Consulted: n/a.
- Enough to fix: n/a.
- Change: (the fix of run 1).

## Test run 1
- `una --json test`: 1 example, passed 1, failed 0, exit 0. Trace shows one `Http.get` to `https://inventory.example.com/v1/items/widget` with header `X-Api-Token` (value redacted as internal).

## Check run 3
- Diagnostics: none, exit 0.
- Consulted: n/a.
- Enough to fix: n/a.
- Change: the stand-in API now answers 200 only when `X-Api-Token` carries the expected token (401 otherwise); added examples for a rejected token and an answer without `count`.

## Test run 2
- `una --json test`: 3 examples, passed 3, failed 0, exit 0.
