# 0048. Hard links are outside the v0.1 model

- Status: Accepted
- Date: 2026-09-30
- Deciders: Owner (answer E4 A to the re-review, 2026-09-30), on the reviewer's recommendation
- Resolves: re-review finding N-013
- Amends: ADR 0038 D9

## Context

ADR 0038 D9 makes `Files` confine paths through symbolic links. A hard link inside the grant to a file outside it is an ordinary directory entry, so it can be read and overwritten through `Files` (N-013). Denying every file with more than one link would also deny legitimate hard-linked files.

Before answering, the closure session confirmed that `Files` has no operation that creates links: it has only `read` and `write` (`spec/14 §3`). So a Una program cannot plant a hard link; only someone with other access to the granted directory can.

## Decision

1. Hard links are outside the v0.1 model, as races between the check and the use of a path already are (ADR 0038 D9 rule 4).
2. The build report lists them with the out-of-model channels, and `spec/07 §1`, `spec/13 §6` and `spec/14 §3` state it.

## Consequences

- The report's out-of-model list has seven entries.

## Alternatives considered

- **Deny any file with more than one link (E4 B).** Not chosen: it denies legitimate hard-linked files.
