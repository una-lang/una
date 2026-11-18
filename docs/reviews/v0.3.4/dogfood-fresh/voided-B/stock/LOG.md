# stock: log

DOC: una-0.3.3/docs is missing (spec/24, which the messages cite for HTTP, is not in the archive). What I knew before starting came from `una explain U6004` (the call `@http.get(url, headers: [Header(name:, value:)], key_name: None)` inside `attempt`), `una explain U4001` (a secret token goes to a request as `key_name`, not as a header value), `una explain U3014`/`U3015` (a handler installed with `given` in an example), and `una explain U4004`/`U4005` (how a policy is written).

## Run 1
- U4001: field `value` of `Header` is `internal Text`, and this value is `secret Text`: a flow from `secret` to `int
- U2001: expected `Response`, found `Option[_]`
- U2001: `shape` is declared as `Int`, found `Response`
- Consulted: message only, plus una explain U4001 (read before).
- Enough: partly. It names the two options (a policy, or `key_name`) and the result type `Response`, but not what `key_name` does with the key.
- Changed: probe: built an empty `Response()` and passed an `Int` as `key_name` to learn their shapes.

## Run 2
- U2001: argument `key_name` of operation `Http.get` must be `Option[Text]`, found `Int`
- U2009: `Response` is missing fields `status`, `headers`, `body`
- Consulted: message only.
- Enough: yes for the shapes.
- Changed: probe: built an empty `Header()`, called an unknown `Keys` operation, and bound each `Response` field to `Int`.

## Run 3
- U2009: `Header` is missing fields `name`, `value`
- U1001: capability `Keys` has no operation `zzz`
- U2001: expected `Response`, found `Option[_]`
- U2001: `s` is declared as `Int`, found `List[Header]`
- U2001: `b` is declared as `Int`, found `Text`
- U4001: `t` is `Int`, and this value is `internal Int`: a flow from `internal` to `public`
- Consulted: message only.
- Enough: yes for the types. U1001 on a capability does not list its operations.
- Changed: probe: empty handlers for `Http` and `Keys`, so that U3011 lists their operations.

## Run 4
- U3011: handler `fake_api` does not implement the operations `post`, `put`, `delete` of `Http`
- U3011: handler `fake_keys` does not implement the operation `hmac` of `Keys`
- U3010: capability `Keys` has no operation `zzz`
- U2001: expected `Response`, found `Option[_]`
- Consulted: message only.
- Enough: partly. Decision: `Keys` has only `hmac`, and nothing I can read says which header, if any, a `key_name` token is sent in. The task wants the caller's secret value in `X-Api-Token`, so I took the compiler's other repair: a trusted policy that lowers `secret Text` to the `internal Text` that `Header.value` declares. The token never becomes `public`.
- Changed: wrote the implementation: the trusted module `token_policy` with policy `api_token_header`, `stock_level(item, token: secret Text)`, JSON helpers, and a stand-in `fake_api` that answers `{"count": 7}` only when the `X-Api-Token` header is present (401 otherwise).
DOC: `una explain U4001` says to give a secret token "as the name of its key, `key_name`", but no text I could reach says what the runtime does with `key_name`: which header it fills, in what format, or how it relates to `Keys` (whose only operation is `hmac`) and to the `keys:<file>` grant, whose file format is also undocumented here.

## Run 5
- U2001: expected `Response`, found `Option[_]`
- U4001: parameter `body` of `count_of` is `Text`, and this value is `internal Text`: a flow from `internal` to `public`
- U4001: parameter `headers` of `has_token` is `List[Header]`, and this value is `internal List[internal Header]`: a fl
- Consulted: message only, and una explain U6004 again (for `attempt`).
- Enough: yes.
- Changed: `attempt @http.get(...)` and a `when` over `Ok`/`Err`; `internal` labels on the helpers' parameters and results.

## Run 6
- U4001: parameter `value` of `as_int` is `Option[Json]`, and this value is `internal Option[Json]`: a flow from `inte
- U4001: `found` is labeled `public`, and this update runs under a `internal` condition, which it would reveal
- Consulted: message only.
- Enough: yes.
- Changed: labeled `as_int` and `int_of` `internal`; `let edit found: internal Bool = false`.

## Run 7
- U4001: the result of `int_of` is `internal Option[Int]`, and this value is `Option[internal Int]`: a flow from `inte
- Consulted: message only, and una explain U4001 ("each position of its type is labeled below the declared label there").
- Enough: yes, though it took a moment to see that `internal Option[Int]` labels only the outer position.
- Changed: `internal Option[internal Int]` on all four results.
DOC: nowhere I could read explains whether a label on `Option[...]` covers its element; the U4001 text implies per-position labels, but an example with a labeled container would make it clear.

## Run 8
- Diagnostics: none (exit 0). PASS.

## una test
- `example stock "the API answers a count of 7" ... ok`: 1 passed, 0 failed, exit 0.

Note: `item` is interpolated into the URL path as is; an item text with `/` or `..` would change the path. The task does not say what item names look like, so I left it as is.
