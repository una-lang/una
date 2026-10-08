# greet log

Prepared from: README.md, docs/guide/getting-started.md, docs/guide/una-for-agents.md §9 (Env text is `internal`, printing it needs a policy), `una explain U4003` (read before writing, for the Env/policy pattern). Wrote a trusted module `release` with a policy that lowers the name's length to public, so the name itself is never printed.

## Run 1 — `una fmt` + `una --json check`
- Diagnostics: none (exit 0).
- Consulted: n/a (no diagnostic).
- Enough to fix: n/a.
- Changed: nothing; first version passed.

## `una run` (after run 1)
- Output: `Hello! Your user name has 10 characters.` (exit 0).
- Diagnostics: none.

## `una test` (after run 1)
- `0 examples: 0 passed, 0 failed` (exit 0). The task asks for no example here.
