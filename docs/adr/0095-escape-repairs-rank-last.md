# 0095. Escape repairs rank last and say what they cost

- Status: Accepted
- Date: 2026-10-12
- Deciders: Una project (accepted on 2026-10-12)
- Resolves: issue #574
- Target: every edition. Repairs, messages and explain pages are tooling output, not language rules, so nothing a program means changes. The Gate 1 pin does not move.
- Implementation: #574.
- Amends: `spec/13 §1` (the repair object), `spec/13 §3`, the open fields of the `spec/13` preamble, and the texts of U2016, U4003, U4004, U5001, U5002, U7003 and U7005

## Context

A repair is the route the compiler proposes out of a diagnostic, and agents follow it. `spec/13 §3` ranks the repairs of U5001 and U5002 and gives each an `escape_cost`: a stronger type, a `requires` clause and a guard cost 0, and `checked` costs 1. Output sorts repairs by `escape_cost`, then `rank`. That rule covers only U5001 and U5002. Other diagnostics build their repairs freely, and some of them propose an escape first, unmarked.

**The evidence.** In the full run of una-bench Suite C (`docs/full-c.md`, `findings/for-v0.2.md` entry 2), every Una run that used an escape hatch (7 of 36) used the same pair: one or two `checked`, and a raised `[escape_budget] checked_per_module`. The transcripts show the route:

- U5001 already ranks `checked` last, as "repair 4 (escape cost 1)". All 7 runs saw it. The label says nothing about what the cost is.
- U7003, the escape budget exceeded, has one repair: "prove the fact statically, or raise `checked_per_module` in `una.toml` and explain the escape in review". It is rank 1 with `escape_cost` 0, so the escape is the first route and is not marked. 5 of the 7 runs saw it. Its explain page's fixed example fixes the code only by raising the budget.
- U2016, an opaque predicate called outside `checked`, has one repair, `checked`, with `escape_cost` 0. 3 of the 7 runs saw it. Its explain page's fixed example uses `checked` and a raised budget.

**Trust escapes have the same shape.** A policy lowers labels, and its `via` function is not label-checked (`spec/07 §5` rule 3), so declaring a module trusted, or declassifying through a policy, widens the trusted base:

- U4003, a labeled argument at a sink, says in its message "declassify it through a policy first". Its field repair merges "interpolate the accepted fields, or declassify through a policy", at cost 0.
- U4004, a policy outside a trusted module, says in its message to declare the module trusted and list it in `una.toml`. It has no repairs, so the only route offered widens trust.
- U7005, trust declared in one place only, has no repairs. Its two fixes point in opposite directions: one keeps the trusted base as it is, and the other widens it. The diagnostic does not say which is which.

## Decision

1. **One repair, one route.** A repair never joins a fix that keeps the guarantee and an escape in one description. "Prove X, or raise Y" is two repairs.
2. **Escape kinds.** A repair is an escape when it takes one of these routes:
   - `checked`: a runtime test replaces a static proof (`spec/06 §6`);
   - `escape_budget`: it raises `[escape_budget] checked_per_module`;
   - `trusted`: it declares or lists a trusted module, or declassifies through a policy (`spec/07 §5`, `§6`).

   An escape has `escape_cost` 1 or more. Every other repair has `escape_cost` 0.
3. **Order.**
   - Output keeps the order of `spec/13 §1`: by `escape_cost`, then `rank`.
   - Ranks are numbered 1 to n, without gaps, in output order, so the printed "repair 1" is the first route.
   - Every escape ranks after every repair that keeps the guarantee. When a repair that keeps the guarantee exists, rank 1 is never an escape.
4. **Escape advice lives in repairs, never in the message.** A message states the problem. U4003 and U4004 lose their imperative clause, and the routes become repairs.
5. **The `escape` key.** Each repair object of `una.diagnostic/1` gains an optional key, after `escape_cost`:

   ```json
   { "rank": 3, "escape_cost": 1,
     "escape": { "kind": "checked", "cost": "a runtime test replaces the static proof: the program fails at run time where the proof would have rejected it; it counts against [escape_budget] and is listed in the build report" },
     "description": "check the value with `checked(value, x > 0) else fail <Variant>`", "edit": null }
   ```

   - The key is present exactly when `escape_cost` is above 0. `kind` is one of the kinds of rule 2, and `cost` is one sentence that says what the guarantee loses.
   - Under ADR 0076 rule 1, an added key keeps the schema at `una.diagnostic/1`.
   - `escape.kind` is an open field under ADR 0076 rule 4. A consumer that meets an unknown kind still knows from `escape_cost` that the repair is an escape. `spec/13` lists it among the open fields.
   - Human output prints the kind and the cost in place of "(escape cost N)", as in "repair 3 (escape: checked; a runtime test replaces the static proof ...)". It is built from the same data as the JSON.
6. **The repairs of each diagnostic.** Each repair is offered when it applies.

   | Diagnostic | Repairs that keep the guarantee (cost 0) | Escapes, last |
   | --- | --- | --- |
   | U5001, U5002 (refinement, precondition) | stronger parameter type, `requires` clause, guard (unchanged) | `checked` |
   | U7003 | state the fact in a refined type or a `requires` clause and remove the `checked`; handle the failing values with a guard | raise `checked_per_module` (`escape_budget`) |
   | U2016 | if code needs only the test and not the fact, test with an ordinary function with the same body under `if` (conditions below) | `checked` |
   | U4003 | pass or interpolate only data at the accepted label | declassify through an existing policy (`trusted`) |
   | U4004 | move the policy into a module that is already trusted, when one exists | declare this module trusted in its header and in `[trusted] modules` (`trusted`) |
   | U7005, header only | remove `trusted` from the header (see below when the module declares a policy) | list the module in `[trusted] modules` (`trusted`) |
   | U7005, listing only | remove the entry from `[trusted] modules` (likewise) | add `trusted` to the header (`trusted`) |

   The ordinary-function route of U2016 establishes no fact: the result of a call reaches the solver only through the function's `ensures`, which cannot name an opaque predicate (`spec/03 §4`), and an ordinary call is not in the refinement language, so nothing is selfified. A function that returns a kind whose `where` names the predicate fails its own return obligation, since the solver knows nothing of the predicate, so `checked` stays the only route there. The route is offered under three conditions:
   - only for a use in code, not in a clause, a `reflect` body or an example, where a call of a function is not allowed either;
   - only when the predicate's body calls no other opaque predicate, which would be U2016 again in a function, and only for a predicate of the package, whose body the checker has seen;
   - its description never suggests a result kind whose `where` names the predicate.

   U2016 has a third route that keeps the guarantee: declaring the predicate `reflect`, so that the solver knows its definition. It applies only when the predicate's body is in the refinement language, and the checker learns that only by checking the body as a `reflect` body. So it is not offered as a repair: the explain page names it with its condition, and a body outside the refinement language is then U2015. It is a plain strengthening only when the predicate appears in no capability operation's clauses and in no kind of an operation's signature. Otherwise those contracts are assumptions (`spec/05 §1`): with `reflect` they give callers the predicate's whole definition, not an uninterpreted atom, so code that was rejected can be accepted by assumption. The page says so, and that each such operation must be verified against a handler.

   Removing trust never lets a policy through: a policy in a module that is not trusted is U4004 (`spec/07 §6`), so the narrowing repairs of U7005 can only produce a diagnostic, never a silent change of what a policy may do. The other effects of trusted status do not change silently either: the label exemption covers only a policy's `via` function, which is then behind U4004; the rest of a trusted module is checked normally; the trusted base in the build report shrinks; a policy call does not depend on the caller's trust; and no verification rule depends on it. When the module declares a policy, narrowing only moves the error from U7005 to U4004. The repair stays first, since every escape ranks last (rule 3), and its description says so: "its policies are then U4004: move them into a trusted module".
7. **Explain pages.** The fixed example of each page above shows a fix that keeps the guarantee. The escape is named last, with its cost.
8. **Enforcement.** A test runs every conformance case with `--json` and checks rules 2, 3 and 5 on every diagnostic: `escape` is present exactly when `escape_cost` is above 0, escapes come after every other repair, and ranks have no gaps. Named cases (the U5002 step budget case, a U5001 refinement case, and cases for U7003, U2016, U4003, U4004 and U7005) check the exact order and keys.

## Consequences

- An agent that takes the first repair keeps the guarantee whenever a repair that keeps it exists. When it does take an escape, the diagnostic has told it what the escape costs.
- The routes of the trust escapes are visible and ranked, so widening the trusted base is never the first route offered.
- `una.diagnostic/1` keeps its version, and consumers that ignore unknown keys are not affected. The messages of U4003 and U4004 change. Conformance cases match codes and positions, not messages, so they are not affected.
- Algorithmic cost: sorting the repairs of a diagnostic is O(r log r) in its r repairs, as today, and renumbering is O(r). The enforcement test is linear in the number of cases.
- `una fix` is not affected. It applies only mechanical edits, and none of these escapes has an edit.

## Alternatives considered

- **Drop escapes from repairs.** Rejected: `checked` is the only way to establish an opaque fact (`spec/03 §4`), and an agent should learn the escape and its cost from the diagnostic rather than from guessing. Ranked last and marked, it stays honest without being the default.
- **Keep the order and only improve the explain pages.** Rejected: the evidence shows agents act on the repair lines, and U7003's escape is rank 1 today.
- **A new conformance directive for repairs.** Rejected for now: it changes the format of ADR 0013. The JSON test of rule 8 checks the same facts without it.
- **Bump to `una.diagnostic/2`.** Not needed: no existing key changes (ADR 0076 rule 1).

## Open obligations

- Whether to mark authority-widening repairs (U3002) in the JSON the same way is a separate issue, #579. This decision covers the escapes of rule 2 only.
