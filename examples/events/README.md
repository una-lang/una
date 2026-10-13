# events

Event check-in: attendees check in by phone number, the phone is used only through a keyed hash, and every check-in is audited.

## What it shows

- `una.toml` declares the [label categories](../../docs/guide/configure/labels/categories.md) `PII` and `Location`, names `events.keys` as a [trusted module](../../docs/guide/configure/trusted/modules.md), and sets the [escape budget](../../docs/guide/configure/escape-budget/checked-per-module.md) to one `checked` per module.

`src/events/types.una`: `Phone = Text where valid_phone(it)`, a [refinement](../../docs/guide/language/types/where.md) by an [`opaque` predicate](../../docs/guide/language/types/predicate.md), and `parse_phone`, the only way to get one, with [`checked`](../../docs/guide/language/errors/checked.md).

`src/events/seating.una`:

- A [requirement](../../docs/guide/language/requirements/requirement.md), `EVENTS_3`: an event with no seats left is shown as sold out.
- `Seats = Int in 0..5000`, and `availability`, which maps the remaining seats to `SoldOut`, `Few` or `Open` with [`when`](../../docs/guide/language/control-flow/when.md) over ranges, and [`ensures`](../../docs/guide/language/functions/ensures.md) the requirement.

`src/events/keys.una`, a [`trusted module`](../../docs/guide/language/information-flow/trusted.md): the [`policy`](../../docs/guide/language/information-flow/policy.md) `attendee_key` turns a `secret[PII] Phone` into an `internal Hashed`, through the host capability `Keys`, for "attendee lookup and audit correlation".

`src/events/checkin.una`:

- `check_in` takes a `secret[PII] Phone` ([labels](../../docs/guide/language/information-flow/labels.md)) and returns an `internal` result. The phone itself never leaves the function: only its keyed hash does.
- Two [capabilities](../../docs/guide/language/capabilities/capability.md), `Registrations` and `Audit`, whose operations `accepts internal`, so they may receive the hash.
- The policy's failure depends on the phone, so `check_in` catches it as a value with [`attempt`](../../docs/guide/language/errors/attempt.md) and keeps every later outcome inside its `internal` result. The comment in the code gives the rule.
- Handlers for the examples, and three [`example`](../../docs/guide/language/examples/example.md) blocks. One checks that an unknown event fails before the phone is hashed (`expect not performed @keys.hmac`).

`src/events/main.una` prints the availability of one event.

## What `una check` proves

- `availability` shows an event without seats as sold out, as `EVENTS_3` requires, and its `when` covers every seat count.
- Every seat count is in `0..5000`, and every `Phone` came from `parse_phone`.
- The phone reaches the registrations and the audit log only as its keyed hash, through `attendee_key`. `una report .` lists that use of the policy, the one `checked`, and the trusted module.

## Run it

From this directory:

```sh
una check .   # copy examples/events
una test .
una report .
una run .
una build .
./target/una/events-example
```

`una run .` and the native executable print:

```text
availability of Launch: open
```

## Edition

Edition `0.1` ([edition](../../docs/guide/configure/package/edition.md)).
