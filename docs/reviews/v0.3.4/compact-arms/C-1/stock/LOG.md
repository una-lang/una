# stock log

## check run 1
- U4001 "parameter `body` of `count_of` is `Text`, and this value is `internal Text`: a flow from `internal` to `public`"
- U4001 "the result of `fetch` is `Int`, and this value is `internal Int`: a flow from `internal` to `public`"
- U4002 "`if none fail` depends on `internal` data, so its failure would reveal it; return a labeled value instead of failing"
- U3011 "handler `inventory` does not implement the operations `post`, `put`, `delete` of `Http`"
- U2009 "`Response` is missing field `headers`"
- consulted: message only (labels from una-compact.md §Labels; header policy from una explain U4001 / U6004 found earlier)
- enough: yes for the labels and U4002 (restructure: fetch only fails on HttpError, parsing returns an `internal Option[internal Int]`); partly for U3011/U2009 (they name what is missing but not the operation parameters or the field type)
- changed: label the parsing helpers `internal`, return `internal Option[internal Int]`, add post/put/delete ops and a `headers` field to the stand-in
- DOC: una-compact.md §Library/Http gives `get(url, headers:, key_name:)` and `Response (status, body)` but not the `headers` field of Response nor the signatures of post/put/delete, which a handler for Http must implement.

## check run 2
- U3007 "`@http.get` can fail with `HttpError`; handle the failure with an `if error` clause, or wrap the call in `attempt`" (as the last expression of a `fails HttpError` function)
- U4001 "the result of `fetch` is `Response`, and this value is `internal Response`: a flow from `internal` to `public`"
- consulted: message only
- enough: yes
- changed: `if error pass` under the get call; result type `internal Response`

## check run 3
- no diagnostics, exit 0 (PASS)

## test run 1
- 1 example passed, exit 0

## change: added a second stand-in (`outage`, status 503) and an example expecting `None`

## check run 4
- no diagnostics, exit 0 (PASS)

## test run 2
- 2 examples passed, exit 0
