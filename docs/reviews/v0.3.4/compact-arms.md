# The compact reference, measured (#430)

- **Issue:** #430. The owner decided on 2026-10-08 that the compact reference is measured, not assumed.
- **Protocol:** N-64 (`docs/reviews/v0.3.3/n64-plan.md`), with its amendment on temporary files (`dogfood-fresh.md`), and the changes below.
- **Runner:** Claude (Una-1).
- **Compared with:** N-64 run A, 7 `una check` runs (twice), on the v0.3.3 archive with the three guides.
- **Status:** done (2026-10-09). The plan was fixed before the runs; the results, arm C's rerun included, follow in this file.
- **Timing:** the arms run after Gate 1's calibration round 1 ends (the coordinator, 2026-10-08), since Gate 1 has priority on usage and on the machine.

## Arms

All arms use one archive: the one `scripts/local-release.sh` builds for aarch64-apple-darwin from the tip of #430, rebased on a `main` that holds `una fix` (#429). Its `docs/guide/` is then cut down per arm:

| Arm | `docs/guide/` holds |
| --- | --- |
| **C**, compact only | `una-compact.md` |
| **F**, full guides | `una-for-agents.md`, `syntax-reference.md`, `getting-started.md` |
| **CF**, both | all four |

- **Agents and setup:** each arm is one general-purpose Opus 5.5 subagent at default effort, with no prior conversation, in its own `mktemp -d` directory `$W`. This is N-64's setup. Agent runs are billed to the session's usage, as N-64's were, not to the benchmark's.
- **Prompt:** N-64's exact prompt, its five tasks plus the two below, and the sentence of its amendment ("keep temp files under $W/tmp"). The prompt names `$W/una-0.3.3/docs`, as it did for run B, whatever the arm holds there.
- **Logging, the 10-run limit, the audit of every tool call for paths outside `$W`, and my verification of every final package:** as N-64.

## Two held-out tasks

At the coordinator's request (2026-10-08), every arm also runs two tasks that neither N-62 nor N-64 used. I wrote them before any arm ran. They are reported apart from the five original tasks, so the result shows whether the compact reference generalizes or only covers what N-64 found.

The prompt is N-64's, with "Write five small programs" changed to "Write seven small programs", and these two tasks after its fifth:

> 6. report/: a command-line program whose `main` reads the text file `readings.txt`, in the directory `data/` of the package, which `una run` may only read. The file holds one integer per line, but some lines are blank or not numbers. The program prints one line with the number of valid readings and their sum. The counting is a separate pure function, with at least one contract (`requires` or `ensures`) that the toolchain proves, and an example. `una run` must print the line.
>
> 7. allow/: a function that decides whether an e-mail address, which the caller holds as a secret value, is on an allowlist that is stored only as HMAC hashes made with the key named `pepper`. The address is never compared in clear. Include an example that runs it against a stand-in for the key service.

**Setup.** The runner makes `$W/work/report/data/readings.txt` with the six lines `12`, `7`, `x`, `-3`, an empty line and `40`. It makes `$W/work/allow` empty, as for the other tasks.

**Acceptance, checked by me:**
- **report:**
  - `una run` prints a line that holds `4` and `56`;
  - the grant reads `data` only;
  - the contract is proven, with no `checked` and no weakened contract.
- **allow:**
  - the address is `secret` in the function's signature;
  - it goes through `Keys.hmac` with the key name `pepper`;
  - only `Hashed` values are compared;
  - the example passes with a handler.

**Feasibility.** Before writing this section, I solved both tasks with the compiler of `main` (cbc72b5), to be sure each has a solution in edition 0.2.
- Both needed `internal` labels at several positions.
- `allow` needed `filter` for a membership test over `Hashed`, since the prelude has no `any`.
- My solutions stay outside the repository until the runs end. They are then added under `compact-arms/reference/`. They are not a baseline: I know Una.

## Measures

For each arm, with the five original tasks and the two held-out tasks reported apart:
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

## Results (2026-10-09)

The arms ran one at a time, arm C twice (C-1, then its rerun C-2, per the validity rule), on the archive built from 15ed392f (sha256 `8c602dd32dd35d9417d3f70a63fff3a1b83379de4c33706b11c22d753dd8c549`). Each had a fresh `$W` and N-64's exact prompt, with the two held-out tasks and the temp-file sentence from the start.

`una check` runs per task are counted as N-64 counted run A: every run that changed something, without a final run with no change. The run that first passed is in parentheses.

| Task | C-1, compact only | C-2, its rerun | F, full guides | CF, both | N-64 run A (v0.3.3) |
| --- | --- | --- | --- | --- | --- |
| greet | 2 (2) | 3 (3) | 1 (1) | 1 (1) | 1 |
| orders | 4 (3) | 4 (3) | 1 (1) | 1 (1) | 1 |
| stock | 4 (3) | 2 (2) | 1 (1) | 2 (1) | 3 |
| tally | 2 (2) | 2 (2) | 1 (1) | 1 (1) | 1 |
| sensors | 2 (2) | 3 (2) | 3 (1) | 1 (1) | 1 |
| **Original five** | **14 (12)** | **14 (12)** | **7 (5)** | **6 (5)** | **7** |
| report (held out) | 3 (2) | 7 (4) | 3 (2) | 3 (2) | – |
| allow (held out) | 6 (6) | 3 (3) | 1 (1) | 2 (2) | – |
| **Held-out two** | **9 (8)** | **10 (7)** | **4 (3)** | **5 (4)** | – |
| Passed / gave up | 7 / 0 | 7 / 0 | 7 / 0 | 7 / 0 | 5 / 0 |
| Guide text read | ~6 KB, plus a dump of the 83 `una explain` texts it searched | ~23 KB (it read the compact reference three times), plus ~18 KB of `una explain` texts for 13 codes | ~119 KB | ~85 KB | – |
| `DOC:` notes | 14 | 17 | 9 | 3 | 2 |
| `una fix` used | no | no | no | no | – |

**The rerun.** C-2 ran from a fresh `$W`, on the same archive, with the same prompt, after the session's usage allowed it (2026-10-09). It repeats C-1 on the original five exactly, 14 (12), and differs by 1 on the held-out two, 10 (7) against 9 (8): within the noise rule. Its report took 7 runs: the agent first read `data/readings.txt` under the grant `files:data:read`, the path gap below, then made a failed read fail `main` instead of printing a false zero, which took two runs to name `FileError`. Its packages and logs are in `compact-arms/C-2/`, with the user name the greeting printed replaced by `<user>`.

**Acceptance.** I reran `una check`, `una test` and `una run` on every final package, and every arm met every acceptance item:
- greet prints through a policy;
- orders checks the exact output;
- stock's token is `secret` and reaches `X-Api-Token` through a policy;
- tally has a proven contract over the map;
- sensors uses `together for`;
- report prints 4 and 56, with a read-only grant and a proven contract;
- allow's address is `secret`, goes through `Keys.hmac` with `pepper`, and only `Hashed` values are compared.

No package uses `checked`, and no obligation was a wallclock unknown.

**Audit.** Every tool call of every arm was scanned for paths outside `$W`.
- C-1, C-2 and F are clean.
- CF is clean with the harness exception N-64 accepted for run B: one Read of a Claude Code persisted-output file, which held only the agent's own `cat` of `readings.txt` and `una-for-agents.md`.
- F's first attempt stopped after its first task on the session's weekly usage limit. It is void, kept in `compact-arms/F-voided/`, and was rerun from a fresh `$W`.

### Findings

- **The compact reference does not replace the guides.**
  - Alone, it cost twice the check runs of the full guides on the original five tasks (14 against 7, in both of C's runs) and on the held-out two (9 and 10 against 4).
  - The gap is the same on tasks it was not written after, so it is not an artifact of teaching to the test: what it leaves out costs iterations on any task.
- **Added to the full guides, it gains nothing measurable** (6 against 7, and 5 against 4, both within noise).
- **What it saves is reading:** about 6 KB against about 85 to 119 KB, roughly a twentieth.
- **Where C stumbled, from its `DOC:` notes:** what the compact reference omits.
  - The `import` syntax and the content of `verification.lock`.
  - One `uses` line per capability.
  - The leading `|` of a one-variant sum, without which `kind E = V` is an alias.
  - `old(p).len`, not `old(p.len)`.
  - That `parse_int` fails rather than returning an `Option`.
  - The signatures of `Keys.hmac`, of the other `Http` operations, and of `std.testing`.
  - How a `files:` grant resolves a path.

  Each item is short, but together they would take the reference past 2,000 tokens.
- **A gap in the full guides too:** every arm, F and CF included, wrote `data/readings.txt` for a grant of `files:data:read`. Under that grant the path is `readings.txt`, and no guide says how a `Files` path relates to its grant.

### Recommendation

Ship `una-compact.md` as a quick start that points to the full guide, not as a substitute.
- The DOC gaps above go to an issue for the full guide, the path of a `files:` grant first.
- The reference stays under its cap.

The weaker-model question is separate and is not part of this measurement.

### Done after the first results

- **Arm C's rerun** (C-2, above): C-1's difference from 7 is confirmed, and no conclusion changes.
- **`compact-arms/reference/`** holds my solutions of the two held-out tasks, written before any arm ran.
