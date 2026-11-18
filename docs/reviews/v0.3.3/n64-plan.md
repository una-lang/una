# N-64: a second dogfood pass by a fresh agent (protocol)

- **Status:** fixed before the runs, by decision N-64 (2026-10-08). It runs after the v0.3.3 tag.
- **Compares with:** the dogfood pass of #402 (`dogfood.md`): 12 `una check` runs over five programs.
- **Author:** Una-1.

## 1. Materials, runs and isolation

From v0.3.3 the release archive holds the three guides under `docs/guide/` (#410, N-65): `una-for-agents.md`, `syntax-reference.md` and `getting-started.md`, next to `una`, `lib/libuna_rt.a`, `LICENSE` and `README.md`.

- **Run A:** the v0.3.3 archive for this host, as released. This is the realistic first contact, and it compares with the 12 of #402.
- **Run B:** the same archive with `docs/` removed, so README plus `una explain` only. It measures what the guides are worth.
- **Model:** both runs use Opus 5.5 at default effort, as the #402 pass did, so the comparison isolates the documents. There is no rerun on another model.

Setup for each run:
1. Make a fresh empty directory `$W = mktemp -d`, and unpack the archive into `$W/una-0.3.3`. For run B, remove `$W/una-0.3.3/docs` afterwards.
2. Put Z3 at the locked version on PATH. Without it, tier 1 is U5003.
3. Make empty task directories `$W/work/{greet,orders,stock,tally,sensors}`. There is no `una.toml` and no lock: the agent makes them.
4. Nothing from the repository is given: no spec, ADR, source, tests, examples, reviews, and none of #402's programs or report.

Isolation and audit:
- The agent is a general-purpose subagent with no prior conversation, not a fork.
- After each run, its tool calls are scanned. A path outside `$W` (other than Z3 on PATH), or a web search about Una, voids the run.

In run B, the prompt's sentence about `$W/una-0.3.3/docs` still names the directory, whose absence the agent then finds.

## 2. The prompt (exact text; $W is filled in)

You are trying out a programming language you have never used, called Una. Write five small programs in it. Everything you need is in the directory $W: the `una` toolchain in $W/una-0.3.3 (use `$W/una-0.3.3/una`, and `una --help` and `una explain <code>`), and its documentation in $W/una-0.3.3/docs and its README in $W/una-0.3.3. Work only inside $W. Do not read, list or search any other directory, and do not search the web. If something is unclear, rely on the documentation, the compiler's messages and `una explain`. Do not guess from other languages, and do not give up on the language's rules.

Each program goes in its own directory under $W/work/, as a package of edition 0.2 that `una check` accepts with exit code 0. Where a task asks for examples, `una test` must pass them.

1. greet/: a command-line program whose `main` reads the environment variable USER and prints one line about the user. Printing the name itself or something derived from it is fine. `una run` must print the line.
2. orders/: a pure function that takes the JSON text `{"lines": [{"name": "a", "qty": 2}, {"name": "b", "qty": 3}]}` and returns the JSON text `{"count":2,"names":["a","b"],"total":5}` (key order and spacing as the language's JSON writer gives them). Include an example that checks exactly this input and output.
3. stock/: a function that asks an HTTP API, `GET https://inventory.example.com/v1/items/<item>`, for an item's stock level. It sends the API token, which the caller holds as a secret value, in the header `X-Api-Token`, and returns the `count` field of the JSON response, or nothing when anything fails. Include an example that runs it against a stand-in for the API that answers `{"count": 7}`.
4. tally/: a function that counts votes per candidate from a list of names into a map, with at least one contract (`requires` or `ensures`) about the map that the toolchain proves, and an example.
5. sensors/: a function that reads a list of sensors concurrently, each of which can fail. One version fails with the first failure in list order; another counts how many sensors answered. Include examples for both, with a stand-in sensor bank where one sensor fails.

Process, and logging (this matters as much as the programs):
- After writing or changing a program, run `una fmt <dir>` and then `una --json check <dir>`.
- Keep a log file, $W/work/<task>/LOG.md. Append one entry per `una check` run, with:
  - the run number;
  - every diagnostic code and the first 100 characters of its message;
  - what you consulted to decide the fix: "message only", "una explain <code>", or "docs: <file> §<section>";
  - whether that was enough to fix it (yes, partly or no);
  - what you changed, in one line.
- When `una check` passes, run `una test` (and `una run` for greet). Log them the same way.
- When you think the documentation was missing, wrong or ambiguous, add a line starting with `DOC:` saying where and what.
- Stop a task after 10 `una check` runs without success. Log `GAVE UP` and move on.
- At the end, write $W/work/SUMMARY.md with each task's number of check runs, and whether it reached a passing state.

## 3. Results sheet

Per task, from LOG.md and verified by me by rerunning `una check`, `una test` and `una run` on the final files:

| Task | Check runs to clean (mine) | Check runs (fresh) | Passed? | Codes met | Message-only fixes | Explain fixes | Doc fixes | Unfixed or gave up | DOC: notes | Recurrence of D-1 to D-8 |
|---|---|---|---|---|---|---|---|---|---|---|
| greet | 2 | | | | | | | | | D-2? |
| orders | 3 | | | | | | | | | D-1? D-7? |
| stock | 2, +1 for the example | | | | | | | | | D-5? |
| tally | 4 | | | | | | | | | D-3? D-4? D-8? |
| sensors | 1 | | | | | | | | | none |
| Total | 12 (+1) | | | | | | | | | |

Acceptance, checked by me:
- greet prints a line about USER under `una run --grant console --grant env`.
- orders' example passes with the exact output text.
- stock's token is `secret` at its source, the header carries it, and the example passes with a handler.
- tally has a proven `requires` or `ensures` over the map, with no `checked` and no weakened contract (I read it).
- sensors has both versions with examples passing.

Derived measures:
- iterations ratio (fresh / mine);
- the share of diagnostics fixed from the message alone;
- the share that needed the docs;
- each D-finding marked as recurred, fixed-by-release (#403 and #404) or not met;
- new findings, ranked the same way as dogfood.md.

Validity notes, recorded with the result:
- the model and effort;
- the transcript audit result (paths touched);
- whether the agent knew Una beforehand, judged from its first attempts (for example, writing `when`/`is` without having read the guide).

## 4. Deliverable

- `docs/reviews/<next version>/dogfood-fresh.md`, with the filled sheet, the agent's LOG.md files and SUMMARY.md quoted, its final programs as cases, and the comparison with my 12.
- Issues for the new findings go to the coordinator first.

## 5. Cost and time

Two Opus subagents, one per run, each capped at 50 check runs (10 per task), plus test and run commands. It is light, but tier 1 spawns Z3, so each run happens in a local-ci slot.
