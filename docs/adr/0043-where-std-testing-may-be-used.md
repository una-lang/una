# 0043. Where `std.testing` may be used

- Status: Accepted
- Date: 2026-09-30
- Deciders: Leonardo (answer E9 A to the v0.1 review, on 2026-09-30), on the reviewer's recommendation
- Resolves: review finding R-017

## Context

`spec/14 §4` allows `std.testing` inside handler bodies and example lines, and `spec/03 §1` says `Hashed` values are produced only by host capabilities. But a handler that calls `fake_hash` could be installed with `with` in `main`, so fake digests reached production runs (R-017).

## Decision

1. A function of `std.testing` is called only in example lines and in the operation bodies of handlers that are installed only by `given` lines of examples.
2. A handler that uses `std.testing` and is installed by a `with` binding is U3015 at the `with` binding, with the handler's use of `std.testing` as a related span. A use is a call of, or a reference to, a function of `std.testing` in one of the handler's operation bodies; the related span is the first one. Every `with` binding counts, including one inside an example line.
3. A function of `std.testing` used as a value, not called, is U3015 everywhere.
4. `spec/14 §4` and ADR 0028 rule 7 (by amendment here) state the rule.

## Consequences

- Fake handlers stay available to examples, and production code cannot reach them.

## Alternatives considered

- **Allow `std.testing` in any handler and report it (E9 B).** Not chosen: fake digests could reach production runs.
