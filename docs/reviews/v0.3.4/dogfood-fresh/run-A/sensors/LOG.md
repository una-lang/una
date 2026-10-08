# sensors log

Prepared from: una-for-agents.md §18 and syntax-reference.md §5.7 (`together for x in xs`, results in list order, "the block fails with the failure of the lowest-indexed failing task"), §7.6 (capability references are not values, so "a list of sensors" is a list of sensor names read through one `SensorBank` capability), §6.4 (`attempt`), §7.5 and §10 (handlers, `given`, `expect fails`). Design: `read_all` uses `together for` with `if error pass`; `answered` uses `together for` with `attempt` per sensor and sums 1/0; the stand-in `bank_of(readings: Map[Text, Int])` fails `Offline(sensor)` for any sensor not in the map.

## Run 1 — `una fmt` + `una --json check`
- Diagnostics: none (exit 0).
- Consulted: n/a.
- Enough to fix: n/a.
- Changed: nothing; first version passed.

## `una test` (after run 1)
- 4 examples: 4 passed, 0 failed (exit 0): all answer in list order; one sensor fails so `read_all` fails `Offline`; with two failing sensors `attempt read_all(...)` is `Err(Offline("west"))`, the first in list order (the task of the later failing sensor never ran: the trace shows it cancelled before its operation); `answered` gives 2 of 3 with one failing.
- Observation: `una test` prints the failure as `Offline("south")`, without the field name `sensor:`, while una-for-agents.md §4 shows records printed with field names (`Point(x: 1, y: -2)`).
DOC: una-for-agents.md §4 says interpolation and `una test` write values "in their Una form" as `Point(x: 1, y: -2)`, but a one-field variant prints as `Offline("south")`, without its field name; the rule for when field names are shown is not stated.
