# 0037. Refinements in function types and their obligations

- Status: Accepted
- Date: 2026-09-29
- Deciders: Owner (accepted on 2026-09-29), on a proposal by Owner (fix request F-0001-02)

## Context

`spec/03 §6` makes function types contravariant in their parameters and covariant in their result, and `spec/10 §2` states it as rule S-Fun, whose premises are `Γ ⊢ τ̄2 <: τ̄1` for the parameters and `Γ ⊢ τ_r <: τ_r'` for the result. For refined types, `{v: B | p} <: {v: B | q}` is the obligation `p implies q` (`spec/03 §6`), a `refinement` obligation at a subsumption point (`spec/12 §3`). The spec does not say what that obligation looks like when the refined types sit inside function types: which binder stands for the parameter or the result, what the goal text is, what the counterexample binds (`spec/12 §7`), or which repairs of `spec/13 §3` apply.

The reference compiler does not implement these premises. It drops every refinement inside a function type before Core IR (a known gap of `docs/history/v0.1/runs/M8-plan.md` §6):

- the checker lowers `fn(Pos) -> Int` to `fn(Int) -> Int` (`crates/una-check/src/types/env.rs`, `lower_type`), and the type of a named function used as a value keeps only the bases of its parameters and result (`types/expr.rs`);
- a named function used as a value is elaborated to `λ(%t0: Int). f(%t0)` (`types/elab.rs`, `function_value`), so the call inside the lambda yields a `refinement` obligation over the internal binder `%t0`;
- a lambda's parameter annotation keeps only its base: `fn(y: Pos) -> y` is `λ(y: Int). y`.

The result is sound, since no refinement of a function type is ever assumed, but it is incomplete and its diagnostics are wrong. With `pub kind Pos = Int where it > 0` and `takes_pos(x: Pos) -> Int`, the review of the syntax guide (`docs/fixes/0001-syntax-guide-review.md`, F-0001-02) and this investigation found:

1. `apply(takes_pos, x: 0)`, with `apply(f: fn(Int) -> Int, x: Int)`, is rejected, as it should be, but with U5001 "refinement `takes_pos > 0` is refuted": the goal substitutes the function's name for the refined value, the counterexample is empty because `%t0` has no source name, and the repairs propose `checked(value, takes_pos > 0)` and a `requires takes_pos > 0`.
2. `apply(takes_pos, x: 1)`, with `apply(f: fn(Pos) -> Int, x: Pos)`, is rejected with the same U5001, although S-Fun holds (`Pos <: Pos`). A correct program is refused.
3. `apply(fn(y) -> takes_pos(y), x: 1)`, with the same `apply`, is rejected with "refinement `y > 0` is refuted", for the same reason.
4. `apply(fn(y: Pos) -> y, x: 0)`, with `apply(f: fn(Int) -> Int, ...)`, is accepted: the annotation is ignored, so S-Fun's `Int <: Pos` is never checked.
5. In the covariant position, `apply_pos(gives_int, x: 0)`, with `apply_pos(f: fn(Int) -> Pos, x: Int)` and `gives_int(x: Int) -> Int`, gives no obligation at all; and inside `apply_pos`, `f(x)` is not known to be positive, so `apply_pos(...) -> Pos = f(x)` is refuted.

Fixing the diagnostic of case 1 alone would leave cases 2 to 5, and any fix of them needs the obligation's shape, which the spec leaves open. Hence this ADR.

## Decision

Option A: **carry refinements in function types, and give S-Fun's premises named obligations.** The owner accepted it with conditions, which are rules 9 to 12.

1. **Types.** A function type keeps the refinements (and labels) of its parameters and result, from its written form, from the header of a named function used as a value, and from a lambda's annotations. Core's `Type::Fn` already has room for them.
2. **Where.** When a value of type `fn(A1, ..., An) -> R` flows into a position of type `fn(B1, ..., Bn) -> S` (argument, `let` with annotation, constructor field, return), each position whose refinements differ gives one `refinement` obligation at the value's span, under the path context of the flow:
   - parameter `i` (contravariant): hypotheses the refinements of `Bi`, goal the refinements of `Ai`;
   - the result (covariant): hypotheses the refinements of `B1` to `Bn` over their binders and those of `R`, goal the refinements of `S`.
   A goal that is trivially implied (no refinement in the target, or the same refined kind) gives no obligation.
3. **Binders.** The binder of parameter `i` is the parameter's name in the function that flows: the declared name for a named function (`x` for `takes_pos`), the written name for a lambda. The result's binder is `result`. A function-typed parameter or local that flows on has no names of its own; its binders are then the expected type's written names if any, and otherwise `arg1` to `argN`.
4. **Goal text.** The goal is written as the refinement instantiated at the binder, prefixed with the position: `parameter x of takes_pos: x > 0`, or `result of gives_int: result > 0`; a lambda is named `the lambda`. The message stays "refinement `<goal>` is refuted", with the refinement instantiated at the binder (`x > 0`, never `takes_pos > 0`).
5. **Counterexample.** The binders are source names in the encoder's name table, so the counterexample binds them (`x = 0`), as `spec/12 §7` requires of parameters and `result`.
6. **Repairs.** Of `spec/13 §3`, only "strengthen a parameter or field type" applies, and it names the expected function type's position ("strengthen parameter 1 of `fn(Int) -> Int` to a refined kind that implies `x > 0`"). No `requires`, guard or `checked` repair is proposed, since none of them can reach a value that exists only when the function is called.
7. **Calls through a function value.** Calling a value of type `fn(B1, ..., Bn) -> S` proves the refinements of `Bi` at the arguments, as for any call, and assumes the refinements of `S` of the result (T-App with the refined type).
8. `spec/12 §3` gets a row "function value flowing into a function type" in the obligation kinds, with rules 2 to 6; `spec/13 §3` gets rule 6; and `spec/10 §2` notes that S-Fun's refined premises are the obligations of `spec/12 §3`. Rules 2 to 7 add no syntax, keyword or diagnostic code: their obligations report as U5001 and U5002.
9. **Quantifier-free.** Every obligation of rules 2 and 7 stays in QF_UFLIA (`spec/12 §4`): the binders of rule 3 are fresh variables of the query, never bound by a quantifier.
10. **No silent drop.** A refinement in a function type that these rules cannot support, such as a result refinement that depends on a parameter, is rejected with an explicit diagnostic at the type. It is never dropped. Which registered code carries it is settled before the implementation (`docs/fixes/0001-syntax-guide-review.md`, "Needs review").
11. **Internal check.** The compiler checks, as an internal invariant, that every refinement of a function type written or inferred by the checker is present in Core IR; a discarded refinement is an internal compiler error (U8001), never a silent pass.
12. **Conformance.** Cases under `tests/conformance/verification/` cover: a correct program passing a named function with refined parameters where the same refined function type is expected, accepted; a lambda whose annotation is stronger than the expected parameter type, rejected; and a function whose result is less refined than the expected result, rejected with a named obligation and a non-empty counterexample.

The implementation is scheduled for the v0.1 closure run, after the review freeze ends, and the spec changes of rule 8 land in the same change (`CLAUDE.md`, "Changing the spec"). Until then the compiler keeps the behavior described in the context.

## Consequences

- Cases 2 and 3 become accepted, and cases 1, 4 and 5 are rejected with an obligation an agent can act on: a named binder, a counterexample and one applicable repair.
- The checker, the elaborator and the obligation generator change together; the eta-expansion of named functions keeps the parameters' declared names and types.
- Refinements inside type arguments, such as `List[Seats]`, stay untracked (the same known gap); this ADR does not cover them.
- Existing programs that pass today stay accepted, except those like case 4, which rely on an ignored annotation and were wrong.

## Alternatives considered

- **Option B: function types carry no refinements in v0.1.** The spec would say that the refinements of a function type's parameters and result are erased: a function value's type is compared on bases only, calls through it neither prove nor assume refinements, and a named function with refined parameters used as a value is wrapped so that each call proves them inside the wrapper. The obligation of case 1 would still need a binder, a goal and repairs (rules 3 to 6 above, for the parameter case only). Not proposed because it keeps rejecting case 2, a correct program that S-Fun accepts, and because it changes S-Fun and `spec/03 §6` instead of implementing them.
- **Option C: fix only the diagnostic of case 1.** Rename `%t0` after the parameter and reword the goal and repairs, leaving the erasure. Not chosen alone: cases 2 to 5 remain, and the shape of the obligation still has to be defined by the spec. It is a possible first step under option A or B.
- **Option D: reject refinements inside function types with a diagnostic.** It would need a new diagnostic code, which a fix request cannot add, and it removes a feature the spec already has.
