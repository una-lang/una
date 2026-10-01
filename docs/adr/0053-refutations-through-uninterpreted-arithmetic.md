# 0053. Refutations through uninterpreted arithmetic

- Status: Accepted
- Date: 2026-10-01
- Deciders: Leonardo (accepted on 2026-10-01), on a proposal by Claude (closure)
- Resolves: issue #72 (UI-004), the open part; final review note F-008
- Amends: `spec/12 §6` (statuses) and `§7` (counterexamples)

## Context

The encoding leaves a product, quotient or remainder of two values uninterpreted (`spec/12 §2`, ADR 0033). The solver may then satisfy the negated goal with a model in which `a * b` has a value that no execution gives. One example is `product(a: Nat, b: Nat) -> Nat = a * b`: the model has `a = 0`, `b = 38`, `result = -1`.

`spec/12 §6` makes every satisfiable query `refuted`, which is U5001 with a counterexample. Since v0.1.0, the message says "is refuted in a model where a product, quotient or remainder of two values is uninterpreted, so the counterexample may not be a real execution". But the status is still `refuted`, and the JSON `counterexample` is shaped like any other. An agent reading the ABI takes it as an execution.

Rejection is right either way: an obligation the verifier cannot prove is an error in v0.1 (charter P8). The question is only what the diagnostic claims. The repairs are already valid:
- a guard on the same product makes the obligation provable;
- `checked` works within the escape budget;
- the invalid `requires result >= 0` was removed (R-021).

## Decision

1. **Validate the model.** When the query of a satisfiable obligation uses `i!mul!Int`, `i!div!Int` or `i!mod!Int`, the verifier evaluates each application of those symbols in the model with real arithmetic, using Euclidean division and remainder (`spec/11`). The evaluation is exact and deterministic. A model that cannot be checked counts as disagreeing:
   - a value beyond 128 bits, or an overflow;
   - a zero divisor;
   - such an application inside a reflect predicate's definition.
2. **If every application agrees**, the model is an execution of the encoding. The obligation is `refuted` (U5001), with the counterexample and the plain message.
3. **If any application disagrees**, the obligation is `unknown` (U5002). Its reason is "the refutation depends on a product, quotient or remainder of two values, which the encoding of `spec/12 §2` leaves uninterpreted". It points at the same expression as a refutation would. It has no counterexample, and keeps the repairs of its kind. It is still an error in v0.1.
4. `spec/12 §6` and `§7` state the rule, and the diagnostics ABI does not change.

## Consequences

- No diagnostic presents an impossible model as a counterexample, and real counterexamples through products keep their status.
- Programs are rejected exactly as before: `unknown` is an error in v0.1. The build report's obligation counts move from refuted to unknown where a model is rejected.
- The verifier needs the model's interpretation of the three uninterpreted symbols, which Z3 returns with the model, and an evaluator for them. Cost: one evaluation per such refutation.

## Alternatives considered

- **B. Always `unknown` when the query uses uninterpreted arithmetic and is satisfiable.** Simpler, but a real counterexample through a product is lost, which costs agents the most useful evidence.
- **C. Keep `refuted`, and mark the counterexample in the ABI (`"exact": false`).** This changes `una.diagnostic/1`, whose schema versions D-148 froze at v0.1.0.
- **D. Keep v0.1.0's behavior,** where the message carries the caveat. Not chosen: the status and the JSON still present the model as an execution.
