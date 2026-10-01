# 0052. An operation argument's sink bounds only its covariant positions

- Status: Accepted
- Date: 2026-10-01
- Deciders: Leonardo (accepted on 2026-10-01), on a proposal by Claude (closure)
- Resolves: issue #84 (round-3 review finding R3-001)
- Amends: ADR 0050 rule 2 and `spec/07 §7` rule 4; supersedes D-164

## Context

`spec/07 §7` rule 4 bounds an operation argument by `accepts` "with every label of the argument's type" (U4003). D-164 chose to count the labels of function parameters too, as a conservative choice. ADR 0050 then added a flow from each argument into the declared parameter type:
- its covariant positions are taken as `⊤`, since `accepts` bounds them;
- its contravariant positions, and every position of an invariant kind argument, are compared as at any call argument.

Together, the two rules leave no type to pass when an operation's parameter is a function whose parameter carries a label. R3-001 shows this with `op apply(f: fn(internal[Location] Int) -> Int)` under the default `accepts public`:
- a function with a lower or different parameter label is U4001 (ADR 0050), which is right;
- the function with the exact type is U4003, because D-164 counts its `internal[Location]` parameter label as data sent to a `public` sink.

The parameter of a function argument carries data the other way: from the handler, which calls the function, to the caller's function. That function is pure (`spec/05 §6`), and its result returns to the handler. So the handler reveals nothing to the caller through it, and ADR 0050's flow already keeps its parameter label from being lowered. The rejection is sound, but no program can use such an operation.

## Decision

1. **The sink of an operation argument bounds the labels at its covariant positions**, the data the caller sends to the handler: `pc ⊔ ℓ⁺(arg) ⊑ accepts` (U4003). `ℓ⁺` joins these labels:
   - the top label;
   - list elements;
   - the arguments of covariant kind parameters, with the labels their kinds' fields declare;
   - function results;
   - the parameters of function parameters, since they are covariant again.

   Every label of an argument of an invariant kind parameter counts, as before, since data may flow both ways there.
2. **Contravariant positions** are not part of the sink. ADR 0050's flow into the declared parameter type compares them (U4001).
3. **Spec text.** `spec/07 §7` rule 4 states this in place of "every label of the argument's type". ADR 0050 rule 2 gets an "Amended by" note, and rule 1 supersedes D-164.

## Consequences

- R3-001's exact function is accepted, and so is a function whose parameter label is higher than declared (`secret[Location]` where `internal[Location]` is declared).
- These stay rejected:
  - a lower or different parameter label: U4001;
  - a labeled result: U4003;
  - the parameter of a function parameter given secret data by the caller: U4003;
  - an invariant kind argument above `accepts`: U4003;
  - every leaking form of F-001: U4001.
- One existing conformance expectation changes: `sweep_label_fnvalue_sink.una`, `parameter`. A `fn(x: secret Int) -> 1` passed where `fn(Int) -> Int` is declared under `accepts public` is accepted. The handler can pass it only public data.
- A prototype passes every other conformance case unchanged.

## Alternatives considered

- **B. Keep D-164, and accept R3-001 as a limitation.** Not chosen: such an operation can never be called, and the conservatism protects nothing, since ADR 0050 already compares those positions.
- **C. Count no function labels at all (D-164 option B).** Not chosen: a function's result, and the parameters of its function parameters, are data the caller sends to the handler, so dropping them would leak.
