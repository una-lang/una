# 0097. Widening repairs rank after the ones that keep authority, and say what they widen

- Status: Accepted
- Date: 2026-10-13
- Deciders: Una project (accepted on 2026-10-13)
- Resolves: issue #579
- Target: every edition. Repairs, messages and explain pages are tooling output, not language rules, so nothing a program means changes. The Gate 1 pin does not move.
- Implementation: #579.
- Amends: `spec/13 §1` (the repair object), `spec/13 §3`, the open fields of the `spec/13` preamble, and the texts of U1004, U3001, U3002, U3003, U3004 and U3012

## Context

ADR 0095 ranks escape repairs last and marks them in `--json`. A second kind of repair needs the same treatment: one that gives code more authority or visibility than it had. Such a repair is not an escape, since every check still applies, but it changes what the code may do, and agents follow the first repair they see.

**The evidence.** una-bench `findings/for-v0.2.md`, entry 1, from the security errors Claude Haiku 4.5 made in TypeScript, set against their Una outcomes (`docs/haiku-b-violations-in-una.md`):

- In B10 and B12, U3002 arises from a helper with a narrower header, and its rank-1 repair widens that header, which produces the violation.
- In B09, a confused deputy, U3002 flags the privileged lookup, but the first-ranked repair widens the editable headers up the call chain, and the variant then builds and violates.

The finding names "repairs that do not widen authority as their first-ranked option" as a direction for v0.2.

**Today.** The checker marks a repair that widens authority or visibility with an internal flag, and `una fix` never applies such a repair (ADR 0084). The flag is not part of the JSON shape, so an agent cannot see it. The repairs and messages that widen are:

- U3002 at an operation: "add `op` to its `uses` clause", rank 1 unless another binding in scope permits the operation. The message itself also says "add `op` to its `uses` clause".
- U3002 at a call: rebinding with `with @need = @other` ranks first when it applies (issue #69); otherwise "add `op` to the `uses` clause of `@x`" is rank 1. The first route is given its rank by hand.
- U3003, code without `uses` that calls an effectful callee: "declare `uses ...` in the header" is the only repair.
- U3001, a binding not in scope: "declare `uses ...` in the header" is the only repair, and the message says "declare it with `uses`". Two variants have the advice in the message alone, with no repair.
- U3004, a binding used in code without `uses`: the advice is in the message alone.
- U1004, a private declaration used from another module: "declare `x` with `pub`" is the only repair.
- U3012, `main` uses a host operation that is not granted: there are no repairs, and the message names `[run] grants` and `--grant`. For a capability that is not a host capability, the message says to install a handler with `with` inside `main`.

## Decision

1. **Widening kinds.** A repair widens when it takes one of these routes:
   - `authority`: it adds operations to a `uses` clause, or declares a `uses` clause;
   - `visibility`: it makes a declaration `pub`;
   - `grant`: it grants a host capability's operations to the program, in `[run] grants` or with `--grant`.
2. **Widening is not an escape.** An escape (ADR 0095) replaces a check with a runtime test or with trusted code. A widening is fully checked: the header states the new authority, every caller must hold it, and the grants of `main` still gate every host capability. So a widening keeps `escape_cost` 0, and `escape_cost` keeps meaning "checks replaced".
3. **The `widens` key.** Each repair object of `una.diagnostic/1` gains an optional key, after `escape` when both are present:

   ```json
   { "rank": 2, "escape_cost": 0,
     "widens": { "kind": "authority", "cost": "the function then holds this authority, and every caller must hold it too: check that the operation belongs to this function's job" },
     "description": "add `write` to the `uses` clause of `@files`", "edit": { ... } }
   ```

   - The key is present exactly when the repair widens. `kind` is one of the kinds of rule 1, and `cost` is one sentence:
     - `authority`: "the function then holds this authority, and every caller must hold it too: check that the operation belongs to this function's job";
     - `visibility`: "every module may then call it: check that it belongs to the module's interface";
     - `grant`: "the program then gets this host authority when it runs".
   - The internal flag is derived from the key, so the two cannot disagree.
   - Under ADR 0076 rule 1, an added key keeps the schema at `una.diagnostic/1`. `widens.kind` is an open field under ADR 0076 rule 4: a consumer that meets an unknown kind still knows from the key's presence that the repair widens. `spec/13` lists it among the open fields.
   - Human output prints `repair N (widens authority: <cost>): ...`, `widens visibility` or `widens grant` as the kind says. It is built from the same data as the JSON.
4. **Order.** Output sorts repairs by `escape_cost`, then by whether they widen, then by `rank`, and numbers the ranks 1 to n in that order (ADR 0095 rule 3). So a repair that keeps authority comes before a widening, and a widening before an escape. When a repair that keeps authority exists, rank 1 never widens. The ranks set by hand in U3002 are no longer needed.
5. **`una fix`.** A widening repair is never applied by `una fix`, as ADR 0084 rules today. The key changes what an agent sees, not what `una fix` does.
6. **Messages state the problem.** As in ADR 0095 rule 4, the routes out of a diagnostic are repairs:
   - U3002 at an operation drops "; add `op` to its `uses` clause" and "use `@other`, which permits it, or ...". Both routes stay as repairs.
   - U3001 and U3004 drop "declare it with `uses`" and "declare `uses @x: ...` in its header". Where the message was the only route, a repair takes its place: an edit of the header when the header is editable, and advice otherwise. It is marked `authority`.
   - U3012 gains repairs. For a host capability: "grant `C[ops]` in `[run] grants`", marked `grant`. For a capability that is not a host capability: "install a handler for it with `with` inside `main`", which keeps authority, and the message keeps only the problem.
7. **The repairs of each diagnostic.**

   | Diagnostic | Repairs that keep authority | Widening, after them |
   | --- | --- | --- |
   | U3002 at an operation | use `@other`, which permits the operation, when one is in scope | add the operation to the `uses` clause (`authority`) |
   | U3002 at a call | rebind with `with @need = @other`, when it applies | add the operations to the `uses` clause (`authority`) |
   | U3003 | none | declare `uses` in the header (`authority`) |
   | U3001, U3004 | none | declare the binding with `uses` (`authority`) |
   | U1004 | none | declare it `pub` (`visibility`) |
   | U3012, host capability | none | grant the operations in `[run] grants` (`grant`) |
   | U3012, other capability | install a handler with `with` inside `main` | none |

   No new route that keeps authority is invented. A generic "move the call to code that already holds the authority" would be advice without a target. The value is that the widening is visible and costed, and that the routes that keep authority, where they exist, come first.
8. **Enforcement.** The conformance runner checks, on every diagnostic of every case, that `widens` is present exactly when the repair widens, that no repair that keeps authority follows a widening among repairs of equal `escape_cost`, and that ranks have no gaps. A text guard fails any repair without `widens` whose description adds to a `uses` clause, declares `uses`, adds `pub` or grants. Named cases check the exact repairs of U3002 at both sites, with and without another binding, U3001, U3003, U3004, U1004 and U3012 for both kinds of capability.

## Consequences

- An agent that takes the first repair keeps the code's authority whenever a route that keeps it exists. When it widens, the diagnostic has told it what the widening costs. In B10 and B12 the rank-1 repair is still the widening, since no other route exists, but it is marked with its cost.
- An operator sees a grant that the program would need as a costed repair of U3012.
- `una.diagnostic/1` keeps its version, and consumers that ignore unknown keys are not affected. The messages of U3001, U3002, U3004 and U3012 change. Conformance cases match codes and positions, not messages, so they are not affected; unit tests that assert the messages change with them.
- `una fix` is not affected (rule 5).
- Algorithmic cost: sorting the repairs of a diagnostic is O(r log r) in its r repairs, as today, and renumbering is O(r). The enforcement checks are linear in the repairs of the suite.

## Alternatives considered

- **An `escape` kind `authority`.** Rejected: a widening replaces no check (rule 2), and folding it into `escape_cost` would make that count mean two things.
- **Mark only authority, not visibility or grants.** Rejected: a host grant is the widening an operator most needs to see costed, and `pub` widens what every other module may call.
- **Keep the messages and change only the repairs.** Rejected: an agent reads the message first, and a widening there is unranked and unmarked (ADR 0095 rule 4).
- **Generic routes that keep authority.** Rejected for now (rule 7). The design gaps behind B07 and B09, on whose behalf and at which boundary authority is used, are ADR 0093 and ADR 0094.
