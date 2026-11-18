# stock log

Design: `stock_level(item: Text, token: secret Text) -> internal Option[internal Int]`, `uses @http: Http[get]`.
The secret token goes through the policy `token_header` (trusted module `token`, `secret Text` -> `internal Text`)
into the `X-Api-Token` header, with CR and LF removed (U6004 otherwise). The request is wrapped in `attempt`, since
its failure is `internal`; non-200 statuses, transport failures, unparsable bodies and a missing or non-integer
`count` all give `None`. Items that cannot be one URL path segment (empty, `/`, `?`, `#`) give `None` without a request.
The example installs a stand-in `Http` handler (all four operations) that answers `{"count": 7}`.

## Check run 1
- Diagnostics: none (exit 0).
- Consulted: docs: una-for-agents.md §17 (HTTP, stand-in handler, X-Api-Token via policy), §9 (policies), §16 (replace, contains_text); syntax-reference.md §7.7 (Http table).
- Enough: yes.
- Changed: first version.

## una test
- Result: 2 examples, 2 passed (exit 0): count 7 from the stand-in, and None for an answer without `count`.
- Consulted: nothing. Changed: nothing.

DOC: una-for-agents.md §17 says a header value with a control character "aborts the run with U6004" but does not say whether `\u{D}` (CR) counts; I removed both CR and LF to be safe. The example there removes only "\n".
