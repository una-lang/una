# 0054. Solve again with the arithmetic interpreted

- Status: Accepted
- Date: 2026-10-01
- Deciders: the coordinator session, on Leonardo's behalf (issue #104, moved into v0.1.2 on 2026-10-01 as a fix of ADR 0053's stated consequence), on a proposal by Claude (closure); the zero-divisor rule and "never proven" are the proposal's choices, for the review
- Resolves: issue #104 (v0.1.2 review finding V12-001)
- Amends: ADR 0053 rules 1 and 3, and `spec/12 §6`

## Context

ADR 0053 checks the model of a satisfiable query through uninterpreted arithmetic against real arithmetic, and reports `unknown` when the model disagrees. Its consequences promise that "real counterexamples through products keep their status". They did not: the solver is free to pick a disagreeing model even when an agreeing one exists, and it did so in several cases:
- `a * a` as `Pos`, which fails at `a = 0`;
- `a * b < 4` on `1..2`, which fails at `2, 2`;
- `7 / -2 == 3`;
- the first review's `negarith.e`.

It also counted a zero divisor as disagreeing, even in an arm the model never takes, which turned `rv-smt-internal-names` from refuted into unknown.

## Decision

1. **A second query.** When the first model disagrees, the verifier solves the same query once more, in `QF_UFNIA`, with `i!mul!Int`, `i!div!Int` and `i!mod!Int` defined as `*`, `div` and `mod`, the Euclidean operations of SMT-LIB. It uses the same seed and step budget, and caches the answer like any other.
   - If that query is refuted and its model agrees, the obligation is `refuted` (U5001), with that model's counterexample.
   - Otherwise it is `unknown` (U5002), as ADR 0053 states. Even when the second query is unsatisfiable, the obligation stays `unknown`, never `proven`: this release accepts no program that v0.1.1 rejected.
2. **Zero divisors.** By a zero divisor, any value agrees. An execution that reaches the division fails there, and that division's own obligation reports it.
3. `spec/12 §6` states both rules.

## Consequences

- The cases above, and `rv-smt-internal-names`, are refuted again, with real counterexamples.
- Refutations that no execution has stay `unknown`: a product of two `Nat`s that is negative, or `a * a < 0`.
- At most one extra solver call is made per disagreeing refutation, and the result is deterministic.

## Alternatives considered

- **Pin each disagreeing application to its real value at the model's point, and solve again a bounded number of times.** Not chosen: the solver can pick a new failing point every round, so `a * a` as `Pos` may never converge. Defining the operations covers every point at once.
- **Report `proven` when the second query is unsatisfiable.** Not chosen for a patch release: it would accept programs that v0.1.1 rejects.

## Amendment of 2026-10-08: a counterexample that divides by zero

- Deciders: the coordinator session, on Leonardo's behalf, under the delegation of 2026-10-06 (decision N-61, issue #399), on finding V-1 of the deferred review of this ADR (`docs/reviews/v0.3.3/repairs-and-0054.md`, #124).

**Context.** Rule 2 lets any value agree at a zero divisor, which keeps the obligation sound: a run that reaches the division fails there, and that division's own obligation reports it. The model is no run, though, and U5001 showed it as one.
- `divided(a, b) = a / b` with `ensures result == 7` was refuted with the counterexample `a = -1, b = 0, result = 0`.
- Its help said to add a guard so that this path returns 7, a path that cannot run.

**Decision.** A refuted obligation keeps its status, but when its model sets to zero the divisor of a division that its own path evaluates, the obligation shows no counterexample and no repair of its own. U5001 instead names that division as a related location: "the refuting path divides by zero here, so it is no run: fix this division, whose own obligation reports it".
- Only a division by a value that is not a literal counts, since it is one with a division obligation.
- The division must be evaluated on the obligation's path, before the obligation's site: in Core IR's A-normal form, the `let` of the division is in the obligation's context.
- For a return obligation, the path is the one the model takes.
- A division on another path does not count. In `if b == 0 then 5 else a / b`, a model with `b = 0` takes the other arm, and keeps its counterexample.

**Consequences.**
- The divisors are read from the model's values of the operands that ADR 0053 already requests. The query, its text and the proof cache key are unchanged.
- The status of every obligation is unchanged. Only the counterexample and the repairs of these diagnostics change.
- The cost is a scan of the divisions on the model's path for each refuted obligation.

Note (2026-10-16): on a query with an `F64` term, whose logic is `ALL`, the interpreted re-solve leaves the logic line as it is and still replaces the declarations of `i!mul!Int`, `i!div!Int` and `i!mod!Int` by their definitions; it runs as on any query (ADR 0114 rule 8.2).
