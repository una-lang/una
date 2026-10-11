# 0092. Conditions on allowed effects

- Status: Proposed
- Date: 2026-10-11
- Deciders: Una project (pending acceptance)
- Resolves: issue #520, part A ("when an allowed operation runs"). Parts B and C are ADR 0093 and ADR 0094.
- Target: edition `0.3`. Editions `0.1` and `0.2` keep today's rules, and the Gate 1 pin (v0.1.9, edition `0.1`) does not move.
- Amends, once accepted: `spec/02 §3`, `§4`, `spec/05 §3`, `§9`, `spec/12 §3`, `spec/13 §3`

## Context

### The evidence (shared with ADR 0093 and ADR 0094)

Gate 1's Q1 analysis (una-bench `findings/for-v0.2.md` §1, `docs/haiku-b-violations-in-una.md`) compared the security errors Claude Haiku 4.5 made in TypeScript on Suite B with what Una does with the same errors:
- 5 of the 13 tasks stop at build time (U4001, U4003);
- 1 stops at run time (B13, the host grant);
- the other 6 are accepted.

Suite B's full run (una-bench `docs/full-b.md`) adds B07, and B09 is a related case. Each is accepted because the checker cannot express what the task forbids, not because a check is wrong:

| Task | What the violation does | Why Una accepts it today | Where it is answered |
| --- | --- | --- | --- |
| B04 | retries a timed-out refund without asking `is_refunded` first | `refund` is in the function's `uses`; nothing says when | not covered here: #558 (state about the external system) |
| B05 | mails a welcome to an address that left later in the same batch | `send` is in `uses`; the forbidden case depends on data the body computes | not covered here: #558 |
| B06 | journals a lookup failure during a dry run | `journal` is an operation of the one `Catalog` binding; nothing ties it to `dry_run` | this ADR |
| B10 | hands the user's read-write files to a vendored library | the library inherits `@files` by name | ADR 0093 |
| B11 | hands the HTTP client to a plug-in's alert hook | a private helper's `uses` are inferred from its callers | ADR 0093 |
| B12 | hands the writable record store to a third-party rule pack | inheritance by name, and the rank-1 repair widens a header | ADR 0093 |
| B07 | reads an archived document through the service's storage binding | the template read and the archive read are the same operation of one binding | not covered: #560 (a deputy chosen by the key read) |
| B09 | looks up a file's owner through the service's binding for a requester who cannot see it | no operation says on whose behalf it answers | ADR 0094 |

These are the variants' outcomes, not Una agent runs. The full run measures what Una agents do, and there Una's remaining violations fall in exactly these rows.

### What a capability says today

A capability says which operations a function may perform (`spec/05 §3`). B06's reference (`pricing.update`) declares `uses @catalog: Catalog[price, set_price, journal]`. That lets the body, and every callee it passes `@catalog` to, journal at any point.
- An operation's `requires` speaks of the operation's arguments only (`spec/05 §2`).
- An example can say `expect not performed @catalog.journal` (`spec/08 §2`), but that tests one run. It is not a contract of the function.
- One binding can carry no condition for only some of its operations: B06's dry run needs `price` and forbids `set_price` and `journal`. A second `uses @catalog` clause is U1002 (`spec/05 §9` rule 8).

### Constraints

- **Quantifier-free verification.** Every query is in QF_UFLIA (`spec/12 §2`). A rule over the whole trace needs a quantifier over its events, unless it is checked event by event.
- **Public headers are the review boundary** (ADR 0007). What a function promises about its effects belongs in its header, where every reader of the code sees it.
- **One effect per line** (`spec/04 §4`). Every effect is a statement of its own, so every effect site has a path condition.
- **Syntax.** Una has its own syntax family, a foundation rule of the language. A new form extends one Una already has, and an agent meets as few new forms as possible: in the full run, 75% of the friction was syntax.

## Decision

1. **A condition on an operation of a binding.** In edition `0.3`, an operation listed in the `[ops]` of a `uses` clause MAY carry `where` and a condition:

   ```una
   pub update_prices(changes: List[PriceChange], dry_run: Bool) -> PriceReport
     uses @catalog: Catalog[price, set_price where not dry_run, journal where not dry_run]
     fails UpdateError
   ```

   - The condition is a `line_expr` of the refinement language (`spec/04 §2`) over the function's parameters, as in a `requires` clause. `result` is not in scope (U2012). It MUST be a `Bool` (U2001).
   - An operation without `where` is unconditioned, as today.
   - `where` is a reserved keyword (`spec/01 §5`), already the word that bounds a refinement (`Int where it >= 0`, `spec/03`). It introduces no new keyword. Unlike a suffix `if`, it cannot be read as a handling clause (`if error pass`, `if none fail`).
   - Next to the forms it extends:

     | Today | Edition `0.3` |
     | --- | --- |
     | `uses @catalog: Catalog[price, set_price, journal]` (`spec/05 §3`) | `uses @catalog: Catalog[price, set_price where not dry_run, journal where not dry_run]` |
     | `kind Seats = Int where 0 <= it and it <= 5000` (`spec/03`): `where` bounds a value | `journal where not dry_run`: `where` bounds when the operation may run |
     | `requires changes.len > 0` (`spec/04 §2`) | the condition is in the language of `requires` |
     | `expect not performed @catalog.journal` (`spec/08 §2`): a test of one run | a contract of every run |

   - The same `where` is allowed in the `uses` of a handler (over the handler's parameters) and of a function type (over its parameters).

2. **Meaning.** Every performance of a conditioned operation through the binding happens only where the condition holds of the function's arguments. Parameters are immutable, so the condition's value is fixed for the whole call.

3. **Obligations.** One obligation of a new kind, `effect_condition` (`spec/12 §3`), at each site below. Its goal is ground and its path context Π is the site's:

   | Site | Goal |
   | --- | --- |
   | an operation `@b.op(...)` in the body, `op` conditioned by `C` on `@b` | `C` |
   | a call that passes the binding `@b`, by name or with `with @x = @b` or `@b[ops]` (ADR 0093), to a callee whose binding `@x` lists `op` with condition `C′` (none: `C′ = true`), where the caller conditions `op` on `@b` by `C` | `C′[args] ⇒ C`, with `C′` instantiated at the call's arguments |
   | a `with @x = h(args)` that installs a handler whose own `uses` list the binding `@b`'s conditioned `op` with `C′` (none: `true`) | `C′[args] ⇒ C`, charged at the `with` |

   - **Implication at calls.** A callee that guards internally says so in its header, and its callers prove only that wherever the callee's condition holds, theirs does too. Soundness:
     - the callee proves that each of its performances happens where `C′` holds of its arguments;
     - the caller proves `Π ∧ C′[args] ⇒ C`;
     - so each performance happens where `C` holds.
   - **Per binding passed.** The obligation arises only for the binding the caller passes and the callee's binding it becomes. It does not arise for every callee whose effect set has the capability: a callee that uses another binding of the same capability gets none.
   - **Handlers** are charged where they are installed (the reading #538 gives `with` scopes). A `with` that installs a handler using a conditioned operation is the site, under the `with`'s Π.
   - **Lambdas** are charged where they are defined. An operation in the body of a lambda is a site of the enclosing function, under the path context of the lambda's definition, its own parameters unconstrained. This is sound because a lambda with effects is second-class: it does not escape (`spec/05 §9` rule 6), and the enclosing function's parameters are immutable. A call of a function value is not a site.
   - Each obligation is QF_UFLIA. There is one query per site of a conditioned operation, linear in the effect sites and the call edges that pass a conditioned binding. Code that writes no condition gets none.
   - The diagnostic is U5001 (refuted) or U5002 (unknown), as for every obligation, so no new code is added.
     - Its message is "`@catalog.journal` may run where `not dry_run` is not proven".
     - The primary span is the site, and the related span is the `where`.
     - The counterexample gives the arguments (`dry_run = true`), as for any obligation (`spec/12 §7`).

4. **No inference, no defaults.** A condition is never inferred:
   - a private helper with inferred `uses` (`spec/05 §9` rule 2) has `C′ = true` for every operation, so a call of it must be under `C`;
   - no name has a meaning: a parameter named `dry_run` gets no condition by convention.
   
   A convention would be magic. It would hold in one package and not the next, change a program's meaning when a parameter is renamed, and be invisible in the header. What a function promises is written in its header.

5. **Repairs** (`spec/13 §3`, ADR 0084). For an `effect_condition` diagnostic, in this order:
   - **at a call or a `with`:**
     1. write the condition in the callee's (or handler's) `uses`, `op where C″`, where `C″` is `C` with each of the caller's parameters replaced by the callee's parameter it is passed to. This is mechanical when every parameter `C` names is passed unchanged as an argument, so `una fix` may apply it: it adds a promise to the callee, never authority.
     2. guard the call with `if C`.
   - **at an operation:**
     1. guard it with `if C` (mechanical);
     2. strengthen the function's `requires` with `C`.
   - **never:** remove or weaken a condition. That is an authority change, so it is not offered as a repair, and it shows in the header's diff.

6. **Who writes the conditions.** A condition protects only where someone writes it:
   - **the author of an API or a specification**, in the header of the public function the rule governs, as with `requires` and `ensures`;
   - **a scaffold**, in benchmark-like settings: the task's entry points carry the conditions the task states, and the body is the task;
   - **an agent editing a function with a condition** sees it in the header. An unguarded site it adds gets U5001 with the counterexample, and the rank-1 repair: a guard, or the condition on the helper that guards. Removing the condition from a public header is an authority change that shows in the diff.

7. **Formal model.** The theorem this ADR needs, *trace safety*, is an open obligation and a separate milestone of the formal track. It states: in every run of a checked function, every performance of a conditioned operation happens where its condition holds of the arguments of the function that declares it. Until the theorem lands, this ADR is "accepted, not proved", the formal-track status page lists it, and the re-check reads a conditioned operation as not re-checked.

## B06's reference under this ADR

B06's reference (una-bench `tasks/B06-dry-run-prices/una/reference/src/pricing/update.una`) builds with three header changes and no change to a body:

| Function | `uses` in edition `0.3` |
| --- | --- |
| `update_prices` (the entry point, the scaffold's) | `@catalog: Catalog[price, set_price where not dry_run, journal where not dry_run]` |
| `report_from(changes, index, report, dry_run)` | `@catalog: Catalog[price, journal where not dry_run]` |
| `stop_lookup(sku, reason, dry_run)` | `@catalog: Catalog[journal where not dry_run]` |
| `apply_from`, `set_back` | unchanged: `Catalog[set_price, journal]`, `Catalog[set_price]` |

The obligations, site by site:

- `update_prices` calls `report_from(changes, index: 0, report: empty_report(), dry_run)`, which passes `@catalog`:
  - `price` is unconditioned on both sides, so there is no obligation;
  - `journal`: `Π ∧ (not dry_run) ⇒ not dry_run`, proven.
- `update_prices` calls `apply_from(report.changed, index: 0)` in the `else` of `if dry_run`. So Π holds `not dry_run`, and `apply_from` lists `set_price` and `journal` without condition: `Π ∧ true ⇒ not dry_run`, proven, for both operations.
- `update_prices` performs `@catalog.journal(summary_entry(...))` in the same `else`: `not dry_run` is in Π, proven.
- `report_from` calls `stop_lookup(change.sku, reason, dry_run)`: `Π ∧ (not dry_run) ⇒ not dry_run`, proven, at its own condition. It calls itself with `dry_run` unchanged: proven the same way.
- `stop_lookup` performs `@catalog.journal(...)` in the `else` of `if dry_run`: proven.
- `apply_from` and `set_back` have no conditioned operation, so there is no obligation inside them.

The violating variant journals the lookup failure whatever `dry_run` is, as the scaffold's note invites. Wherever it drops the guard, one obligation is refuted with `dry_run = true`:
- in `stop_lookup`, if its header keeps the condition;
- in `report_from`'s call, if `stop_lookup`'s header drops it;
- in `update_prices`'s call, if `report_from`'s drops it too.

The entry point's header is the scaffold's, so the chain always ends at a refuted obligation.

## Consequences

- **B06 fails at build and its secure reference builds**, as above.
- **B04 and B05 are not covered.**
  - B04's rule depends on what `is_refunded` answered earlier in the run.
  - B05's rule depends on which addresses left later in the batch.
  
  Neither is a condition over the function's parameters. They need state about the external system, which #558 designs. This ADR says so instead of stretching its form to them.
- **Agent cost.**
  - One new form: `where` after an operation in a `uses` line's brackets. It uses a keyword the language already has, in the language of `requires`.
  - No new diagnostic code: an agent meets U5001 or U5002.
  - The rank-1 repairs are a guard, or the callee's header condition. Both are mechanical in the common case, and neither adds authority.
- **Cost for existing code: none.** A `uses` clause without `where` means what it means today, and edition `0.1` and `0.2` packages do not change.
- **Verification cost:** one QF_UFLIA query per site of a conditioned operation, cached like every obligation (`spec/12 §9`).
- **Deferred:** state conditions (#558), and the trace-safety theorem (rule 7).

## Alternatives considered

- **A condition on the whole binding** (`uses @catalog: Catalog[...] if not dry_run`, the first draft). Rejected:
  - it cannot express B06, whose one binding has an operation the dry run needs (`price`) beside two it forbids;
  - a second `uses` of the binding is U1002;
  - a suffix `if` reads like the handling clauses `if error pass` and `if none fail`.
- **The callee's condition as its precondition** (the first draft's rule 4). Rejected: a caller would have to prove the callee's condition outright. That refutes every call of a helper that guards internally, which is B06's reference (`stop_lookup`). Implication (rule 3) is the sound and complete reading.
- **Charging a call of a function value** (the first draft). Rejected: a higher-order callee does not know the condition, so a call inside it cannot prove it. Definition-site charging (rule 3) is sound for second-class lambdas, and charging both would refute `let f = fn ...; if not dry_run then f(x)` at the definition.
- **Trace postconditions** (`ensures not performed @catalog.journal ...`). Not chosen:
  - over the whole trace they need a quantifier over events, or an encoding of the trace as a sequence, which leaves QF_UFLIA;
  - checked event by event they are this ADR's condition, written away from the binding they bound.
- **State on capability declarations, for B04 and B05** (typestate). The precise answer for those two, but it needs ghost state, a frame rule and loop invariants. Deferred to its own ADR (#558). A condition may later name such facts.
- **Modes as narrowing** (rebind `@catalog` in the dry-run branch to a handler without `journal`). Possible today with `with`, but it protects only when the author chooses to write it, and it leaves nothing in the header. It stays available, and it is not a contract.
- **A `dry_run` convention or other inferred conditions.** Rejected by rule 4.

## Open obligations

- The trace-safety theorem in F1 (rule 7), a milestone of the formal track.
- The re-check's reading of conditioned operations, after the theorem.
- The state form (B04, B05), a separate ADR (#558).
