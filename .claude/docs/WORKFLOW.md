# Workflow (v4, 2026-10-27)

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
