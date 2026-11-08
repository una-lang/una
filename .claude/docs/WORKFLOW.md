# Workflow (v5 pilot, 2026-11-08; v4 below where v5 is silent)

Goal: a usable language first - a user installs `una` and writes, checks, runs, tests and builds Una programs with nothing else installed (no Rust, no separate Z3, no C toolchain). Capabilities grow edition by edition.

1. **Board first.** Every change starts as a card on the "Una roadmap" board in Ready (skill `board`), with its spec or ADR reference. No card, no branch.
2. **One card, one branch, one PR.** `dev/<issue>-<topic>` from `origin/main`, worktree under `.claude/worktrees/<issue>`; status In progress -> In review -> Done. Push often.
3. **Build gate per merge** (skill `merge`): build, workspace tests, clippy, fmt, on this Mac. Review (skill `review`) for code PRs, at most two rounds.
4. **Heavy evidence only at release** (skill `release`): sanitize, Miri, fuzzing, the hunt, timing oracles, Linux legs, benchmarks and parity measurements run on the release candidate, not per PR. Findings become cards.
5. **Only `main`** is long-lived (plus `release/v0.1`, `release/v0.3`). Branches and worktrees die at merge.
6. **Principles never bend:** no foreign code, security over schedule, the spec wins, accepted ADRs are amended not edited, English in every artifact, public-repo neutral text.
7. **State is written down** (skill `state`): STATE.md and NEXT.md after every merge or decision; DECISIONS.md for the project's decisions.
8. **Subagents:** `builder` implements, `reviewer` attacks, `mechanic` does procedures, `scout` reads. Spawn them with the card number and the files; never let one wait for input.
9. **Scout first.** Consultation (where is X, quote the rule, list the cases, dependencies of a card) goes to `scout` (haiku) before a builder, reviewer or planner spends context on it; independent lookups run as parallel scouts. A scout answer is a lead with `path:line`, never an approval; the reviewer re-reads the decisive lines. Subagents cannot spawn subagents, so a role that needs more lookups lists them under "Scout requests" and the main session runs them.
10. **Workspace harness** (`../AGENTS.md`, `../.agents/`): complements this process and never overrides it. Use it for evidence runs: `python3 .agents/scripts/harness.py start --card <N> --workflow <feature|bugfix|...> --author <actor>`, then `run --gate build|tests|verification|docs` and `status`; the evidence is bound to the source fingerprint of the repo. Board statuses are still written only through `board_set.py`. Role mapping: builder = implementer, reviewer = reviewer (adversarial), mechanic = mechanical procedures, scout = scout; planner, architect, verifier, tester, benchmarker and documenter are roles the main session plays or spawns from `../.agents/agents/`.

## Process v5 (owner-signed 2026-11-08, pilot, card #1768; `../ops/coordination/PROCESS-V5-PROPOSAL.md`)

Goal: a pull request takes 10 to 15 minutes from open to merge; a release ships when its minimum usable release (MUR) is met. Where v5 and the numbered rules above differ, v5 wins during the pilot.

- **Releases.** Each release has a MUR checklist (`../ops/coordination/releases/MUR-R<x.y>.md`) and a time box (default 7 days). Scope flexes, the date does not. Hardening (hunts, formal models, refinements beyond the MUR) is a release of its own.
- **Slots.** A builder works in a fixed worktree `.claude/worktrees/slot-N` with its own target (`../ops/slot.sh init|take|release`). Up to four slots.
- **PR check.** `../ops/pr-check.sh` in the slot: fmt, clippy and all tests of the changed packages plus the fast subset of `scripts/gate-tier.sh` (docs, corpus and test-only changes: the fast subset only), doc tests, changelog fragments, doc links. The exclusive timing tests, the long native suites (unless their crate changed) and the heavy evidence run in the nightly and at release validation. No merge train, no per-PR harness run, no evidence pack.
- **Merge.** `../ops/merge.sh <PR>`: refuses without a pass for the exact head, while `NIGHTLY-RED` exists (except the fix), or when main moved on files the PR changes; merges with `--merge`, reads back MERGED, moves the card to Done, deletes the branch, records the event in `../ops/flow/events.jsonl`. Board statuses: Ready, In progress, Done.
- **Review.** No per-PR review, except lane S (soundness, trusted computing base, security fixes, information-flow rules, the verifier's encoding): one adversarial review before merge, recorded in `../ops/coordination/reviews/pr<N>-<topic>.md` with the line `verdict: pass`. All other review happens at **release validation**: one adversarial reviewer per area on the cumulative diff of the release candidate, findings as cards labelled `rc-blocker` or `rc-defer`.
- **Decisions never block.** Proceed on the recommended default, note it in the PR (`Decision D-n (default)`), the owner looks back in batch.
- **State.** The board is the live state; STATE.md and NEXT.md are refreshed at release close and at session start.
- **Measure.** `../ops/tools/flow-report.py` prints PRs, open-to-merge minutes and tokens per day.
