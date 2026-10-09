# LOG: sensors

## Check run 1
- Diagnostics: none (exit 0).
- Consulted: docs: una-for-agents.md §18 and syntax-reference.md §5.7 (`together for`, first failure in list order), §7.5 (handlers may fail with the op's error kind), §7.6 (capabilities are not values, so the "list of sensors" is a list of names read through one `SensorBank` capability), §6.4 (`attempt`), §10 (`do attempt ...` with `expect result == Err(V)`).
- Enough: yes.
- Change: initial version: `read_all` (together for, `if error pass`), `answered` (together for with `attempt`, counts `is_ok`), stand-in handler `bank_with(failing)`.

## Test run 1
- `una test`: 4 examples, 4 passed (exit 0), including two failing sensors where the first in list order (`bb`) is the reported failure.

## Check run 2 (final verification sweep)
- Diagnostics: none (exit 0); `una test` also exit 0.
- Consulted: (nothing)
- Enough: yes
- Change: none
