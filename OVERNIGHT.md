# Overnight run

Source of truth for the autonomous run, now on branch `dev/v0.1` of una-lang/una (until the public import, `overnight/v0.1` of the legacy repository). Parts B and C below are copied verbatim from the owner's instructions of 2026-09-29, with the mechanical rename of ADR 0025 applied (Una, codes `U####`, `.una` files). Part D records the consolidated instructions that the owner gave later the same day. Re-read this file at the start of every milestone and after any context compaction or restart.

## Part B. Answers to your M1 plan

Your M1 plan is approved with the amendments below. Record them in ADR 0017 (Accepted, Deciders: Leonardo) in the lexer commit, together with the spec and registry edits they need. Write the approved plan plus these amendments to `docs/plan/runs/M1-plan.md`.

### B.1 The four checks on 7920c27

1. Yes, the truncated row was `type` → "use `kind`".
2. Approved: the message form is "`<keyword>` is <message>", for example "`match` is not a keyword in Una; use `when`".
3. Approved: D6 also skips hidden files inside multi-module cases.
4. Approved: human diagnostics move to stderr in the M1 CLI commit.

### B.2 Decisions 1 to 8

1. **Malformed integers.** Add a new lexical code, **U0007 "Malformed literal"**. It covers a leading zero on anything other than `0` itself, a doubled underscore, and a leading or trailing underscore.
   - Message: "malformed integer literal `007`".
   - Attach a mechanical edit when the fix is unambiguous: `007` �7`, `1_` → `1`, `1__0` → `1_0`.
   - Why not U0101: this is a lexical error, and a precise code with a repair is better for agents.
2. **Control characters.** Accepted: U+0000 to U+001F except tab and line feed, plus U+007F, are U0001 everywhere, including literals and comments.
   - Extend the rule to Unicode bidirectional controls, U+202A to U+202E and U+2066 to U+2069. This defends against Trojan Source attacks in reviewed code.
   - Text that needs any of these characters uses `\u{...}` escapes.
3. **Characters that cannot start a token.** Accepted: U0001, one per character.
   - Add a hint specific to Una to the message where one helps:

     | Character | Hint |
     | --- | --- |
     | `;` | statements end at line breaks |
     | `!` (not followed by `=`) | use `not` |
     | `&` | use `and` |
     | `'` | text literals use double quotes |
     | `_` starting a name | names start with a letter |

   - Unit tests cover the hints; conformance directives check code and position only.
4. **Bare `}` in text.** It is an error, reported as **U0007**, not U0101, because it is lexical.
   - Message: "unescaped `}` in text; write `\}`".
   - Attach the mechanical edit.
5. **Missing interpolation `}`.** Accepted: a single U0004 at the outer opening quote.
6. **Capability op clauses.** Accepted: extend U0102 with the order `requires*`, `ensures*`, `fails?`, `accepts?`, allowing at most one `fails` and one `accepts`.
7. **Orphan doc comments.** Accepted: treated as plain comments, with no error.
8. **Robustness wording.** Accepted as you wrote it. Update the M1 acceptance text in `docs/plan/milestones-v0.1.md`.

### B.3 Design choices (your section 7)

All accepted:
- where each code points;
- the U0002 and U0003 repairs;
- the recovery rules;
- the explain file format;
- `--tier` accepting 0 and 1, defaulting to 1.

### B.4 Housekeeping (your section 8)

Accepted. The `.gitattributes` entry becomes `tests/conformance/syntax/*.una -text`.

### B.5 Explain texts and cases

- Your nine texts are approved after the rename: Una in prose, ` ```una ` fences.
- Add a U0007 explain text in the same format.
- Add conformance cases for:
  - U0007: `007`, `1__0`, `1_` and an unescaped `}`;
  - U0001: a bidi control character inside a comment;
  - a tier0-ok case with `\u{202E}` written as an escape inside a text literal.

---

## Part C. Autonomous run (after Part A succeeds)

### C.0 Mission

Implement M1 (your approved plan plus Part B), then M2 to M10 in order, without waiting for my input.

Quality beats progress. Stopping at M5 with everything green and every decision documented is better than reaching M10 with shortcuts.

### C.1 Setup

1. Create branch `overnight/v0.1` from `main` after Part A, and push it. Work only on this branch.
2. Create `OVERNIGHT.md` at the repository root:
   - copy Parts B and C of this prompt into it verbatim;
   - add a "Progress" section;
   - re-read it at the start of every milestone and after any context compaction or restart. It is the source of truth for this run.
3. Create `docs/reviews/open-decisions.md` (the decision log, C.4) and `docs/reviews/morning-report.md` (C.8).
4. Add the status `Provisional (implemented, pending human review)` to `docs/adr/0000-template.md`. Provisional ADRs in this run start at 0018.
5. If CI does not run on pushes to this branch, make the smallest change to `ci.yml` that enables it, and log it.

### C.2 Autonomy rules

These override CLAUDE.md where they conflict, for this run only.

1. **Plans.** Do not wait for approval. Write each milestone plan to `docs/plan/runs/M<n>-plan.md`, then execute it.
2. **Spec gaps.** Do not stop. Decide using C.3, write an ADR with status Provisional, implement it and log it.
3. **Nothing silent.** Every choice not literally dictated by the spec goes into the decision log. Small choices get short entries.
4. **Everything else in CLAUDE.md still applies:** source-of-truth order, scope guard, Rust rules, tests first, commit conventions.

### C.3 Decision protocol (think deeply before choosing)

1. Restate the problem precisely and cite the spec sections involved.
2. Generate at least two real options, or three when the impact is high.
3. Evaluate the options against these criteria, in order:
   1. the spec's text, then its evident intent;
   2. charter principles P0 to P9 and the proof boundary (charter §3);
   3. strictness: prefer the option that rejects more programs or preserves more guarantees, since relaxing a rule later breaks no program and tightening one does;
   4. consistency with accepted ADRs and existing code;
   5. simplicity, both to implement and to reverse;
   6. agent ergonomics: precise diagnostics and mechanical repairs.
4. Choose one option. State your confidence (high, medium or low) and the reversal cost (low, medium or high, and what would change).
5. Isolate the decision so it is cheap to reverse:
   - put it in one clearly named function, constant or module where possible;
   - tag the code and its tests with the comment `DECISION D-NNN`.

### C.4 Decision log format (`docs/reviews/open-decisions.md`)

Keep entries sorted with the highest review priority first: low confidence combined with high reversal cost goes at the top.

```
### D-NNN: <short title> (M<n>)
- Question: <one or two sentences>
- Context: <spec sections, ADRs>
- Options: A) ... B) ... C) ...
- Chosen: <option>, because <two to four sentences>
- Confidence: high | medium | low
- Reversal cost: low | medium | high, <what would change>
- Implemented in: <commits>, <files>, tag `DECISION D-NNN`
- ADR: <path> (Provisional), or "none (minor)"
- Answer needed: Keep <option>? Or switch to <alternatives>?
```

### C.5 Hard limits (never cross these, whatever the reason)

**Guarantees and tests**
- Never weaken a guarantee to make progress. This includes:
  - letting `unknown` pass;
  - relaxing information-flow rules;
  - treating capabilities as values;
  - skipping obligations;
  - loosening one-effect-per-line.
- Never edit or delete tests, snapshots or expectations to make them pass, unless the spec shows the old expectation was wrong. If so, log it as a decision.
- Use `-- pending: M<n>` only for features of a future milestone. Never use it to hide a failure in the current one.

**Scope**
- Never implement backlog items or reserved-keyword features.
- No dependencies outside the allowlist in `compiler/architecture.md` §7.
  - If one seems necessary, first try implementing the small piece with the standard library or an allowed crate.
  - If you still need it, write a Provisional ADR and log it with high priority.
- After Part A, never edit the charter or an accepted ADR. To change an accepted ADR, write a Provisional ADR that supersedes it and log it with high priority.

**Git, repository and machine**
- After Part A, never push to `main`.
- Never force-push or rewrite published history.
- Never change repository settings or add secrets.
- Never edit files outside this repository, including Claude configuration files.
- Never use sudo or install global software beyond what `scripts/` already does.

### C.6 Execution loop, per milestone

1. Re-read OVERNIGHT.md, CLAUDE.md, the milestone section and everything in its Read list.
2. Write the plan file: tasks, conformance cases per code, explain texts, known gaps. For M1, use `docs/plan/runs/M1-plan.md` from Part B.
3. Write the tests first (conformance and unit), then implement in small commits. Each commit passes fmt, clippy and tests.
4. Self-review before declaring the milestone done:
   - build a checklist of every MUST and MUST NOT in the milestone's spec sections;
   - mark each one as implemented and tested, or log why not;
   - append the checklist to the plan file.
5. Remove the `-- pending: M<n>` markers of this milestone. Those cases must now pass.
6. Run the full local checks and the determinism tests (`compiler/testing.md` §4). Push the branch and wait for CI. If it is red, fix it before moving on.
7. Update the Progress section of OVERNIGHT.md and the morning report, then start the next milestone.

### C.7 When stuck

- After three genuinely different attempts at the same problem, log it as a Blocker in the decision log, with what you tried and what you learned.
- If the milestone cannot meet its acceptance criteria:
  - do not start the next one on a red base;
  - finish the morning report;
  - stop.
- If a problem blocks one task but the milestone can still pass its criteria another way:
  - take the most conservative path;
  - log it;
  - continue.

### C.8 Morning report (`docs/reviews/morning-report.md`)

Update it after every milestone, not just at the end.

1. **Status:** last completed milestone, branch, last green CI run link.
2. **Part A:** the rename commit, the CI result and the `grep` output from A.7.
3. **Milestones table:** status (done, partial, not started), commits, test counts, and conformance cases passed and pending.
4. **Decisions needing my review:** the top 10 from the log, each answerable in one line ("Keep" or "Switch to B").
5. **Blockers and deviations from the spec.**
6. **Hindsight and next steps:** anything you would do differently, and what to do next.

### C.9 Refactoring contract

In the morning I will answer entries like "D-007: switch to B". For each answer:
- update the ADR to Accepted or Superseded;
- update every place tagged `DECISION D-007`, its tests and the spec;
- log the change.

Design every provisional decision now so that this refactoring is cheap.

---

## Part D. Consolidated instructions (2026-09-29)

The owner's consolidated instructions take precedence over Parts B and C where they conflict. Parts 1, 2 and 5 are summarized here; Part 3 is copied verbatim below.

### D.1 Summary of Parts 1, 2 and 5

- **Name.** The final name of the language is Una (ADR 0025, which supersedes ADR 0016). "Vera" and "Troth" remain only as history: `docs/reviews/triage.md §5`, ADRs 0016 and 0025, and the process files (this file, the morning report, the decision log and `docs/plan/runs/`). Prose never puts an article directly before the name ("not a keyword in Una").
- **Examples.** The reference example is `examples/events` (ADR 0026). Every example, specification fragment, test fixture and explain text stays domain-neutral, and documents refer to the example it replaced only as "the previous reference example".
- **Publication (Part 5).** Done on 2026-09-29, after the owner withdrew the deferral: the canonical remote is `origin` = una-lang/una, private until launch, whose history starts at the initial public import `25d865b`; the earlier history stays in the `legacy` remote (ADR 0030). The working branch is `dev/v0.1`, and everything Parts B and C say about `overnight/v0.1` applies to `dev/v0.1`. Nothing is ever pushed directly to `main`; the owner merges after review.
- **Hard rule.** Never push to `origin` any branch whose history includes commits from before the initial public import, including the old `main` and `overnight/v0.1`. Never push to `legacy` again.
- **CI quota.** Linux runs on every push; macOS only on pull requests and on pushes to `main` (D-083). Local checks on the owner's Mac cover macOS during the run.
- **Scope of the run.** The owner is not launching yet: after checkpoint 3, continue through M10 as far as quality allows.
- **Reminder for M8.** The M8 plan records that M8 removes the interim run behavior of ADR 0027.

### D.2 Part 3, verbatim

## Part 3. Usage checkpoints

### 3.1 Roadmap section

Add a section to `docs/plan/roadmap.md` called "v0.1 usage checkpoints". The milestone order does not change.

| Checkpoint | Reached after | What the user can do | Demo |
| --- | --- | --- | --- |
| 1. Read | M1, M2 | Write `.una` files, get syntax diagnostics with exact positions, format code | `una check --tier 0`, `una fmt`, `una explain U0101` |
| 2. Understand | M3 to M6 | Get name, type, authority and information-flow errors | a missing `uses` (U3001), `@` in a pure function (U3004), a `secret` value printed to `Console` (U4003), `una contract` |
| 3. Run (first usable version) | M7 | Write `main`, check it and run it; run the examples with effect traces | `una run examples/hello --grant console`, `una test examples/events`, `una check --dump-core` |
| 4. Trust | M8 to M10 | Proven contracts with counterexamples, build report, release v0.1.0 | a false `ensures` gives U5001 with a counterexample; `una obligations`, `una report` |

The critical path to checkpoint 3 is M1, M3, M4, M5, M7.

### 3.2 Demo scripts

For each checkpoint whose last milestone is complete:
- add `scripts/demo/checkpoint-<n>.sh`, which prints each command, its expected outcome and its actual outcome;
- put the demo programs in `tests/demo/checkpoint-<n>/`, with at least one intentional error per diagnostic family the checkpoint introduces;
- add an end-to-end test that runs every existing demo script;
- add a "Checkpoint demo" section to the morning report.

For checkpoints already reached, write the scripts now.

### 3.3 Interim run behavior

Write the next free ADR, "Interim run behavior before tier 1", status Accepted, Deciders: Leonardo.

- From M7 until M8, `una run` and `una test` perform tier 0 checks before executing.
- In human mode they print one line to stderr: "note: proof obligations are not verified yet (tier 1 arrives in M8)".
- In `--json` mode their output carries `"tier": 0` and no note.
- M8 removes this behavior. Record that in the M8 plan.

### 3.4 Priority

Reaching checkpoint 3 with every acceptance criterion green matters more than starting M8. Stop on a red base, as `OVERNIGHT.md` C.7 says.

---

## Progress

Newest entries last. Reading notes on the instructions above:
- In B.2.1, "`007` �7`" is read as "`007` → `7`" (a character was lost in transmission).

| When | Milestone | State | Notes |
| --- | --- | --- | --- |
| 2026-09-29 | Part A | done | `c198b65` renamed Vera to Troth on `main`; CI run 36517282489 green on Linux and macOS |
| 2026-09-29 | Setup | done | branch `overnight/v0.1`, this file, decision log, morning report, ADR template status, CI trigger for the branch |
| 2026-09-29 | M1 | done | `2bafddd` to `045bafa`; CI run 36522873179 green on Linux and macOS; 154 tests; conformance 50 cases: 47 passed, 3 pending (M5, M6, M8); Provisional ADRs 0018 and 0019; decisions D-009 to D-022 |
| 2026-09-29 | M2 | done | `8a59109` to `77e4d5d`; CI run 36565528335 green on Linux and macOS; 178 tests; conformance 63 cases: 60 passed, 3 pending (M5, M6, M8); Provisional ADR 0020; decisions D-023 to D-031 |
| 2026-09-29 | M3 | done | `0e8030c` to `f98912a`; CI run 36568569555 green on Linux and macOS; 204 tests; conformance 92 cases: 89 passed, 3 pending (M5, M6, M8); Provisional ADRs 0021 to 0023; decisions D-032 to D-046 |
| 2026-09-29 | Consolidated instructions, Part 0 | done | clean state at `fc9301b`; the uncommitted M4 task 2 work was set aside, not committed (morning report §2); una-lang/una exists and is empty |
| 2026-09-29 | Part 1 | done | `1fa2523` renamed Troth to Una (ADR 0025, D-058); 204 tests; conformance 92 cases: 89 passed, 3 pending |
| 2026-09-29 | Part 2 | done | `9101398` neutral reference example `examples/events` (ADR 0026, D-059, D-060); domain audit empty |
| 2026-09-29 | Part 3 | done | `a3ac631` usage checkpoints (roadmap §4), checkpoint 1 demo and its end-to-end test, ADR 0027; 205 tests |
| 2026-09-29 | Part 4 | done | `580882e` Apache-2.0 license |
| 2026-09-29 | Part 5 | deferred, then done | first deferred by the owner; then, on the owner's correction, `25d865b` imported to una-lang/una (`main` and `dev/v0.1`), remotes `origin` and `legacy`, ADR 0030; CI quota rule D-083 |
| 2026-09-29 | Part 6 | done | `002b22d`, this file and the morning report updated; CI run 36574885457 green on Linux and macOS; M4 resumes |
| 2026-09-29 | M4 | done | `fc9301b`, `0c07912` to `a5c8d3d`; CI run 36578625540 green on Linux and macOS; 236 tests; conformance 116 cases: 113 passed, 3 pending (M5, M6, M8); ADR 0024 rules 1 to 15; decisions D-047 to D-057 and D-061 to D-070 |
| 2026-09-29 | M5 | done | plan and ADRs 0028, 0029 in the initial import `25d865b`; `9d34aab` to `79b7a2c` on `dev/v0.1`, across a restart of the session; CI run 36606305565 green on Linux (macOS by local checks, D-083); 259 tests; conformance 144 cases: 142 passed, 2 pending (M6, M8); ADR 0028 rules 1 to 12; decisions D-071 to D-082 and D-084 to D-087 |
| 2026-09-29 | M6 | done | `2348672` to `b37eaa2`; CI run 36610873278 green on Linux (macOS by local checks, D-083); 274 tests; conformance 160 cases: 159 passed, 1 pending (M8); ADR 0031 rules 1 to 10; decisions D-088 to D-097; usage checkpoint 2 reached, with its demo; an M5 fix (`906bd9e`) |
| 2026-09-29 | M7 | done | `6c648fb` to `511b4eb`; CI run 36618844273 green on Linux (macOS by local checks, D-083); 296 tests; conformance 173 cases: 172 passed, 1 pending (M8); ADR 0032 rules 1 to 11; decisions D-098 to D-110; usage checkpoint 3 reached, with its demo |
