# 0084. `una fix`: apply the mechanical repairs

- Status: Accepted
- Date: 2026-10-08
- Deciders: Leonardo (2026-10-08, written); written by Claude (Una-1) at the coordinator's request
- Resolves: issue #429
- Amends: `spec/13 §3` ("Repairs are never applied automatically") and the command table of `spec/13 §5`

## Context

`una check` reports repairs (`spec/13 §3`). A repair whose `edit` is not `null` is mechanical: a file, a range and a replacement. Every agent that uses Una applies these edits itself. The dogfood passes (#402, N-64) and the Gate 1 notes show agents re-implementing edit application: positions in Unicode scalar values, edits that overlap, an order to apply them in, and a check after. A mistake there is the agent's, and it costs an iteration. The repairs test (`crates/una-cli/tests/repairs.rs`, #257) already applies every first edit of a check together and asserts that the edits never overlap. That is the procedure an agent needs, minus a command.

The owner decided on 2026-10-08 to add the command, with these rules: only edits that are not `null`, never an edit with `escape_cost > 0`, deterministic rounds of non-overlapping edits per file, then `una fmt`, `--dry-run`, and `--json` as a versioned document.

**What the compiler offers today.** All mechanical edits have `escape_cost` 0. Most fix a spelling or a form: a closest name, an argument label, `is` for `==` on a variant, an escape, an indentation. Five kinds change what a declaration grants:
- U3002 "add `op` to the `uses` clause of `@x`";
- U3001 and U3003 "declare `uses ...` in the header";
- U1004 "declare `name` with `pub`", which exports a private item;
- U3008 and U3009 "declare `fails K` in the header".

The first three widen authority or visibility: the checker reports that a body does more than its header permits, and the edit makes the header permit it. Applying that without a person is how least authority erodes. The owner's rule (2026-10-06) is to pick the most secure option. A `fails` clause states a failure the body already has, and widens no authority.

## Decision

1. **Command.** `una fix [path]` applies the mechanical repairs of the package at `path`. It accepts `--dry-run`, `--tier 0`, `--json` and `--color`, as `una check` does. `una check` never writes, and `una fix` writes only module files.
2. **Which edits.** For each diagnostic, only its first repair, in the order `spec/13 §1` prints them (by `escape_cost`, then `rank`), is a candidate. It is applied when all of these hold:
   - its `edit` is not `null`, so advice is never applied;
   - its `escape_cost` is 0, so nothing adds `checked`, `trusted`, a policy or a budget;
   - it does not widen authority or visibility: the U3002 edit that adds operations to a `uses` clause, the U3001 and U3003 edits that declare `uses`, and the U1004 edit that adds `pub` are never applied, and no flag turns this off (Leonardo, 2026-10-08, written: "Never apply, no flag");
   - its file is a module of the package, never `una.toml` or `verification.lock`, and holds valid UTF-8, since writing a decoded text back would replace its invalid bytes;
   - it is not a U7001 edit, since step 4 formats every file.

   A later repair is never applied in place of the first: the ranking is the checker's choice of fix, and a lower-ranked alternative is the agent's choice. A candidate that is not applied is reported as skipped, with its reason.
3. **Rounds.** Edits are computed and applied in memory, and each round checks the package again at tier 0, where every mechanical edit comes from:
   - Within a file, a round sorts candidates by start, then end, then diagnostic order. It takes each candidate that does not conflict with one already taken.
   - Two edits conflict when their ranges overlap, or when they touch at one point and either inserts there. An edit identical to one taken (same range and replacement) is merged into it.
   - Conflicting edits wait for the next round, where the new check recomputes them.
   - Rounds stop when a round takes no edit, when the texts return to a state already seen, or after 16 rounds. All three cases are reported.
4. **Format.** After the rounds, every module that parses is rewritten in canonical form, as `una fmt` does. A file that does not parse is left as the rounds left it, and its syntax errors are reported.
5. **Write.** Unless `--dry-run` is given, each changed file is written once, through a temporary file and a rename, as `una fmt` writes (a symbolic link is resolved, and the link is kept). With `--dry-run`, nothing is written, and the output is what a run would apply.
6. **Report.** Finally the package is checked at the requested tier (1 by default), and its diagnostics are the ones left.
   - **Human output:** one line per applied edit on stdout, `fixed <file>:<line>:<col> <code> <description>`, with positions in the text before the edit's round. Then one line per skipped candidate, `skipped <file>:<line>:<col> <code> <description>: <reason>`. The remaining diagnostics go to stderr, as `una check` prints them.
   - **`--json`:** prints `{"schema":"una.fix/1","dry_run":...,"rounds":n,"stopped":"done"|"cycle"|"limit","applied":[...],"skipped":[...],"formatted":[...],"diagnostics":[...]}`.
     - `applied`: each entry is `{"round","file","code","description","start","end","replacement"}`, with positions in the text of its round.
     - `skipped`: each entry is `{"file","code","start","description","reason"}`, with its position in the final text. The reason is one of `widens authority`, `escape cost`, `not a module` or `invalid UTF-8`. Edits still waiting on a conflict when the rounds stop are reported with `round limit`.
     - `formatted`: the paths of files rewritten by step 4.
     - `diagnostics`: the remaining `una.diagnostic/1` objects.
     - The schema follows ADR 0076.
7. **Exit codes** (ADR 0036): 0 when no diagnostic remains, 1 when one does, 2 for a usage error, 3 for U8001.
8. **Complexity.** A round costs one tier 0 check, linear in the package as `una check` is, plus O(e log e) to sort its e edits and O(n) to apply them to a text of n characters. The number of rounds is bounded by 16, so `una fix` is at most 17 checks: the rounds plus the final one.

`spec/13 §3` is amended: `una check` and the other commands never apply repairs, and `una fix` applies the mechanical ones under the rules above. `spec/13 §5` lists the command.

## Consequences

- An agent runs `una fix`, then reads what remains. The edits it used to apply by hand, and their overlaps, are the compiler's job.
- What an agent must still decide is visible: skipped entries name the authority widenings, which need a person or a deliberate edit, and advice-only repairs stay in the diagnostics.
- Adding a new mechanical repair to the checker makes `una fix` apply it, unless it widens authority or costs an escape. A repair that widens authority MUST be marked as such where it is built, so that `una fix` skips it.
- `una check` output and its schema are unchanged.

## Alternatives considered

- **Apply authority widenings too, or behind a flag.** That is what an agent following rank 1 does today. A tool that does it silently, for every function at once, makes the `uses` clause track the body. A clause that does that cannot bound it. The owner declined both, with no flag (2026-10-08, written). An agent can still apply the reported edit deliberately.
- **Fall through to the next mechanical repair** when the first is skipped. That would apply a fix the checker ranked lower, without the agent seeing the first. Declined.
- **One round only.** Overlapping edits (for example, a label and a name in one argument) would need a second `una fix` run. Bounded rounds converge in one command.
- **Apply U7001 edits in the rounds.** A U7001 edit spans whole declarations and conflicts with every other edit in them. Formatting once at the end is equivalent and simpler.

## Amendment (2026-10-14): tier-1 rounds for refuted effect conditions

- Status: Accepted
- Deciders: Una project (accepted on 2026-10-14)

Rule 3 checks each round at tier 0, "where every mechanical edit comes from". ADR 0092's amendment of the same date gives one tier-1 diagnostic an edit: a refuted `effect_condition` obligation at a call or a `with`, whose first repair writes the condition in the callee's or handler's `uses`. A tier-0 round never sees that diagnostic, so this amendment adds where `una fix` looks for it.

**Rounds.**

- A round checks the package at tier 0, as today. When that check reports no diagnostic, and the requested tier is 1, the round also checks the package at tier 1. Its candidates are then the tier-1 diagnostics that rule 2 admits.
- With `--tier 0`, no round runs tier 1, so no tier-1 edit is ever applied.
- When the tier-0 check reports a diagnostic, the round takes only tier-0 candidates, as today. The package's tier-1 diagnostics wait until tier 0 is clean, as they do in `una check`.
- **An unknown obligation stops the tier-1 edits.** While any obligation of the package is unknown (U5002), even one unrelated to the sites, no `effect_condition` repair carries an edit (ADR 0092's amendment, condition (h)). So such a round has no tier-1 candidate, and when it has no tier-0 edit either, it takes no edit and the rounds stop as `done`, as rule 3 says. The repairs without an edit are not candidates, so they are not reported as skipped, and the U5002 stays among the remaining diagnostics. Whether a query is unknown can depend on the solver's time (`wallclock`, which the proof cache never stores, ADR 0033 rule 7). Two sites refuted in different rounds could otherwise be taken in another order, and leave another text. With this rule, timing can only stop the rounds earlier: a slower run applies a prefix of the rounds of a faster one.
- **The output schema is unchanged.** No `reason` of `skipped` and no value of `stopped` is added, so `una.fix/1` and `spec/13 §5` keep their closed sets (ADR 0076 rule 3).
- The rest of rule 3 is unchanged: conflicts, merges, ordering, the stop conditions and the limit of 16 rounds.

**Which tier-1 edits.** Rule 2 holds unchanged, with one more condition for a diagnostic of tier 1: it MUST be refuted (U5001). An unknown obligation (U5002) never yields an edit, whatever its repairs say. Only the `effect_condition` repairs of ADR 0092's amendment are tier-1 candidates. Today no other tier-1 repair carries an edit. A future kind that does needs its own amendment, which states its determinism as (h) does, before `una fix` applies it. The edit's text depends on the source alone (ADR 0092's amendment: "solver timing can only withhold an edit, never change it").

**An edit outside the reported declaration.** The edit can change a declaration other than the one its diagnostic is reported in: the callee or handler, which must be in the same module. Rule 2's conditions apply to it as written. Its effect on other declarations is stated in ADR 0092's amendment, and the next round's check sees it. The report gives the edit's own file and position, as for every applied edit.

**The count** (ADR 0096 rule 6). `"fixable"` counts these diagnostics too, when their first repair carries an edit. As rule 6 says, it counts the candidates of the first round, not the edits that round applies. Two sites that ask the same callee for different conditions count 2, while the first round applies 1 of them (rule 3, conflicts). `una check` reports tier-1 diagnostics only when tier 0 is clean, which is when the first round of `una fix` looks at them too.

**Complexity** (rule 8). Write T0 and T1 for the costs of a tier-0 and a tier-1 check.
- Today `una fix` costs at most 16 × T0 for the rounds, plus the final check at the requested tier, T0 + T1.
- With this amendment, a round whose tier-0 check is clean also costs T1, so the bound is 16 × (T0 + T1) + (T0 + T1).
- With a proof cache (ADR 0064, which the command uses when it can read or make the user's key), a round's queries that an earlier round already solved are answered from the cache, so a round solves again only the queries its edits change. Without a key there is no cache (ADR 0064 rule 3), and every tier-1 round solves every query again.
- A package that writes no conditioned operation generates no `effect_condition` obligation. Its first tier-1 round finds no candidate and ends the rounds, as a round that takes no edit does today.

**Stricter only.** No rule of acceptance changes, and no edit that widens authority or costs an escape becomes applicable. The one new kind of edit adds a condition.

A conformance case pins each point:
- `--tier 0`, which applies no tier-1 edit;
- a package with a tier-0 diagnostic and a refuted `effect_condition`, where only the tier-0 edit is applied in the first round and the condition in a later one;
- an obligation made unknown by `step_budget = 1`, with no applied edit, no skipped entry, `stopped` `done`, the source unchanged, and the repair's description saying why (ADR 0092's amendment, (h));
- the run-twice comparison of ADR 0092's amendment;
- the `fixable` count of two sites asking one callee.

## Amendment (2026-10-15): a restored locked condition is a tier-1 candidate

- Status: Proposed (#725)

The amendment of 2026-10-14 admits one kind of tier-1 edit, and says that "a future kind that does needs its own amendment, which states its determinism as (h) does, before `una fix` applies it". ADR 0092's amendment of 2026-10-15 is that kind: the first repair of a refuted `contract_lock` obligation restores the locked condition in the header, and its determinism is stated there, under its condition (e), as (h) states it.

**Which tier-1 edits.** A tier-1 diagnostic is a candidate when its obligation is refuted and its first repair carries an edit, for these two kinds only:
- an `effect_condition` obligation, reported as U5001, as the amendment of 2026-10-14 says;
- a `contract_lock` obligation, reported as U3023 (ADR 0092 rule 8), whose repair is ADR 0092's amendment of 2026-10-15.

An unknown obligation (U5002) is never a candidate. A U3023 of tier 0, for a binding or an operation that the lock does not grant, has no obligation and no edit, so it is never one either. The rest of rule 2 holds unchanged.

**Rounds.** The rounds of the amendment of 2026-10-14 are unchanged: a round checks tier 1 only when its tier-0 check reports no diagnostic and the requested tier is 1. With `--tier 0` no tier-1 edit is ever applied. While an obligation is unknown, no tier-1 repair carries an edit, so the round takes none, and when it has no tier-0 edit either, the rounds stop as `done`. A program for which the checker computed neither a condition edit nor a lock restoration has no tier-1 candidate, so its tier 1 is left to the final check, as before.

**An edit inside the reported declaration.** Unlike the condition on a callee, this edit changes the declaration its diagnostic is reported in: the header of the locked function. Its effect on the function's body and callers is stated in ADR 0092's amendment, and the next round's check sees it.

**The count** (ADR 0096 rule 6). `"fixable"` counts these diagnostics too, when their first repair carries an edit.

**The output schema is unchanged.** No `reason` of `skipped` and no value of `stopped` is added (ADR 0076 rule 3).

**Complexity** (rule 8). Unchanged: a round whose tier-0 check is clean costs T0 + T1, and `una fix` at most 16 × (T0 + T1) + (T0 + T1). The lock restorations are computed with the check, linear in the locked conditions' text.

**Stricter only.** No rule of acceptance changes, and the new kind of edit only adds a condition back.

Conformance cases:
- a package with a refuted `contract_lock` and nothing else, restored in the first round, with `stopped` `done` and the final check clean;
- a package with a tier-0 diagnostic and a refuted `contract_lock`, where the tier-0 edit is applied in the first round and the lock restoration in a later one;
- `--tier 0`, which applies no restoration;
- an obligation made unknown by `step_budget = 1`, with no applied edit, no skipped entry, `stopped` `done` and the source unchanged;
- the `fixable` count of a package with one restorable lock.
