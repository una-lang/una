# stock LOG

Consulted before writing: docs: una-for-agents.md §17 (Http, `internal Option[internal Int]`, `attempt`, stand-in handler with all four ops), §9 (policies), docs: syntax-reference.md §7.7 (Http row: token in another header goes through a policy to `internal` when `secret`), §8.5 (policy syntax).

## Check run 1
- Diagnostics: none (exit 0).
- Consulted: n/a
- Enough: n/a
- Change: first version: `stock(item: Text, token: secret Text) -> internal Option[internal Int] uses @http: Http[get]`; trusted module `release` with policy `api_token_header` (secret Text -> internal Text, strips CR/LF); item names that could change the URL path are refused before any request; stand-in handler `inventory` answers 200 only when `X-Api-Token` is sent.

## Test run 1
- `una test`: 3 examples, 3 passed, exit 0 (count 7 returned with `performed @http.get`; no `count` gives None; `../admin` refused with `not performed @http.get`). The JSON trace shows the header value as `<redacted:internal>`.

DOC: una-for-agents.md §17 says a `secret` token "needs a policy to `internal` first (§9)" but gives no example of that policy for a header; I adapted the shape of §9 / syntax-reference.md §8.5. Not stated whether the runtime percent-encodes interpolated URL parts, so I validated the item name myself.
