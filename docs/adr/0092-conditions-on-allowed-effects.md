# 0092. Conditions on allowed effects

- Status: Proposed
- Date: 2026-10-11
- Deciders: Una project (pending acceptance)
- Resolves: issue #520, part A ("when an allowed operation runs"). Parts B and C are ADR 0093 and ADR 0094.
- Target: edition `0.3`. Editions `0.1` and `0.2` keep today's rules, and the Gate 1 pin (v0.1.9, edition `0.1`) does not move.
- Amends, once accepted: `spec/02 §3`, `spec/05 §3`, `§9`, `spec/12 §3`, `spec/13 §3`

## Context

### The evidence (shared with ADR 0093 and ADR 0094)

Gate 1's Q1 analysis (una-bench `findings/for-v0.2.md` §1, `docs/haiku-b-violations-in-una.md`) set the security errors that Claude Haiku 4.5 made in TypeScript on Suite B against what Una does with the same errors. Una stops 5 of the 13 tasks with a violation at build time (U4001, U4003) and 1 at run time (B13, the host grant). It accepts the other 6. Suite B's full run (una-bench `docs/full-b.md`) adds B07, and B09 is a related case. Each one is accepted because the checker has no way to say what the task forbids, not because a check is wrong:

| Task | What the violation does | Why Una accepts it today | This ADR series |
| --- | --- | --- | --- |
| B04 | retries a timed-out refund without asking `is_refunded` first | `refund` is in the function's `uses`; nothing says when | 0092, deferred form (§ Alternatives) |
| B05 | mails a welcome to an address that left later in the same batch | `send` is in `uses`; the forbidden case depends on data the body computes | 0092, deferred form |
| B06 | journals a lookup failure during a dry run | `append` is in `uses`; nothing ties it to `dry_run` | 0092 |
| B10 | hands the user's read-write storage to a vendored library | the library inherits `@storage` by name | 0093 |
| B11 | hands the HTTP client to a plug-in's alert hook | a private helper's `uses` are inferred from its callers | 0093 |
| B12 | hands the writable record store to a third-party rule pack | inheritance by name, and the rank-1 repair widens a header | 0093 |
| B07 | reads an archived document through the service's own storage binding | both reads use one binding of one capability type | 0094 |
| B09 | looks up a file's owner through the service's binding for a requester who cannot see it | the same: no binding says on whose behalf it acts | 0094 |

These are the variants' outcomes, not Una agent runs. The full run measures what Una agents do; there, Una's remaining violations fall in exactly these rows.

### What a capability says today

A capability says which operations a function may perform (`spec/05 §3`): `uses @journal: Journal[append]` lets the body, and every callee it passes `@journal` to, append at any point. An operation's `requires` speaks of the operation's arguments only (`spec/05 §2`). An example can say `expect not performed @journal.append` (`spec/08 §2`), but that is a test of one run, not a contract of the function.

So B06's rule, "never journal in a dry run", has nowhere to live. The body can write `if not dry_run` around the append, and nothing makes it.

### Constraints

- **Quantifier-free verification.** Every query is in QF_UFLIA (`spec/12 §2`). A rule over the whole trace ("no append anywhere in the run") would need a quantifier over events, unless it is checked event by event.
- **Public headers are the review boundary** (ADR 0007). What a function promises about its effects belongs in its header, where every reader of the code sees it.
- **One effect per line** (`spec/04 §4`). Every effect is a statement of its own, so every effect site is a point of the program with a path condition.
- **Syntax.** Una has its own syntax family, a foundation rule of the language. A new form extends one Una already has, it does not borrow one for familiarity, and an agent meets as few new forms as possible: in the full run, 75% of the friction was syntax.

## Decision

1. **A condition on a capability binding.** In edition `0.3`, a `uses` clause of a function header MAY end with `if` and a condition:

   ```una
   pub update_prices(changes: List[PriceChange], dry_run: Bool) -> Report
     uses @prices: Prices[get, set]
     uses @journal: Journal[append] if not dry_run
     fails PriceError
   ```

   The condition is a `line_expr` of the refinement language (`spec/04 §2`) over the function's parameters, as in a `requires` clause; `result` is not in scope (U2012). It MUST be a `Bool` (U2001).

   Next to the forms it extends:

   | Today | Edition `0.3` |
   | --- | --- |
   | `uses @journal: Journal[append]` (`spec/05 §3`) | `uses @journal: Journal[append] if not dry_run` |
   | `requires changes.len > 0` (`spec/04 §2`) | the condition is the same language as `requires` |
   | `expect not performed @journal.append` (`spec/08 §2`, an example) | `uses @journal: Journal[append] if not dry_run`: a contract of every run, not a test of one |

2. **Meaning.** Every effect through the binding happens only on a path where the condition holds. "Through the binding" means:
   - an operation `@journal.op(...)` in the body;
   - a call that passes `@journal` to a callee, by name or with `with @x = @journal`, when the callee's effect set (`spec/05 §3` rule 3) contains an operation of the binding's capability;
   - a call of a function value whose `uses` contain the binding.

3. **Obligations.** Each such site is one obligation of a new kind, `effect_condition` (`spec/12 §3`):

   | Kind | Generated at | Goal |
   | --- | --- | --- |
   | `effect_condition` | each effect site of a binding whose `uses` clause has a condition (rule 2) | the condition, instantiated at the function's parameters, under the site's path context Π |

   - Each obligation belongs to one site, and its goal is ground. So the query stays in QF_UFLIA, and the cost is one query per effect site of a conditioned binding: linear in the effect sites, and none for code that writes no condition.
   - The diagnostic is U5001 (refuted) or U5002 (unknown), as for every obligation. Its message is "the effect `@journal.append` happens where `not dry_run` is not proven". The primary span is the effect site and the related span is the `uses` clause. The counterexample gives the parameters (`dry_run = true`) as for any obligation (`spec/12 §7`).
   - No new diagnostic code is added.

4. **Callees.** A callee that receives the binding needs no condition of its own. The obligation is at the call, in the caller, under the caller's path. A private helper with inferred `uses` (`spec/05 §9` rule 2) is covered the same way: its effects count at each call of it. A callee that writes the same condition in its own header proves it at its own sites, and its callers prove it at the call as a `precondition`: the condition becomes part of the callee's `requires` for that binding.

5. **Handlers and lambdas.** A handler's `uses` and a function type's `uses` MAY carry a condition with the same meaning, over the handler's parameters or the function type's parameters. A lambda given where such a type is expected proves it at its effect sites.

6. **Repairs** (`spec/13 §3`, ADR 0084). For an `effect_condition` diagnostic, in this order:
   1. guard the effect with the condition: `if not dry_run` around the statement (rank 1, mechanical, so `una fix` may apply it);
   2. strengthen the function's own `requires` with the condition (rank 2, not mechanical: it changes the header);
   3. never: remove or weaken the condition on the `uses` clause. That is an authority change, so it is not offered as a repair. It shows in the header's diff.

7. **Who writes the conditions.** A condition protects only where someone writes it:
   - **the author of an API or a specification**, in the header of the public function that the rule governs, as with `requires` and `ensures`;
   - **a scaffold**, in benchmark-like settings: the task's entry points carry the conditions the task states, and the body is the task;
   - **an agent editing a function with a condition** sees it in the header and gets U5001 at the effect site it adds without the guard, with the counterexample and the rank-1 guard. Removing the condition is a header change of a public function, an authority change that shows in the diff.

8. **No inference and no defaults.** A condition is never inferred, and no name has a meaning: a parameter named `dry_run` gets no condition by convention. A convention would be magic. It would hold in one package and not in the next, it would change the meaning of a program when a parameter is renamed, and an agent could not see it in the header. The rule stays: what a function promises is written in its header.

9. **Formal model.** The theorem this ADR needs, *trace safety*, is an open obligation and a separate milestone of the formal track. It states that in every run of a checked function, every effect through a conditioned binding happens in a state where the condition holds of the function's arguments. Until it lands, this ADR is "accepted, not proved", and the formal-track status page lists it. The re-check reads a conditioned `uses` clause as not re-checked until then.

## Consequences

- **B06 is stopped by construction** once its scaffold writes `uses @journal: Journal[append] if not dry_run`: the violating variant's append is U5001 with `dry_run = true` as the counterexample. Its secure variant guards the append and builds.
- **B04 and B05 are not covered.** B04's rule depends on what `is_refunded` answered earlier in the run, and B05's on which addresses left later in the batch. Neither is a condition over the function's parameters. They need state about the external system, which is the deferred form below. This ADR says so instead of stretching its form to them.
- **Agent cost.**
  - One new form: a suffix on a `uses` line, in the language of `requires`.
  - No new code: an agent meets U5001 or U5002, already in every guide.
  - One repair at rank 1: a guard, which `una fix` can apply.
  - The full run's friction was mostly syntax, and this form adds no new keyword. `if` already starts a line expression.
- **Cost for existing code: none.** A `uses` clause without a condition means what it means today. Edition `0.1` and `0.2` packages do not change.
- **Verification cost:** one QF_UFLIA query per effect site of a conditioned binding, cached like every obligation (`spec/12 §9`).
- **Deferred:** state conditions (B04, B05), and the trace-safety theorem (rule 9).

## Alternatives considered

- **Trace postconditions** (`ensures not performed @journal.append if dry_run`, the form of the outline). Not chosen as the base: an `ensures` over the whole trace needs a quantifier over its events, or an encoding of the trace as a sequence, which leaves QF_UFLIA (`spec/12 §2`). Checked event by event, it is this ADR's condition, written in a place that hides which binding it bounds. The `uses` line is where the authority is granted, so the condition sits on the grant.
- **State on capability declarations, for B04 and B05** (typestate). A capability declares abstract facts about the external system, such as `refunded(charge)`, that its operations' `ensures` establish and its failures make unknown. An operation's `requires` can then name them: `refund` requires `not refunded(charge)`, known only after `is_refunded` answered `false` and no `Timeout` since. This is the precise answer to B04 and B05, but:
  - it adds ghost state to every capability that uses it;
  - it needs a frame rule for which operations change which facts;
  - it needs facts that hold across loop iterations (B05's batch), which needs invariants.
  
  Deferred to its own ADR. This one leaves room for it: a condition may later name such facts.
- **Modes as narrowing** (rebind `@journal` to an absent or no-op binding in the dry-run branch). This is possible today with `with`. It protects only when the author chooses to write it, the same as a guard, and leaves nothing in the header. It stays available, and it is not a contract.
- **A `dry_run` convention or other inferred conditions.** Rejected by rule 8.

## Open obligations

- The trace-safety theorem in F1 (rule 9), a milestone of the formal track.
- The re-check's reading of conditioned `uses` clauses, after the theorem.
- The state form (B04, B05), a separate ADR.
