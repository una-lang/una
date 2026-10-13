# 0092. Conditions on allowed effects

- Status: Accepted
- Date: 2026-10-11
- Deciders: Una project (accepted on 2026-10-11)
- Resolves: issue #520, part A ("when an allowed operation runs"). Parts B and C are ADR 0093 and ADR 0094.
- Target: edition `0.3`. Editions `0.1` and `0.2` keep today's rules, and the Gate 1 pin (v0.1.9, edition `0.1`) does not move.
- Implementation: #564 (edition `0.3`), with its theorem #561. The spec amends below land with the implementation.
- Amends: `spec/02 §3`, `§4`, `§8`, `spec/05 §3`, `§9`, `spec/12 §3`, `spec/13 §3`, `§4`, `§7`

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

   - The condition is in the refinement language RL (`spec/03 §3`), with the restrictions of a `requires` clause (`spec/04 §2`), over the function's parameters. It MUST be a `Bool` (U2001). In particular:
     - an expression outside RL is U2014 at the outermost offending sub-expression, as in a `requires` clause. That includes `checked`, which would put an opaque fact in a header outside ADR 0042's place and escape budget, and also `attempt`, `fail`, a lambda, an operation and a call of a function with effects;
     - a call of an opaque predicate is U2016, as in a function's clauses (ADR 0042);
     - `result` is not in scope (U2012), and `it` is not in scope (U2013, rule 1's grammar).
   - An operation without `where` is unconditioned, as today.
   - `where` is a reserved keyword (`spec/01 §5`), already the word that bounds a refinement (`Int where it >= 0`, `spec/03`). It introduces no new keyword. Unlike a suffix `if`, it cannot be read as a handling clause (`if error pass`, `if none fail`).
   - Next to the forms it extends:

     | Today | Edition `0.3` |
     | --- | --- |
     | `uses @catalog: Catalog[price, set_price, journal]` (`spec/05 §3`) | `uses @catalog: Catalog[price, set_price where not dry_run, journal where not dry_run]` |
     | `kind Seats = Int where 0 <= it and it <= 5000` (`spec/03`): `where` bounds a value | `journal where not dry_run`: `where` bounds when the operation may run |
     | `requires changes.len > 0` (`spec/04 §2`) | the condition is in the language of `requires` |
     | `expect not performed @catalog.journal` (`spec/08 §2`): a test of one run | a contract of every run |

   - The same `where` is allowed in the `uses` of a handler, over the handler's parameters.
   - **Not in function types.** A function type's `uses` (`fn_type_clause`, `spec/02 §3`) keep the names-only form. A function type has no named parameters for a condition to speak of (its binders are `arg1` to `argN`, `spec/12 §3`), so `where` there is a parse error (U0101), and so is ADR 0094's `gives`.
     - For the same reason, a named function whose `uses` carry a condition or a label MAY NOT be used as a function value (U3006, the code for a function value that may not flow there): the higher-order callee that would call it could not prove the condition or keep the label.
     - A lambda needs neither: its effects are charged where it is defined (rule 3).
   - **Grammar** (`spec/02 §3`, with ADR 0094's `gives`):

     ```ebnf
     cap_type = upper_ident [ "[" cap_op { "," cap_op } "]" ] ;
     cap_op   = lower_ident [ "gives" label ] [ "where" line_expr ] ;
     ```

     `cap_type` holds no type, only operation names (`spec/02 §3`), and a refined type's `where` appears only in a `kind` alias (`alias_body`). So `where` inside the brackets is always an operation's condition and never a refinement.
     - **Delimiting.** A `cap_op` ends at the first `,` or `]` at bracket and parenthesis depth 0. Every nested `,`, `[ ]` or `( )` belongs to the label or the expression that holds it: a call `ok(a, b)`, a list literal `[1, 2]`, and a label `gives internal[A, B]`. An expression has no top-level comma.
     - **`it` is out of scope** in an operation's condition, which speaks of the function's parameters and has no value to refine. Using it is U2013, the code for `it` outside a refinement, with the message "`it` names no value in an operation's condition". `spec/02 §8`'s row "`it` appears only in `where` clauses and the second argument of `checked`" is amended to "`it` appears only in the `where` of a `kind` and the second argument of `checked`; not in an operation's `where`".
     - **Ceilings and locks.** The ceiling strings of ADR 0093's `[boundaries]` keep the names-only grammar of today's `cap_type`, so `where` or `gives` there is U7002. The `[contracts]` lock of rule 8 is the one place in `una.toml` where `cap_op` is written.

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

8. **Locked contracts of entry points.** A condition protects only while it stays in the header, and an agent can delete it from a public header. So the conditions of public functions can be locked in a place the agent does not edit.
   - **The lock.** `una.toml` gains a table `[contracts]`. Each key is a public function's qualified name, and each value maps its bindings to their locked operations, with ADR 0094's labels:

     ```toml
     [contracts]
     "pricing.update.update_prices" = { catalog = ["set_price where not dry_run", "journal where not dry_run"] }
     "sharing.links.create_share_links" = { service_files = ["role gives internal[Service]", "owner gives internal[Service]"] }
     ```

     Each entry is a `cap_op` (rule 1) over the function's parameters. U7002 at the lock entry reports:
     - a key that names no public function;
     - an operation that the binding's capability does not declare.
     
     An operation that the capability declares but the header leaves out is not an error: it is the stronger case below.
   - **The lock's conditions are checked against today's header.** A lock condition is type-checked against the function's current parameters. It is U7002 at the lock entry when it is not a `Bool` of RL over them, which includes a parameter that has been renamed, removed or retyped. The message names the parameter: "the lock of `update_prices` names `dry_run`, which is not a `Bool` parameter of it". A lock condition is never ignored, and never read as `true`.
   - **The check.** The source header MUST be at least as strong as the lock, for every locked entry:
     - **the operation, or the whole binding, is absent from the header:** stronger, since it grants less, so nothing is checked;
     - **present with a condition `C_src`:** the obligation `C_src ⇒ C_lock`, over the parameters, in QF_UFLIA, as a new obligation kind `contract_lock`. An operation without `where` has `C_src = true`;
     - **present with a label `l_src`** (ADR 0094): `l_lock ⊑ l_src`, a label comparison.
     
     - **a binding the lock does not name:** within a locked function, the lock covers each capability it names, not only each binding. So a binding of that capability that the lock does not name, such as a second `uses @shadow: Catalog[journal]` or the locked binding renamed, is U3023. Locking by capability costs nothing more, and it closes the path of moving an operation to an unlocked binding of the same capability. This holds even for a binding that carries only unlocked operations, such as `uses @audit: Catalog[price]`. That is intentional and conservative: the rule fails closed, and a locked function that needs such a binding names it in the lock.
     
     A header weaker than its lock is U3023: "the header of `update_prices` lets `@catalog.journal` run where `not dry_run` does not hold, which `una.toml` locks". The primary span is the header's `cap_op`, and the related span is the lock entry.
   - **Helpers are free.** Only the listed public functions are locked. Weakening a helper's condition moves the obligation to its callers (rule 3), and strengthening it moves it into its body, so either way the locked entry point's obligation still holds or is refuted.
   - **Protection.** `una.toml` is the consumer's file, like `[trusted]` (`spec/07 §6`) and ADR 0093's `[boundaries]`. A benchmark scaffold marks it protected, so a task's edits never reach it. A repository marks it with its owners' review rules.
   - **Cost.**
     - One `una.toml` entry per locked function.
     - One `contract_lock` query per locked condition, and one label comparison per locked label. Both are cached like every obligation.
     - No cost for a package that writes no `[contracts]`.
     - The repair of U3023 is to restore the locked condition or label (rank 1, mechanical). Changing the lock is never a repair.

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
- **A deleted condition cannot pass unnoticed** where the entry point is locked (rule 8). B06's scaffold locks `update_prices`, so removing its `where` is U3023 before any effect is checked.
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
- **Contract protection by the diff only.** A deleted condition shows in the header's diff, but a diff is reviewed after the fact, and an agent working alone has no reviewer. Rejected in favor of the lock (rule 8).
- **A separate lock file** (`contracts.lock` beside `verification.lock`) instead of a `una.toml` table. Equivalent in protection. `una.toml` is chosen because it already holds the consumer's other protected decisions (`[trusted]`, `[boundaries]`), so one file is protected, not two.
- **Modes as narrowing** (rebind `@catalog` in the dry-run branch to a handler without `journal`). Possible today with `with`, but it protects only when the author chooses to write it, and it leaves nothing in the header. It stays available, and it is not a contract.
- **A `dry_run` convention or other inferred conditions.** Rejected by rule 4.

## Open obligations

- The trace-safety theorem in F1 (rule 7), a milestone of the formal track (#561).
- The re-check's reading of conditioned operations, after the theorem.
- The state form (B04, B05), a separate ADR (#558).
- Assigned on acceptance: U3023, and the obligation kinds `effect_condition` and `contract_lock`. They join `spec/13 §4` and `spec/12 §3` with the implementation (#564).

## Amendment (2026-10-13): a call of a function value is a site

Rule 3 said "A call of a function value is not a site." That leaves a hole. A call of a function value performs the value's effects, and they count for the caller like those of a named call (`spec/05 §9` rule 7), so they run through the caller's bindings. A parameter of function type with `uses` may be the callee (rule 6 of `spec/05 §9`):

```una
pub g(f: fn(Int) -> Unit uses @log: Log[write], dry: Bool) -> Unit
  uses @log: Log[write where not dry]
=
  f(1)

h() -> Unit
  uses @log: Log[write]
=
  g((n) -> @log.write(n), dry: true)
```

As written, no obligation arises:

- the lambda is charged at `h`, which conditions nothing;
- the call `f(1)` in `g` is a call of a function value;
- at `h`'s call of `g`, the caller has no condition.

So `write` runs during `g` with `dry = true`, through `g`'s binding, which rule 2's meaning forbids.

**The rule.** A call of a function value, held by a parameter, a local or a field, is a site of rule 3 for each operation that its type's `uses` list and that resolves to a conditioned binding of the caller. A function type carries no conditions (rule 1), so the callee's condition is `true`, and the goal is the caller's condition `C`, as for a private callee with inferred `uses` (rule 4). Lambdas keep their charge where they are defined: both charges are needed, since a lambda may also be called where it is defined.

- **Stricter only.** The rule adds obligations and removes none, so it only rejects. Code that writes no condition gets none. `g` above is rejected with "`@log.write` may run where `not dry` is not proven" at `f(1)`, and accepted once the call is guarded: `if not dry then f(1)`.
- **A chain.** When `g` passes `f` on to another function `k` that calls it, the call of `k` in `g` is a call that passes the binding, the second row of rule 3: its goal is `C_k[args] ⇒ C`, which is `C` when `k` writes no condition (rule 4). Inside `k`, the call of `f` is a site against `k`'s own conditions.
- **Rebinds.** The binding a reference holds follows `with @x = @b`: under it, the call charges the condition on `@b`. Under `with @x = h(args)`, `@x` holds a handler, which conditions nothing; the `with` itself is the site of the third row.
- **Cost.** One query per such site, linear in the calls of function values whose types use a conditioned binding.
- **Repairs.** A guard, `if C`, around the call. A function type has no conditions, so no condition can move to the callee.
- **The formal model.** The trace-safety theorem (rule 7) must cover this site: a performance under a call of a function value happens where the caller's condition holds. The theorem (#561) and the re-check's reading of conditioned operations (#601) state it with the other sites. Until then the re-check refuses a conditioned operation, so no claim rests on it.

Conformance cases pin the counterexample (rejected), its guarded form (accepted), the chain through `k`, and a call under `with @x = @b`.

## Amendment (2026-10-13, second): policies, `uses of` and functions passed on

An adversarial review of the implementation found three more paths where an operation runs through a conditioned binding with no site. Each one is closed with the same rule: whatever runs through the caller's bindings is charged where the caller hands them over.

- **A policy call.** A policy uses and fails as its `via` function does, so a call of the policy runs the `via` with the caller's bindings. It is a site of the second row of rule 3, as a call of the `via` would be: the goal is `C′[arg] ⇒ C`, with the `via`'s own condition, or `C` when it writes none.
- **A call of a `uses of` parameter** (ADR 0062). Its type has no `uses`, yet its argument's effects run through the function's bindings, any of them. A call of such a parameter is therefore a site for every operation the function conditions, with goal `C`. This is conservative: the function cannot know which bindings the argument uses.
- **A function value passed on.** A value passed to a parameter whose effects run through the caller's bindings, a `uses of` parameter of the callee or a parameter of a standard function such as `map`, is charged at the call as a call of it would be: for each operation its type's `uses` list, the goal is `C`. A `uses of` parameter of the caller passed anywhere is charged every condition. A lambda keeps its charge where it is defined, and is not charged again.
- **`edit` parameters.** A condition reads the arguments as the caller passed them, whatever the body later assigns (rule 2). An `edit` parameter reassigned in the body does not change it: the condition is evaluated at the values on entry, which is what the caller sees and what the guard protects. Reassigning `dry = false` before `@log.write(n)` does not prove `not dry`.

Each rule only adds obligations, so it only rejects, and code that writes no condition gets none. Conformance cases pin a rejected and an accepted form of each.
