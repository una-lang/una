# stock LOG

## Check run 1
- U2001: expected `Response`, found `Option[_]`
- U2001: `when` with variant arms examines a value of a sum kind, and this value is `Response`
- U2009 (x4): `Response` is missing field `headers`
- Consulted: message only
- Enough: yes
- Change: replaced `if error use None` with `attempt` and a `when` on the Result; added `headers: []` to the stand-in Responses.
- DOC: una-compact.md §Library says a Response has `status`, `body`; it also has a `headers` field, and the Http op signatures a handler must implement are not listed.

## Check run 2
- No diagnostics; exit 0.
- Consulted: n/a
- Enough: yes
- Change: none. (Result variants `Ok(..)` and an `else` arm were accepted; a non-200 status also gives nothing.)
- DOC: una-compact.md §Failures mentions `attempt f(x)` gives a `Result` but not the variant names (`Ok`, and the error one).

## una test
- `example stock "the stand-in API answers a count of 7" ... ok`, 1 passed, exit 0.
