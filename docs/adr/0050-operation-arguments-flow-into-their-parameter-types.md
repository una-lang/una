# 0050. Operation arguments flow into their parameter types

- Status: Accepted
- Date: 2026-10-01
- Deciders: Leonardo (accepted on 2026-10-01), on a proposal by Claude (reviewer)
- Resolves: final review finding F-001 (escalation E1, option A), issue #77
- Amends: `spec/07 §7` rule 4, `spec/10 §5` (T-Op) and ADR 0046 rule 3
- Amended by: ADR 0052 (rule 2: the sink of an operation argument bounds only the labels at its covariant positions, superseding D-164), on 2026-10-01

## Context

`spec/07 §7` rule 4 made an operation's arguments sinks bounded by `accepts`, and said that the labels of the operation's parameter types do not bound them, so that `Files.write`, which accepts `internal` content declared `Text`, can receive internal data. That holds for data the caller sends to the handler, but a handler body assumes the declared parameter type everywhere. At a contravariant position, as in `op apply(f: fn(secret Int) -> Int)`, and at an invariant kind argument, as in `op open(s: Sink[secret Int])` (ADR 0046), the handler may pass `secret` data into the argument. A caller that passed `fn(x: Int) -> x`, or a `Sink[Int]` built from it, then received the secret back as a `public` `Int`: `una check` accepted the program and `una run` printed the PIN (F-001). Handler instance arguments already had the full flow check; only operation arguments were affected.

## Decision

1. **Flow.** An operation argument flows (`spec/07 §7` rule 2) into the operation's declared parameter type, with every covariant position of that type taken as `⊤` for the comparison. A position is covariant when the caller sends data there to the handler: the top of the type, list elements, the arguments of covariant kind parameters (including those of `Option` and `Result`), and function results; the parameters of a function type flip the direction, so the parameter of a function parameter is covariant again. Contravariant positions, and every position of an argument of an invariant kind parameter, are compared as at any other call argument. Otherwise it is U4001 at the argument.
2. **Sinks unchanged.** `accepts` keeps bounding the data at the covariant positions: `pc ⊔ ℓ(arg) ⊑ accepts`, with every label of the argument's type (U4003, D-164). A program that is sound today and passes only data stays accepted, and `Files.write` is unchanged.
3. **Sites.** The rule applies wherever an operation is called: function bodies, handler operation bodies, and `do` lines of examples, whatever binds the capability (`uses`, `with` or `given`).
4. **Spec.** `spec/07 §7` rule 4 states the rule, and `spec/10 §5` T-Op compares the arguments with the parameter types so transformed.

## Consequences

- `op apply(f: fn(secret Int) -> Int)` rejects `fn(Int) -> Int`, and `op open(s: Sink[secret Int])` rejects `Sink[Int]`, at every site; `List`, `Option`, `Result` and covariant kinds of such functions are rejected too.
- Data positions are unchanged: a `secret Int` passed to an operation that accepts `secret` and declares `Int` is accepted, and one passed to an operation that accepts `public` is U4003, as before.
- The closure plan's flow and variance matrices gain an `op argument` row (`docs/history/v0.1/runs/v0.1-rereview-plan.md` §3).

## Alternatives considered

- **Reject operation parameter types that contain a function type or an invariant kind argument (E1 B).** Not chosen: it forbids callbacks through capabilities, which the spec allows.
- **The full flow into the declared type, plus `accepts` (E1 C).** Not chosen: it breaks rule 4's own `Files.write` case.
