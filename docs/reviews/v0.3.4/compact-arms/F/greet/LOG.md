# greet LOG

Consulted before writing: docs: una-for-agents.md §9 (U4003 example for Env.get USER), `una explain U4003` (fixed example with a `shown_name` policy in a trusted module), docs: getting-started.md §2 (package layout, verification.lock).

## Check run 1
- Diagnostics: none (exit 0).
- Consulted: n/a
- Enough: n/a
- Change: first version: trusted module `release` with policy `shown_name` (internal Text -> public Text), `main` reads USER via `@env.get` and prints a greeting.

## Test run 1
- `una test`: 0 examples, exit 0 (the task asks for no example).

## Run 1
- `una run`: printed `Hello, <user>! Welcome to Una.`, exit 0.

DOC: una-for-agents.md §9 says printing Env data "takes a policy" and points to `una explain U4003`; the full fixed program is only in `una explain U4003`, not in the guide itself. Clear enough once found.
