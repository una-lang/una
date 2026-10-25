# 0041. The proof cache is part of the trusted base in v0.1

- Status: Accepted
- Date: 2026-09-30
- Deciders: Leonardo (answer E6 A to the v0.1 review, on 2026-09-30), on the reviewer's recommendation
- Resolves: review finding R-018 and note R-042

## Context

`spec/12 §9` keys each cached solver answer by the hash of its query and reuses it without asking the solver. Nothing authenticates an entry, so anyone who can write `.una/` can turn a `refuted` or `unknown` answer into `proven` (R-018). A warm cache also answers without Z3 (R-042).

## Decision

1. **Trusted.** The proof cache of a package (`.una/cache/`) is part of the Trusted zone in v0.1. The charter's proof boundary (§3), `spec/12 §9` and the Trusted zone of the build report (`spec/13 §6`, the `tcb` section's `proof_cache` field) name it. The owner's answer E6 A authorizes this edit of the charter.
2. **Counted.** The build report states how many obligation results came from the cache: the `cached` field of its `obligations` section (`spec/13 §6`), also printed by the human report. `una obligations` already marks each one `(cached)`.
3. **Deferred.** Authenticating or re-validating cache entries goes to `plan/backlog.md`.

## Consequences

- A writable `.una/` stays a way to forge proofs, but the boundary states it, and a clean cache (`rm -r .una`) re-verifies everything.
- The `una.report/1` document gains two fields, `obligations.cached` and `tcb.proof_cache`. The schema name is kept: fields are added, none is removed or changed.

## Alternatives considered

- **Authenticate or re-validate entries now (E6 B).** Not chosen: a new mechanism before v0.1 closes.

## Amendment (2026-10-25, advisory UNA-SA-019, #1164)

On the 0.1 line, rule 1 (the trusted proof cache) is superseded from v0.1.13 by the authenticated cache of advisory UNA-SA-019, as ADR 0064 did on the 0.3 line. Each entry carries an HMAC-SHA-256 tag under a per-user key outside the package, and an entry without a valid tag is solved again; the proof cache key, not the cache, is in the Trusted zone (`spec/12 §9`, `spec/13 §6`, which adds `tcb.proof_cache_key` beside `tcb.proof_cache`). Rule 2 is unchanged. The authentication that rule 3 deferred is this change.
