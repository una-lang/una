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

## Amendment (2026-10-13, third): a lock entry is a whole `uses` clause

Rule 8's lock maps a binding name to its operations, and the name alone does not say which capability the binding has. A header that renames the locked binding, `uses @shop: Catalog[journal]` for `@catalog`, would leave the lock's binding absent, which rule 8 reads as stronger, and no part of the lock would reach `@shop`. Rule 8 means the lock to cover each capability it names, so the lock must name it.

**The form.** Each entry of `[contracts]` maps a public function's qualified name to an array of `uses` clauses, written as a header writes them, with rule 1's conditions:

```toml
[contracts]
"pricing.update.update_prices" = ["@catalog: Catalog[price, set_price where not dry_run, journal where not dry_run]"]
```

A lock entry lists its operations: a clause without brackets, `@catalog: Catalog`, is U7002 in the lock. A header binding without brackets is checked against the listed operations, so if the capability later declares another operation, the header grants it, the entry does not list it, and it is U3023. The lock freezes the operation set, not only the binding.

**Well-formed locks.** Each of these is U7002 at the lock entry, so a lock never stops checking without an error:

- a key that names no public function of the package, for example because the function was renamed, moved or made private. A handler is not a key: its operations are charged against its own header, and where it is installed, against the conditions of the function that installs it (rule 3), so locking that function covers it;
- an entry that does not parse as a `uses` clause, which includes ADR 0094's `gives` until that ADR is implemented;
- an entry without brackets, which would not list its operations;
- two entries for the same binding of one function;
- a capability name that resolves to no capability in the function's module, for example because the capability was renamed;
- an operation that the capability does not declare;
- a condition that is not a `Bool` of RL over the function's current parameters (rule 8).

**Capability identity.** A capability is the declaration its name resolves to in the function's module, by the rules of name resolution (`spec/10 §11`, ADR 0023). Today a capability reaches a module by its declaration or by an import (`spec/02 §1`), and an import names the same declaration. An alias, a qualified path or a re-export, if the language gains them, names the same capability too.

**The check.** For each binding of a locked function's header:

- **no entry names this binding with this capability:** U3023 at the binding. That covers a second binding beside a locked one, the locked binding renamed, and a binding of a capability that the lock does not name at all, such as a new `uses @net: Http[post]`. A locked function that needs a binding names it in the lock, with its operations;
- **an operation of the binding that the entry does not list** (each operation when the header writes no brackets): U3023 at the operation, since the header grants more than the lock;
- **an operation that the entry lists with a condition `C_lock`:** the obligation `C_src ⇒ C_lock` of rule 8, kind `contract_lock`, with `C_src = true` when the header writes no condition. Refuted, it is U3023 with rule 8's message; unknown, it is U5002. The primary span is the header's operation, and the related span is the lock entry;
- **an operation that the entry lists without a condition:** nothing to check.

An operation or a binding of the lock that the header leaves out is stronger, and nothing is checked, as rule 8 says: removing a locked binding entirely grants less.

**What changes in rule 8.** Rule 8 left an operation that the lock does not list free, and a binding of a capability that the lock does not name unchecked. Both are now U3023: the lock freezes the bindings of a locked function and their operations, and a header may only drop or strengthen them. So a lock lists every operation its function uses, with or without a condition, as the example's `price` shows.

**Entry values.** A lock condition reads the arguments as the caller passed them, as a header's condition does (the second amendment): an `edit` parameter that the body reassigns does not change it.

**Editions.** `[contracts]` is a table of edition `0.3` only. In editions `0.1` and `0.2` it stays the unknown table that it is today (U7002), so those editions do not change.

**Stricter only.** The form adds a capability name and the checks above. Each one only rejects, and a package without `[contracts]` gets none of them.

**Cost.** One pass over the lock and one over the headers of the locked functions, with a map keyed by the resolved function and capability: O(n) on average in the size of the lock and of those headers. One `contract_lock` query per locked condition that the header has. A scaling test pins the linear growth.

Conformance cases pin each check above:
- the second binding, the rename, a binding of a capability the lock does not name, the added operation, and a weakened and a removed condition;
- a capability imported from another module;
- each U7002, including a renamed function, a renamed capability and an entry without brackets;
- a handler from another module, and a handler whose `uses` are inferred, installed by a locked function;
- the removal of a locked binding, which is accepted.

## Amendment (2026-10-14): when `una fix` applies the condition on the callee

- Status: Accepted
- Deciders: Una project (accepted on 2026-10-14)

Rule 5 says the first repair at a call or a `with` "is mechanical when every parameter `C` names is passed unchanged as an argument, so `una fix` may apply it". It left the edit's text and its limits open, and every `effect_condition` repair has carried `edit: null` so far. This amendment settles which repairs carry an edit, and when. ADR 0084's amendment of the same date settles when `una fix` sees them.

**Which repairs carry an edit.** Only the first repair at a call or a `with`: the condition written in the callee's or the handler's `uses`. The others keep a description only:

- **a guard `if C`**, at an operation, a call, a `with` or a call of a function value. It changes what the program does: where `C` is false, the effect no longer runs, and nothing says so. That can hide the mistake the diagnostic found, such as a payment that should have failed instead. When the guarded expression's value is used, a guard also needs an `else` value that no rule gives;
- **`requires C`** on the enclosing function. It changes the function's contract for each of its callers, in other packages too for a public function.

Rule 5 calls the guard at an operation "mechanical". This amendment withdraws that word: the guard is a description, and `una fix` never applies it. At an operation, the first repair is the guard, so `una fix` applies nothing there (ADR 0084 rule 2: a later repair is never applied in place of the first).

**The target.** The edit goes on the binding entry of the callee's or handler's `uses` whose condition was the premise `C′` of the refuted obligation (rule 3): the binding that the caller's binding `@b` becomes, whether it is passed by name, renamed with `with @x = @b`, or narrowed with `@b[ops]`. When the entry writes no condition, `C′` is `true`.

**The edit.** At a site where the caller conditions `op` on `@b` by `C`, the edit writes `C″` on `op` in the target. `C″` is `C` with each occurrence of a caller's parameter replaced by the callee's parameter it is passed to:

- the replacement is simultaneous, so the call `f(b, a)` with `C = a and not b` gives `C″ = y and not x` for `f(x, y)`. Only occurrences that resolve to a parameter are replaced: never a field name, a text literal or another binder;
- when the target writes no condition, the edit writes `op where C″`;
- when it writes `op where C′`, the edit writes `op where (C′) and (C″)`, with both operands in parentheses whatever their operators (`and`, `or`, `=>`), so the result is always their conjunction. The condition already written stays, word for word;
- when the operation carries a label (ADR 0094), the condition follows it: `op gives L where …`, as `cap_op` writes them (rule 1).

**When the repair carries the edit.** All of these must hold. Otherwise the repair carries the same description with `edit: null`:

- (a) **Arguments, at their entry values.** Every parameter of the caller that `C` names is passed to a parameter of the callee or the handler, as an argument that is exactly a reference to that parameter. The parameter is not an `edit` parameter. `C` reads the arguments as the caller received them (the second amendment of 2026-10-13), while the call passes the parameter's current value, and the two are the same only for a parameter the body cannot assign. A parameter passed to several parameters is replaced by the first of them, in argument order.
- (b) **An explicit operation.** The callee or handler is declared in the same module as the site, and its `uses` writes the target binding with `op` listed by name. An inferred `uses` (`spec/05 §9` rule 2), or a binding written without brackets, would need a clause written or an operation list spelled out. ADR 0084 never applies an edit that declares `uses`, so neither gets an edit.
- (c) **Names.** Each name of `C` other than the replaced parameters resolves, in the callee's or handler's header, to the declaration it resolves to in the caller's header, where `C` was written. In particular, no parameter of the callee or handler has the name of such a name. This is ADR 0096 rule 3a's hygiene: the edit never changes what a name of the condition refers to.
- (d) **Not public, and not a value.** The callee or handler is not `pub`. A function is also not used as a function value anywhere in the package. A named function whose `uses` carry a condition may not be a function value (U3006, rule 1), and a public function's uses in other packages cannot be seen from this one.
- (e) **A module of the package.** The callee or handler is declared in a module of the package, never in a dependency (ADR 0084 rule 2).
- (f) **Within the encoding.** `C`, and `C′` when the target writes one, are inside the verifier's encoding (`spec/12 §4`). A caller's condition outside it is refuted at every site (rule 3 keeps the goal `not false`), and a callee's is dropped from the premise, so a conjunction with it would be dropped too. In both cases the edit could never discharge the obligation.
- (g) **Not already written.** `C″` is not already a conjunct of the target's condition. The conjuncts of a condition are its operands after its top-level `and` is flattened, with the parentheses that enclose each one removed, so `((C′) and (C″₁)) and (C″₂)` has the conjuncts `C′`, `C″₁` and `C″₂`. Each is compared with `C″` token for token. An edit whose conjunct is already there is known not to help, and writing it again would only grow the text.
- (h) **No unknown obligation.** No obligation of the package's tier-1 check is unknown (U5002). Whether a query is unknown can depend on the solver's time, and an edit taken in one run and not in another could leave another text. So while any obligation is unknown, even one unrelated to the site, every `effect_condition` repair carries a description only, in `una check` as in `una fix`. The reason is not silent: a first repair at a call or a `with` that meets (a) to (g), at a site whose obligation is unknown or refuted, ends its description with "; not applied while an obligation of this package is unknown (U5002)". The description is free text in `una.diagnostic/1`, so the schema does not change.

(a) and (f) make the edit discharge the obligation it repairs. (h) keeps solver timing from choosing edits. (g) is the backstop: whatever a later rule changes, `una fix` never writes the same conjunct twice, so a round cannot repeat an edit until the round limit.

**What the edit does to authority.** It adds a condition and removes none, so the callee or handler may perform `op` in fewer states than before. It never widens a `uses`, a visibility or a grant. A `[contracts]` lock on the callee still holds, since `C′ and C″ ⇒ C_lock` whenever `C′ ⇒ C_lock` (rule 8). Every caller, lock and body is checked again by the next round and by the final check.

**What the edit does to other declarations.** It edits a declaration other than the one the diagnostic is reported in, so its effect is stated here:

- **at the site,** the obligation becomes `(C′ and C″)[args] ⇒ C`. By (a), `C″[args]` reads the same values as `C`, and by (f) both are encoded, so the obligation holds;
- **at the callee's other call sites,** the callee's condition only gains a conjunct. That makes their obligations `C′[args] ⇒ C` no harder;
- **in the callee's body,** each performance of `op` through the binding, each call that passes it on and each `with` it reaches now has an `effect_condition` obligation under `C′ and C″`. One may be refuted. The diagnostic then moves into the callee, at its own site, where it is either repaired the same way one call further down in a later round of `una fix`, or guarded by hand. ADR 0084's round limit bounds the chain;
- **two sites** that ask the same callee for different conditions give edits with the same range. ADR 0084 rule 3 takes the first by position and diagnostic order. The next round checks the other site again against the conjunction, and if its obligation is still refuted, it adds its own conjunct.

**Determinism.** The edit's text is computed from the source alone: the site, `C`, the arguments and the callee's header. It needs no solver result. The same source gives the same edit whenever it gives one. Solver timing can only withhold an edit, never change it: only a refuted obligation (U5001) carries the edit, and by (h), none does while an obligation is unknown (U5002). A refuted obligation stays refuted and a proven one stays proven whatever the timing, so every check with no unknown obligation gives the same edits. For a whole run, a round of ADR 0084 in which an obligation is unknown takes no tier-1 edit, so the rounds stop there. Timing can then only stop the rounds earlier: a slower run applies a prefix of the rounds of a faster one, and never other edits or the same edits in another order.

A test runs `una fix` twice on the same package and compares the outputs and the written sources byte for byte. A second case, with `step_budget = 1` in its `verification.lock` so that every obligation is unknown, shows no applied edit, the source unchanged, and the repair's description ending with the sentence of (h), on both runs.

**Stricter only.** The amendment changes no rule of acceptance: a program that checks today still checks, and one that is rejected is still rejected. It only lets `una fix` write a condition that the checker already offers as the first repair.

Conformance cases, with the source before and after and a check of the fixed source:
- one case for each of (a) to (h) where the repair stays a description. They include an `edit` parameter assigned before the call, a condition outside the encoding, an unrelated obligation made unknown, and a conjunct already written, in the parenthesized form that `una fix` writes, at a site that stays refuted, so that the next round is refused by (g);
- the simultaneous replacement, with the arguments swapped;
- a call and a `with` where the edit applies, the second through a renamed binding (`with @x = @b`);
- a target that already writes a condition containing `or`, and a `C″` containing `or`, where the parentheses keep the conjunction;
- an operation with a label (`op gives L`);
- a diagnostic that moves into the callee and is repaired one call further down in the next round;
- two sites asking the same callee for different conditions;
- `--tier 0`, which applies nothing.

## Note (2026-10-14): condition (g) as implemented

The amendment of 2026-10-14 states (g) as "`C″` is not already a conjunct of the target's condition". The implementation (#639) withholds the edit when every conjunct of `C″`, its operands once its own top-level `and` is flattened, is already a conjunct of the target's condition. A `C″` that is itself one conjunct is the case the amendment states. A `C″` with several is withheld in more cases, never fewer, so `una fix` writes nothing the amendment would not, and never writes the same conjunct twice. `spec/05 §3` states this form.

## Amendment (2026-10-15): when `una fix` restores a locked condition

- Status: Proposed (#725)

Rule 8 says the repair of U3023 "is to restore the locked condition or label (rank 1, mechanical)", and the repair has carried `edit: null` so far, so `una fix` never applies it. This amendment gives the first repair of a refuted `contract_lock` obligation an edit, and states when. ADR 0084's amendment of the same date lets `una fix` see it.

**The target.** The header's operation that the obligation is charged at: `op` in the header's `uses` clause of the locked binding, the one binding that names the entry's binding with the entry's capability (the third amendment of 2026-10-13). A second binding of that capability, or the locked binding renamed, is U3023 of tier 0 at the binding, with no obligation and no edit. Tier 1 only runs once tier 0 is clean, so the target is never ambiguous.

**The edit.** `C_lock` is exactly the text that the span of the lock's condition expression covers in the clause's string, after the string's escapes are decoded (#727). It holds the condition and nothing else: not the binding, the capability, the operation list, `where`, or whitespace around the expression. So the header gets the lock's condition token for token: a condition written `region == \"eu\"` in `una.toml` is written `region == "eu"`. Then:
- when the header writes `op` with no condition, the edit writes `op where C_lock`;
- when it writes `op where C_src`, the edit writes `op where (C_src) and (C_lock)`, both operands in parentheses whatever their operators, as the amendment of 2026-10-14 writes a conjunction. The condition already written stays, word for word.

The edit changes the module's header only. It never changes `una.toml`, the operation list, the binding, the capability or another clause: changing the lock stays never a repair (rule 8). When ADR 0094's labels are implemented, the condition follows a label as rule 1 writes it, `op gives L where …`. Today a lock entry with `gives` is U7002, so no lock has one.

**When the repair carries the edit.** All of these must hold. Otherwise the repair carries the same description with `edit: null`:

- (a) **An explicit operation.** The header lists `op` by name in the binding's brackets. A binding written without brackets would need its operation list spelled out, which no edit of ADR 0084 writes (as condition (b) of the amendment of 2026-10-14).
- (b) **One line.** The decoded `C_lock` contains no `\n`, the only character that the lexer reads as a line break. A comment runs to the next `\n`, and `C_lock` ends where its expression ends, so it holds no comment, and every token of it lands inside the binding's brackets on the header's line. A comment after the condition in the lock's string is not possible either: it would run over the clause's closing `]`, so the entry would not parse, which is U7002 at the lock. U+2028 and U+2029 are not line breaks to the lexer; inside a text literal they are written back unchanged, on the same line.
- (c) **Within the encoding.** `C_lock`, and `C_src` when the header writes one, are inside the verifier's encoding (`spec/12 §4`). A lock condition outside it is reported at the lock entry, with its own repair (#638). A header condition outside it is dropped from the premise, so a conjunction with it would be dropped too, and the obligation would stay refuted.
- (d) **Not already written.** Not every conjunct of `C_lock` is already a conjunct of `C_src`, in the form of the note of 2026-10-14 on condition (g). With (c), such an obligation is proven, never refuted, so this is a backstop only.
- (e) **Refuted, and no unknown obligation.** Only a refuted `contract_lock` obligation carries the edit, never an unknown one (U5002). While any obligation of the package's tier-1 check is unknown, no repair carries it, under condition (h) of the amendment of 2026-10-14. A repair that meets (a) to (d) then ends its description with the same sentence: "; not applied while an obligation of this package is unknown (U5002)".

**Characters.** The lock's string is decoded and then read by the same lexer as a module. That lexer refuses a carriage return, every control character other than a tab and a line feed, and the bidirectional controls U+202A to U+202E and U+2066 to U+2069 (`spec/01`, ADR 0017), anywhere, text literals and comments included, so a lock that holds one is U7002. The text the edit writes therefore holds only characters that a module's source may hold, and the restored header is read the way the lock was read, with no hidden direction change.

**Names (hygiene).** The edit writes a condition whose names already resolve at the header. Rule 8 checks a lock condition against the function's current parameters, with the context and the scope of the header's own conditions (rule 1), and reports U7002 at the lock entry otherwise. So every name of `C_lock` resolves, at the header, to the declaration it resolved to when the lock was checked, and the edit adds no binder. In particular, a parameter that the header has renamed, removed or retyped since the lock was written makes the lock U7002 at tier 0 (rule 8: "which includes a parameter that has been renamed, removed or retyped"). Tier 1 does not run, no `contract_lock` obligation exists, and the edit never writes a name that resolves to nothing. ADR 0096 rule 3a holds with nothing withheld.

**What the edit does to authority.** The header's condition on `op` becomes `C_lock`, or `C_src and C_lock`. The function may perform `op` in fewer states than before, never more. The edit never widens a `uses`, a grant, a visibility or the lock, and removes no condition. With (c), `(C_src and C_lock) ⇒ C_lock` holds, so the obligation it repairs is proven.

**What the edit does to other declarations.**
- **The function's callers:** at each call or `with` that passes a binding to the locked function, the obligation `C′[args] ⇒ C` gains a conjunct in its premise `C′`. That makes it no harder.
- **The function's body:** each performance of `op` through the binding, each call that passes it on and each `with` it reaches now has an `effect_condition` obligation under the stronger condition. One may be refuted. The diagnostic then sits in the body at its own site. There it is guarded by hand, or, at a call or a `with` that meets the amendment of 2026-10-14, repaired one call further down in a later round. ADR 0084's round limit bounds the chain.
- **Other locks:** a lock constrains its own function's header only, so no other `contract_lock` obligation changes.

**Determinism.** The edit's text is computed from the module and `una.toml` alone: the header's operation, `C_src` and the lock's string. It needs no solver result. Solver timing can only withhold the edit, under (e), never change it, as the amendment of 2026-10-14 states for its own edit.

**Stricter only.** No rule of acceptance changes: a program that checks today still checks, and one that is rejected is still rejected. `una fix` may now write a condition that the checker already offers as the first repair.

**Cost.** One edit per locked condition, computed with the lock's check, linear in the condition's text. No new query.

Conformance cases, with the source before and after, a check of the fixed source and `una.toml` unchanged byte for byte:
- a header operation without a condition, restored to `op where C_lock`;
- a header condition containing `or`, restored to `op where (C_src) and (C_lock)`;
- a lock condition with a text literal written with TOML escapes, restored with the decoded text;
- one case for each of (a) to (e) where the repair stays a description: a binding without brackets, a lock condition spanning two lines, a header condition outside the encoding, and an unrelated obligation made unknown by `step_budget = 1`, with the sentence of (e). (d) is a backstop that a refuted obligation never meets, so a unit test pins it;
- a lock string with a comment after the condition, `… where region == "eu" -- x]`: U7002 at the lock, and `una fix` applies nothing;
- a lock condition with a text literal holding U+2028, written `\u2028` in `una.toml`: restored on one line, with the character unchanged;
- a parameter renamed in the header since the lock was written: U7002 at the lock entry, and `una fix` applies nothing;
- a parameter named like a function of the module, used in the lock condition: the restored header reads it as the parameter, and checks;
- an edit after which an operation in the body is refuted, reported at that operation in the next round;
- `--tier 0`, which applies nothing;
- `una fix` run twice, the outputs and the written sources compared byte for byte.
