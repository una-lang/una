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
