# The agent harness of this repository

One session works on Una, spawning subagents; there are no parallel "Una-N" sessions. Everything an agent needs is under `.claude/`:

| Path | What |
| --- | --- |
| `../CLAUDE.md` | The standing instructions: source of truth, scope guard, how to work, principles. |
| `agents/` | Subagent definitions with the model each one runs on: `builder` (opus), `reviewer` (opus, read-only), `mechanic` (sonnet), `scout` (haiku). |
| `skills/` | Procedures: `board`, `merge`, `review`, `release`, `state`. |
| `docs/` | `WORKFLOW.md` (kanban and gates), `RELEASE.md` (pointer), `DECISIONS.md` (decisions log). |
| `plan/NEXT.md` | The ordered next cards. |
| `state/STATE.md` | Where things stand (main, PRs, release lines, blockers). |
| `tools/board/` | Scripts for the GitHub project board (`board_set.py`, `set_done.py`, `create.py`, `issues.py`, `sync.py`). |
| `../.mcp.json` | MCP servers: GitHub (remote, OAuth on first use). `gh` remains the default interface. |

Project documents the harness points to: `docs/00-charter.md` (principles), `docs/spec/` (normative), `docs/adr/` (decisions; accepted text is immutable, amend by dated sections), `docs/compiler/` (architecture, testing), `docs/plan/` (roadmap, backlog, design notes), `docs/security/` (advisories, TCB), `docs/release-gates.md`.

Operational files that are not part of the language live outside the repository in `/Users/leocardoso/Projects/una-lang/ops/` (coordination logs, gate outputs, local install mirror).
