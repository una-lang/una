# 0026. Neutral reference examples

- Status: Accepted
- Date: 2026-09-29
- Deciders: Leonardo

## Context

Reference examples must illustrate the language without tying it to any industry, organization or regulated domain. The previous reference example did not meet that bar. The reference example matters beyond illustration: the specification quotes it, the conformance suite and the snapshot tests exercise it, and milestone acceptance criteria name it.

## Decision

1. The reference example is `examples/events` (package `events-example`), about event seating and check-in, with modules `events.types`, `events.seating`, `events.keys`, `events.checkin` and `events.main`. It replaces the previous reference example everywhere, and specification fragments use the same names, so the two stay consistent.
2. It exercises every construct that the previous reference example exercised, at least once each:
   - an opaque predicate with a runtime body, established through `checked`, using one unit of the module's `checked` escape budget (`Phone`, `valid_phone`, `parse_phone`);
   - a range refinement (`Seats = Int in 0..5000`), record kinds (`Event`, `Registration`), and sums with and without payload (`CheckInError`, `Availability`);
   - a `when` with range arms whose exhaustiveness is an obligation, an `ensures` with `implies` derived from a requirement (`availability`, `EVENTS_3`), and a short-form function with nested `if` (`clamp_seats`);
   - a trusted module holding a declassification policy whose `via` function uses `Keys.hmac` (`events.keys`, `attendee_key`, `hmac_phone`);
   - capabilities with `fails` and `accepts internal` (`Registrations`, `Audit`), and a public function with three `uses`, a `fails` kind, `if error` and `if none` handling and one effect per line (`check_in`);
   - handlers, including a fake `Keys` built with `std.testing.fake_hash`, and examples with `given`, `expect fails`, `expect performed` and `expect not performed`;
   - `main` printing through `Console` after a `when`, and a `una.toml` with label categories, a trusted module, the escape budget and run grants.
3. Every example, specification fragment, test fixture and explain text stays domain-neutral. No document names a company, an employer or a real policy document.
4. The syntax-tree snapshots of the example and the diagnostic samples of `spec/13 §1` are regenerated once, as a consequence of this decision, and reviewed line by line (D-059).

## Consequences

- The example passes every completed milestone as the previous one did: it parses (M1), it is canonical (M2) and its names resolve (M3). Later milestones name it in their acceptance criteria.
- Documents written before this decision refer to "the previous reference example" and cite commits made before it.
- The previous reference example also declared a variant with the name of a prelude variant; the rule that allows it (ADR 0023) keeps its own conformance coverage, independent of the example.

## Alternatives considered

- **Keeping the previous reference example:** rejected by the context above.
- **Several small examples instead of one package:** the reference example exists to show every construct working together across modules; small examples already live in the conformance suite and the explain texts.
