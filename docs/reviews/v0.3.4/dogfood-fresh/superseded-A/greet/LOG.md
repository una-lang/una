# greet log

Consulted before writing: docs: una-for-agents.md §9 (printing what is read from Env takes a policy), §8 (host capabilities, grants), §2 (packages); getting-started.md §2 (una.toml, verification.lock).

## Check run 1
- Diagnostics: none (`{"schema":"una.diagnostics/1","diagnostics":[]}`), exit 0.
- Consulted: n/a (written up front from docs: una-for-agents.md §9 and syntax-reference.md §8.5 Declassification).
- Enough to fix: n/a.
- Change: first version: trusted module `reveal` with policy `login_name` (internal Text -> public Text), `main` reads USER with `if none use "stranger"`.

## Test run 1
- `una --json test`: 1 example, passed 1, failed 0, exit 0. No diagnostics.

## Run 1
- `una run`: printed `Hello, <user>! Your name has 10 letters.`, exit 0. No diagnostics.
