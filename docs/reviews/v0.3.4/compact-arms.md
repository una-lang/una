# The compact reference, measured (#430)

- **Issue:** #430. The owner decided on 2026-10-08 that the compact reference is measured, not assumed.
- **Protocol:** N-64 (`docs/reviews/v0.3.3/n64-plan.md`), with its amendment on temporary files (`dogfood-fresh.md`), and the changes below.
- **Runner:** Claude (Una-1).
- **Compared with:** N-64 run A, 7 `una check` runs (twice), on the v0.3.3 archive with the three guides.
- **Status:** plan, fixed before the runs. The results follow in this file.

## Arms

All arms use one archive: the one `scripts/local-release.sh` builds for aarch64-apple-darwin from the tip of #430, rebased on a `main` that holds `una fix` (#429). Its `docs/guide/` is then cut down per arm:

| Arm | `docs/guide/` holds |
| --- | --- |
| **C**, compact only | `una-compact.md` |
| **F**, full guides | `una-for-agents.md`, `syntax-reference.md`, `getting-started.md` |
| **CF**, both | all four |

- **Agents and setup:** each arm is one general-purpose Opus 5.5 subagent at default effort, with no prior conversation, in its own `mktemp -d` directory `$W`. This is N-64's setup. Agent runs are billed to the session's usage, as N-64's were, not to the benchmark's.
- **Prompt:** N-64's exact prompt, its five tasks, and the sentence of its amendment ("keep temp files under $W/tmp"). The prompt names `$W/una-0.3.3/docs`, as it did for run B, whatever the arm holds there.
- **Logging, the 10-run limit, the audit of every tool call for paths outside `$W`, and my verification of every final package:** as N-64.

## Measures

For each arm:
- `una check` runs per task and in total;
- passed or gave up;
- acceptance, as N-64 §3;
- the share of fixes from the message alone, from `una explain` and from the docs;
- the `DOC:` notes.

Also recorded:
- whether the agent used `una fix`;
- how much of the docs it read: the files and sections its log names, and the bytes it read, from the tool calls.

## Validity

- **One run per arm.** N-64 run A gave 7 twice, but run B gave 11 and 23. A difference of 2 or less from 7 is within noise. An arm that differs by 3 or more is run again, and both runs are reported.
- **The compiler differs from run A's.** The archive is v0.3.3 plus `una fix`, ADR 0078 and ADR 0082, and the full guide now names `una fix`. So F, not run A, is the fair baseline for C and CF. The comparison with 7 is reported, but as context only.
- **Teaching to the test.** The compact reference was written after N-64. It covers the rules N-64 found agents missing, and those rules are general: labels and policies, JSON escapes, `Map[Text, Nat]`, a loop's contract on a helper. It names no task. Its two snippets are a trusted policy and a map tally. The full guide has the same tally helper (§11), so F and CF see it too. I report this as a limit: a held-out task set would be the stronger test, and it is not part of this decision.
- **Gate 1 is untouched.** una-bench pins v0.1.5, which keeps its guide.
