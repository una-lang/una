# stock log

Before writing: prelude signatures in the binary (Http.get with `headers` and `key_name`, Header with `value: internal Text`, Response, HttpError), `una explain U4001` (a secret token must not go as a header value directly; Header.value cannot be raised), `una explain U4004`/`U4005` (policy in a trusted module), `una explain U6004` (example of an Http.get call with Header and `attempt`), `una explain U3014`/`U3010`/`U3011` (handlers in examples).
Design: the caller holds the token as `secret Text`; the trusted module `credentials` has a policy `api_token` (secret -> internal) whose only purpose is putting it in the `X-Api-Token` header, since `Header.value` is `internal`. `key_name` would not fit: it sends a bearer token from a keys grant, not a caller-held secret in a custom header.
DOC: U4001's explanation says to give a secret token "as the name of its key, `key_name`, rather than as a header's value", which conflicts with an API that wants a custom header (`X-Api-Token`); it does not say that a policy from `secret` to `internal` is the alternative, though that works.

## Check run 1
- U4001: "parameter `response` of `count_of` is `Response`, and this value is `internal Response`: a flow from `inter"
- Consulted: message only.
- Enough: yes.
- Changed: labeled the private helpers' parameters and results `internal`.

## Check run 2
- U4002: "`parse_json` can fail, and it is called under an `internal` condition, which would be revealed"
- U4001: "the result of `int_of` is `internal Option[Int]`, and this value is `Option[internal Int]`: a flow from `"
- Consulted: message only (plus una explain U4002, already read).
- Enough: yes.
- Changed: moved `attempt parse_json(...)` before the `if` on the internal status; result types became `internal Option[internal Int]`.
DOC: That a label must be written at each position of a type (`internal Option[internal Int]`, not just `internal Option[Int]`), and that `attempt` of a fallible call still counts as a failure under a raised pc, are only learnable from messages; the guide section on labels cited by U4001 is not in the archive.

## Check run 3
- Diagnostics: none (exit 0).
- Consulted: -
- Enough: yes.
- Changed: -

## Test run 1
- `una test`: 1 example, 1 passed.

Then: made the stand-in stricter (answers 7 only for the right URL and X-Api-Token header, else 401), added an unreachable stand-in and two more examples.

## Check run 4
- U2004: "argument 2 of `contains` needs a label; only the first argument may go without one"
- Consulted: message only (repair edit provided).
- Enough: yes.
- Changed: `headers.contains(item: Header(...))` (the method receiver counts as argument 1).

## Check run 5
- Diagnostics: none (exit 0).
- Consulted: -
- Enough: yes.
- Changed: -

## Test run 2
- `una test`: 3 examples, 3 passed, exit 0 (count 7 with the right token; None with a wrong token; None when the API is unreachable).
