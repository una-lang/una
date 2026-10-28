---
name: board
description: Create, assign and move cards on the "Una roadmap" GitHub project (board-first kanban). Use before starting any work and whenever its status changes.
---
The board is https://github.com/orgs/una-lang/projects/1 (project 1 of org una-lang). Fields: Status (Backlog, Needs spec, Ready, In progress, In review, Done), Priority (P0-P2), Agent (text), Milestone (GitHub milestone of the issue).

Process (kanban): every piece of work starts as an issue on the board **before** any code: `gh issue create --repo una-lang/una --title ... --body ... --label kind:... --milestone <v0.4|...>` then `python3 .claude/tools/board/board_set.py una#<N> Ready <P0|P1|P2> <agent>`. Move it to In progress when a branch exists, In review when the PR is open, Done after the merge (`python3 .claude/tools/board/set_done.py una#<N>` or board_set with Done). One issue per PR; the PR body says `Closes #N`.

Sweeps: `gh project item-list 1 --owner una-lang --limit 900 --format json` lists everything; keep Ready small (what the next week can take), park the rest in Backlog with a one-line reason. Never run `.claude/tools/board/sync.py` or other scripts "to see their usage": read them; some rewrite the board.
