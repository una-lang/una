# sensors LOG

Consulted before writing: docs: una-for-agents.md §18 (`together`, `together for`, first failure in task order), docs: syntax-reference.md §5.7 (failures cancel later tasks; handling clauses inside the task body), una-for-agents.md §8 (capabilities are not values, so the "list of sensors" is a list of sensor names read through one `SensorBank` capability), §7 (`attempt`).

## Check run 1
- Diagnostics: none (exit 0).
- Consulted: n/a
- Enough: n/a
- Change: first version: `read_all` (`together for` with `if error pass`, `ensures result.len == names.len`), `answered` (`together for` with `attempt`, counts `is_ok`, `ensures result <= names.len`), handler `bank_with_broken` that fails one sensor; three examples.

## Test run 1
- `una test`: 3 examples, 3 passed, exit 0. JSON trace of the failing example: task 1 reads "a", task 2 fails `Offline("bb")`, task 3 ("ccc") never reads: cancelled as documented.

## Check run 2
- Diagnostics: none (exit 0).
- Consulted: n/a
- Enough: n/a
- Change: added an example `do attempt read_all(...)` / `expect result == Err(Offline("bb"))` to check the exact failing sensor.

## Test run 2
- `una test`: 4 examples, 3 passed, 1 failed (exit 1): `expect not performed @bank.read` - "the trace has `@bank.read` at event 1".
- Consulted: message only
- Enough: yes
- Change: my own mistake in the new example (reads before the failure do happen); changed it to `expect performed @bank.read`.

## Check run 3
- Diagnostics: none (exit 0).
- Consulted: n/a
- Enough: n/a
- Change: the one-line fix above.

## Test run 3
- `una test`: 4 examples, 4 passed, exit 0.

DOC: neither guide shows a handler operation that fails (`op read(sensor) = if ... then fail V else ...`); it worked, but the docs only say handlers "implement every operation".
