# Project decisions

- 2026-10-27: one session with subagents replaces the multi-instance setup; everything merges into main; worktrees and branches are deleted after merge; heavy tests only at release; board-first kanban; the goal is a usable language with no Rust needed by users (ADR 0124's one binary is the path).
- 2026-10-27: ADR 0124 accepted (probes P1/P2/P6 as pre-implementation conditions); OS calls and safe assembly allowed, embedded C never.
- 2026-10-27: v0.3.17 tags once every gate passes.
