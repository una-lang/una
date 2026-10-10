# sensors LOG

## Check run 1
- U3007: `@bank.read` can fail with `SensorError`; handle the failure with an `if error` clause, or wrap the
- Consulted: message only, then una explain U3007
- Enough: yes
- Change: added `if error pass` under `@bank.read(id)` in the `together for` body of `read_all`.
- DOC: una-compact.md §Together says "`together for x in xs`; the first failure in list order wins" but shows no example of its body, of what it yields (a list) or of how a failure leaves the body.

## Check run 2
- No diagnostics; exit 0.
- Consulted: n/a
- Enough: yes
- Change: none.

## una test (after run 2)
- 3 examples, 3 passed, exit 0.

## Check run 3
- No diagnostics; exit 0.
- Consulted: n/a
- Enough: yes
- Change: added a `two_broken` stand-in and an example where sensor 1 (Garbled) and sensor 3 (Offline) fail, showing the earlier one in list order wins (the one-failure bank stays as the required example).

## una test (after run 3)
- 4 examples, 4 passed, exit 0.
