# events

Event check-in: attendees check in by phone number, the phone is used only through a keyed hash, and every check-in is audited.

This is the largest example, and it combines the ideas of the others in one realistic service. A phone number is personal data: the program validates it once, labels it `secret[PII]`, and lets it out of its function only as a keyed hash, through a reviewed policy. The registration store and the audit log are capabilities that see only the hash. A business rule about sold-out events is a requirement with a proof. Read it after `journal` and `shortlinks`.

What it demonstrates:

- a refinement by an [`opaque` predicate](../../docs/guide/language/types/predicate.md), established with [`checked`](../../docs/guide/language/errors/checked.md);
- a [requirement](../../docs/guide/language/requirements/requirement.md) proven by an [`ensures`](../../docs/guide/language/functions/ensures.md) on a [`when`](../../docs/guide/language/control-flow/when.md) over ranges;
- [labels](../../docs/guide/language/information-flow/labels.md) with a category, `secret[PII]`, and a [`policy`](../../docs/guide/language/information-flow/policy.md) in a [`trusted module`](../../docs/guide/language/information-flow/trusted.md) that releases a keyed hash through the host capability `Keys`;
- [capabilities](../../docs/guide/language/capabilities/capability.md) that `accepts internal` data, and [handlers](../../docs/guide/language/capabilities/handler.md) that fake them in [`example`](../../docs/guide/language/examples/example.md) blocks;
- three settings of `una.toml`: label [categories](../../docs/guide/configure/labels/categories.md), [trusted modules](../../docs/guide/configure/trusted/modules.md) and the [escape budget](../../docs/guide/configure/escape-budget/checked-per-module.md).

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

`main` shows the availability of one event with 820 seats left. The check-in itself needs a registration store, an audit log and a key store, so it runs in the examples, against fakes. `una test .` runs the examples of the three modules that have them:

```text
example events.checkin "a registered attendee is checked in and audited" ... ok
example events.checkin "an unknown attendee fails and is not audited" ... ok
example events.seating "an event without seats is sold out" ... ok
example events.seating "an event with many seats is open" ... ok
example events.seating "raw seat counts are clamped" ... ok
example events.types "malformed input is rejected" ... ok

6 examples: 6 passed, 0 failed
```

## How it works

The package has five modules, one per concern:

| Module | Holds |
| --- | --- |
| `events.types` | `Phone`, and `parse_phone`, the only way to get one |
| `events.seating` | the sold-out rule, seats and availability |
| `events.keys` | the trusted module: the policy that hashes a phone |
| `events.checkin` | the check-in service, its capabilities, fakes and examples |
| `events.main` | the program |

### The manifest

```text
[labels]
categories = ["PII", "Location"]

[trusted]
modules = ["events.keys"]

[escape_budget]
checked_per_module = 1
```

The categories are the kinds of sensitive data that the package's labels may name. The trusted module is the one place where a label may go down. The escape budget allows each module one run-time check in place of a proof. Each of the three is a decision that a reviewer finds in one file.

### A phone, validated once

```text
pub kind Phone = Text where valid_phone(it)

pub predicate opaque valid_phone(t: Text) -> Bool = t.len == 11 and is_digits(t)

pub parse_phone(raw: Text) -> Phone
  fails PhoneError
=
  checked(raw, valid_phone(it)) else fail Malformed
```

The predicate is `opaque`: the solver sees only its name, so a `Phone` can only come from a test at run time, here in `parse_phone`. That one `checked` spends the module's escape budget, and `una report .` lists it.

### A rule with a proof

```text
requirement EVENTS_3
  text "An event with no remaining seats is always shown as sold out"
  source policy "Box Office Handbook 2.1"
  owner box_office
  status active
```

```text
pub kind Seats = Int in 0..5000

pub availability(e: Event) -> Availability
  ensures e.remaining < 1 implies result is SoldOut
  derived from EVENTS_3
=
  when e.remaining
    in 0..0 -> SoldOut
    in 1..<20 -> Few
    in 20..5000 -> Open
```

The `ensures` is the requirement in logic, and `derived from` links the two. `1..<20` excludes its upper end. The solver proves the `ensures` and proves that the three ranges cover every `Seats` value. `clamp_seats` turns any `Int` into a `Seats`, and the solver proves that its result is in range.

### Releasing a keyed hash: the policy

```text
trusted module events.keys

import events.types (Phone)

pub policy attendee_key
  from secret[PII] Phone
  to internal Hashed
  via hmac_phone
  purpose "attendee lookup and audit correlation"
  owner privacy

hmac_phone(phone: secret[PII] Phone) -> internal Hashed
  uses @keys: Keys[hmac]
  fails KeyError
=
  @keys.hmac(key_name: "phone_pepper", data: phone)
    if error pass
```

The policy lowers a `secret[PII] Phone` to an `internal Hashed`: a value that identifies the attendee for lookups, but from which the phone cannot be read back. `Keys.hmac` computes an HMAC-SHA-256 with a key that the host holds, so the program never sees the key either.

The policy can fail, for example when the host has no key of that name. A failure is observable, so the checker labels it. For most policies, the failure carries the label of the policy's result. This `via` function has a narrow shape: its only failure point is a `Keys.hmac` call with a literal key name, at the top level of its body, handled by `if error pass`, and the package installs no `Keys` handler of its own outside examples. Whether such a call fails depends on the key name, never on the phone, so the policy's failure stays `public`. That is why `check_in` may turn it into a failure of its own.

### The check-in service

```text
pub capability Registrations
  op find(key: internal Hashed) -> Option[Registration]
    fails StoreError
    accepts internal
```

The store's operation accepts `internal` data, so it may receive the hash. It is declared to take a `Hashed`, so it cannot receive the phone.

```text
pub check_in(phone: secret[PII] Phone) -> Availability
  uses @registrations: Registrations[find]
  uses @audit: Audit[record]
  uses @keys: Keys[hmac]
  fails CheckInError
=
  let key = attendee_key(phone)
    if error fail Unavailable(reason: "keys")
  let registration = @registrations.find(key)
    if error fail Unavailable(reason: "registrations")
    if none fail NotRegistered
  let status = availability(registration.event)
  @audit.record(key, status)
    if error fail Unavailable(reason: "audit")
  status
```

The header says everything the function may do: find registrations, record audits, compute keyed hashes, and fail with `CheckInError`. The phone is used once, as the argument of the policy. From there on the function works only with `key`.

The examples install fake stores with `given`. The second example checks a negative: an unknown attendee fails with `NotRegistered`, and nothing is written to the audit log (`expect not performed @audit.record`).

## What `una check` proves

- `availability` shows an event without seats as sold out, as `EVENTS_3` requires, and its `when` covers every seat count.
- Every seat count is in `0..5000`, and every `Phone` came from `parse_phone`.
- The phone reaches the registrations and the audit log only as its keyed hash, through `attendee_key`.

`una report .` lists what a reviewer must still read or trust. Among its lines:

```text
information:
  events.keys.attendee_key at src/events/checkin.una:30:13, purpose "attendee lookup and audit correlation", owner privacy
escapes:
  events.types: 1 checked, budget 1
assumptions:
  events.checkin.Audit: record
  events.checkin.Registrations: find
```

and, in the trusted base:

```text
  trusted modules: events.keys (17 lines)
  host capabilities: Console, Keys
```

The assumptions are the stores: what a real `Registrations` returns is given by its signature, not proven. The trusted module is 17 lines: the only code in the package that can lower a label.

## Common mistakes

### Putting the phone in an error message

A tempting change for debugging: say which phone was not found. Replace `if none fail NotRegistered` with `if none fail Unavailable(reason: "no registration for {phone}")`:

```text
[U4001] Error: field `reason` of `Unavailable` is `Text`, and this value is `secret[PII] Text`: a flow from `secret[PII]` to `public`
    ╭─[ src/events/checkin.una:34:38 ]
    │
 34 │     if none fail Unavailable(reason: "no registration for {phone}")
    │                                      ──────────────┬──────────────
    │                                                    ╰──────────────── field `reason` of `Unavailable` is `Text`, and this value is `secret[PII] Text`: a flow from `secret[PII]` to `public`
    │
    │ Note: run `una explain U4001` for an explanation
────╯
```

Error messages end up in logs. The checker treats them like any other output.

### Sending the phone to the store

Call the store with the phone, `@registrations.find(phone)`. The capability's signature already stops it, before any label is checked:

```text
[U2001] Error: argument `key` of operation `Registrations.find` must be `Hashed`, found `Text`
    ╭─[ src/events/checkin.una:32:42 ]
    │
 32 │   let registration = @registrations.find(phone)
    │                                          ──┬──
    │                                            ╰──── argument `key` of operation `Registrations.find` must be `Hashed`, found `Text`
    │
    │ Note: run `una explain U2001` for an explanation
────╯
```

### A key name the checker cannot see

In `hmac_phone`, move the key name into a binding: `let pepper = "phone_pepper"`, then `@keys.hmac(key_name: pepper, data: phone)`. The `via` function no longer has the shape described above, so the policy's failure carries the policy's result label, `internal`, and `check_in` may not turn it into a public failure:

```text
[U4002] Error: `if error fail` depends on `internal` data, so its failure would reveal it; return a labeled value instead of failing
    ╭─[ src/events/checkin.una:31:5 ]
    │
 30 │   let key = attendee_key(phone)
    │             ─────────┬─────────
    │                      ╰─────────── the labeled condition
 31 │     if error fail Unavailable(reason: "keys")
    │     ────────────────────┬────────────────────
    │                         ╰────────────────────── `if error fail` depends on `internal` data, so its failure would reveal it; return a labeled value instead of failing
    │
    │ Note: run `una explain U4002` for an explanation
────╯
```

> [!NOTE]
> The rule is conservative on purpose. The checker accepts a public failure only for the shape that it can recognize as independent of the data. Any other `via` function keeps the label of its result on its failure.

## Extend it

1. **A store that times out.** Add a handler `slow_registrations()` whose `find` fails with `Timeout`, and an example: `check_in` fails with `Unavailable`, and the audit log is not written. Six examples become seven.
2. **Few seats left.** Add an example to `seating.una` for an event with 5 seats left, and expect `Few`. Then change the first arm to `in 0..1 -> SoldOut`. `una check .` still accepts it: arms may overlap, the first match wins, and the `ensures` only says that no seats means sold out, not that sold out means no seats. Is that the rule that the box office wants? Strengthen the `ensures` so that the change is refuted.
3. **Check in from raw input.** Add `pub check_in_raw(raw: Text) -> Availability` to `events.checkin`, which parses the phone and calls `check_in`. Decide what a malformed phone becomes in `CheckInError`, and give that path an example.

## Learn more

- Construct pages: [predicates](../../docs/guide/language/types/predicate.md), [`checked`](../../docs/guide/language/errors/checked.md), [`requirement`](../../docs/guide/language/requirements/requirement.md), [labels](../../docs/guide/language/information-flow/labels.md), [`policy`](../../docs/guide/language/information-flow/policy.md), [`trusted module`](../../docs/guide/language/information-flow/trusted.md), [`capability`](../../docs/guide/language/capabilities/capability.md), [`handler`](../../docs/guide/language/capabilities/handler.md), [`if error`](../../docs/guide/language/errors/if-error.md), [`example`](../../docs/guide/language/examples/example.md).
- Configuration: [`categories`](../../docs/guide/configure/labels/categories.md), [trusted `modules`](../../docs/guide/configure/trusted/modules.md), [`checked_per_module`](../../docs/guide/configure/escape-budget/checked-per-module.md).
- Learn, the tutorial on the Una website: chapter 6, "Failure"; chapter 9, "Information flow"; chapter 10, "Testing".
- Specification: [policies (spec/07 §5)](../../docs/spec/07-information-flow.md), [trusted modules (spec/07 §6)](../../docs/spec/07-information-flow.md), [`checked` (spec/06 §6)](../../docs/spec/06-errors.md), [requirements (spec/08 §1)](../../docs/spec/08-requirements-and-examples.md), [examples (spec/08 §2)](../../docs/spec/08-requirements-and-examples.md), [host capabilities (spec/14 §3)](../../docs/spec/14-standard-library.md), [the build report (spec/13 §6)](../../docs/spec/13-diagnostics-and-abi.md).

## Edition

Edition `0.1` ([edition](../../docs/guide/configure/package/edition.md)).
