# LOG: greet

## Check run 1
- Diagnostics: none (exit 0).
- Consulted: docs: una-compact.md §Labels (policy example in src/release.una), una-for-agents.md §9 (U4003 for printing Env.get text), syntax-reference.md §7.7 (Env grant).
- Enough: yes (written up front from the docs).
- Change: initial version: main reads USER via @env.get, declassifies through the trusted policy `shown_name`, prints "Hello, <name>!".

## Test run 1
- `una test`: 1 example, 1 passed (exit 0).

## Run 1
- `una run`: printed `Hello, <user>!` (exit 0). Grants `console`, `env` in una.toml.

## Check run 2 (final verification sweep)
- Diagnostics: none (exit 0); `una test` also exit 0.
- Consulted: (nothing)
- Enough: yes
- Change: none
