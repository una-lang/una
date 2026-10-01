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
