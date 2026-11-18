# greet: log

DOC: una-0.3.3/docs does not exist. The task and README.md point to docs/guide/getting-started.md, syntax-reference.md and una-for-agents.md, but the archive holds only README.md, LICENSE, lib/ and the binary. I used `una explain` for every code (U0001 to U8001) as the reference.
DOC: README.md "Install" says `una check` fails with U5003 without the pinned Z3; a z3 5.1.0 was already on PATH here, so that did not come up.

Before writing the code I read `una explain U4003` (its third example, "data from the environment, through a policy", is almost this task) and `una explain U7004` (for the verification.lock content).

## Run 1: una check
- Diagnostics: none (exit 0).
- Consulted: una explain U4003, una explain U7004, una explain U3012 (for `[run] grants`), before writing.
- Enough: yes.
- Changed: first version; main reads USER through `Env[get]`, declassifies it through a policy in trusted module `release`, prints one line.

## una run
- Output: `hello, <user>`, exit 0.

## una test
- 0 examples: 0 passed, 0 failed, exit 0 (the task asks for no example).
