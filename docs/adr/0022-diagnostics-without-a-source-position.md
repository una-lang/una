# 0022. Diagnostics without a source position

- Status: Provisional (implemented, pending human review)
- Date: 2026-09-29
- Deciders: Claude (overnight run), pending review by Leonardo

## Context

Every diagnostic has a required `primary` location (`spec/13 §1`, ADR 0012). When M0 was accepted, the owner asked for an ADR on how to locate diagnostics that have no natural position, to be proposed when the first such code is implemented: U7004 for a missing `verification.lock` in M3. U5003 (solver missing or mismatched, M8) and U8001 (internal compiler error) raise the same question later. The conformance runner requires expected positions to lie in files that exist (`compiler/testing.md §2`).

Decision log entry: D-036 (`docs/reviews/open-decisions.md`).

## Decision

1. `primary` stays required.
2. A diagnostic about something that has no position in any file is located at the empty range at line 1, column 1 of the file that declares what is checked: the package's `una.toml`, or the checked file itself for a package of one file.
3. So a missing `verification.lock` is U7004 at `una.toml` 1:1. When a file exists but a key is wrong or missing, the diagnostic points into that file: at the key or value, or at 1:1 of the file for a missing key.
4. Later codes follow the same rule; for example, U5003 points at the `solver_version` value of `verification.lock` when the file exists.

### Changes to other documents on acceptance

- `spec/13 §1` gains rules 1 and 2 in the `primary` row.

## Consequences

- The Agent ABI keeps one shape, and every consumer can rely on `primary`.
- Conformance cases can expect these diagnostics, since the anchor file exists.
- An agent reads the message, not the position, to learn what is missing; the message names the missing file.

## Alternatives considered

- **Point at the missing file itself:** a position in a file that does not exist cannot be shown or tested.
- **Make `primary` optional:** changes the ABI shape of every diagnostic for a handful of codes, against the M0 decision.
