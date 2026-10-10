# 0090. A policy call's failure is visible only where its release is

- Status: Accepted (coordinator, 2026-10-10, under the standing security rule, for the owner's review)
- Date: 2026-10-10
- Deciders: the coordinator, for the owner's review; written by the builder session
- Resolves: #496 (S-18)
- Amends: `spec/07 §5` and `spec/10 §7` (T-Declassify), by an appended correction (§13.3 on `main`, §13.2 in the v0.1 line)

## Context

A policy `p : from ⇝ to via f` is called as `p(v)`. Its `via` function `f` is trusted: no label is checked in it (`spec/07 §5` rule 3). A policy has the `uses` and `fails` of its `via` function (rule 1). T-Declassify lets the result carry `to`'s labels, under `pc ⊑ ℓ(to)` (rule 2).

The failure of a policy call is decided inside the `via` function, by the `from` value. The checker recorded the `via` function's error kind for the call, but labeled the failure with nothing:
- `attempt p(v)` gave an unlabeled `Result`;
- `if error pass` passed the failure on as public.

**S-18.** Found by the builder session while checking, before any proof, the statement of #453 Phase B (failing releases). Confirmed on the published v0.3.6 (edition 0.2) and v0.1.8 (edition 0.1) binaries:

```una
pub policy checkpin
  from secret Int
  to internal Int
  via gate

gate(x: secret Int) -> internal Int
  fails Gate
=
  if x > 3 then fail TooBig else x
```

- A public function ran `let r = attempt checkpin(n)` on an `internal` number from the environment, and printed "via passed" or "via failed" by `r`.
- `una check` accepted it, and `una run` printed differently for different values of the number.
- The policy authorizes releasing a function of the secret at `internal`. Its failure, a function of the same secret, reached `public`: a release below the policy's own `to`.

**The audit (#496):** every place where a trusted body's failure surfaces at a label it was not checked against.

| Construct | Checked against labels | Its failure, before this ADR |
| --- | --- | --- |
| A policy call, whose `via` function is unchecked | no (rule 3) | unlabeled: **the leak** |
| `checked` inside a `via` function | no, as all of its body | the `via` function's failure: covered by this ADR |
| A capability handler in a trusted module | yes: `trusted` lifts label checks only inside `via` functions (`spec/07 §5` rule 3) | as for any handler (`spec/07 §7` rule 4) |
| A `via` function used other than through its policy | U4005 (`spec/07 §5` rule 5) | not applicable |

## Decision

1. **The rule.** The failure of a policy call is labeled $pc \sqcup \ell(\mathit{to})$, where $\ell(\mathit{to})$ is the top label of `to`. That is where its release is visible:
   - `attempt p(v)` gives a `Result` with that label;
   - `if error use` runs under it;
   - `if error pass` and `if error fail` are U4002 when it is above `public`, as for any labeled failure (`spec/07 §7` rule 4, and `spec/10 §13.1` on `main`).
2. **A public `to` keeps a public failure.** A policy that releases to `public` already releases everything its `via` function decides, its failure included. `tests/conformance/verification/via_result_checked_opaque`, whose `via` function uses `checked`, stays accepted.
3. **Both editions:** `main` for v0.3.7, and `release/v0.1` for v0.1.9.
4. **The formal track:** #453 Phase B's T-Declassify gives the same label, `pc ⊔ dst.top`, and its release event carries the `via` function's failure (`docs/plan/formal-track-453-phase-b.md`).

**Complexity.** One label join per policy call, O(1).

## Consequences

- A program that branches on, or passes on, the failure of a policy whose `to` is above `public` is now rejected (U4003 or U4002). It handles the failure under the release's label instead, or uses a policy whose `to` is the label it means to release at.
- Programs whose policies cannot fail, or release to `public`, are unchanged.

## Alternatives considered

- **Forbid failing `via` functions.** It breaks the documented pattern in which a `via` function establishes a refinement with `checked` (R-007).
- **Label the failure with `from`'s labels.** That is too strict: a policy exists to release what it decides at `to`, and a `public` `to` would lose its public failure.
