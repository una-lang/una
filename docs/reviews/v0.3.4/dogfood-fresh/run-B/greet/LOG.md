# greet log

DOC: The task points to `una-0.3.3/docs`, and README links `docs/guide/getting-started.md`, `docs/guide/syntax-reference.md` and `docs/guide/una-for-agents.md` ("each archive ... also holds the guides under docs/guide/"), but the archive has no `docs/` directory at all. Only README.md, `una --help` and `una explain <code>` are available. I enumerated every code `una explain` knows (U0001..U8001, 83 codes) and read them as the documentation.
DOC: No place documents the prelude/standard library signatures (Map methods, JSON, Http, together). The prelude source is only visible as embedded strings in the `una` binary (`strings una`); I used that as a reference for signatures.

Before writing: consulted `una explain U4003` (§"Fixed example: data from the environment, through a policy") and `una explain U3012` (grants in una.toml), since Env.get returns `internal` text that Console cannot accept.

## Check run 1
- Diagnostics: none (exit 0).
- Consulted: una explain U4003, una explain U3012 (before writing).
- Enough: yes.
- Changed: initial version (main reads USER via Env.get, declassifies through trusted policy `release.shown_name`, prints greeting with name and length).

## Test run 1
- `una test`: 0 examples, 0 passed, 0 failed, exit 0 (the task asks for no examples here).

## Run 1
- `una run`: printed `hello, <user>, you have 10 letters in your name`, exit 0.
- Diagnostics: none.
