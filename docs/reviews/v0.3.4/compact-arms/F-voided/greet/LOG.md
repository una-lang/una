# greet log

Design: `Env.get` returns `internal Text`, so the name cannot be printed directly (U4003). Followed the
"data from the environment, through a policy" pattern of `una explain U4003`: a trusted module `release`
declassifies only the name's length (`internal Text` -> `public Nat`), and `main` prints a line about it.

## Check run 1
- Diagnostics: none (exit 0).
- Consulted: docs: una-for-agents.md §9 and `una explain U4003` (read before writing, to avoid the known U4003).
- Enough: yes.
- Changed: first version (una.toml with [trusted] and [run] grants console, env; src/release.una; src/greet.una).

## una test
- Result: 1 example, 1 passed (exit 0). Consulted: nothing. Changed: nothing.

## una run
- Output: `Hello! Your user name has 10 letters.` (exit 0). Consulted: nothing. Changed: nothing.

DOC: README.md §Install mentions docs/guide/una-compact.md, but the archive's docs/guide holds no such file.
