---
name: state
description: Refresh the project state and plan files at the start and end of a session so the next session resumes without re-deriving anything.
---
Read `.claude/state/STATE.md` and `.claude/plan/NEXT.md` at session start (then `git fetch --prune` and `gh pr list --state open`). At session end, or after any merge, release or decision: rewrite STATE.md (main hash, open PRs with their state, release lines, blockers, machine notes) and NEXT.md (the ordered next cards, each with its issue number); commit both with "state: <date>" directly on main. Project decisions go to `.claude/docs/DECISIONS.md` (date, decision, why). Personal memory stays in the Claude memory directory, never in the repo.
