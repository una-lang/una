# greet LOG

## Check run 1
- U0101: unexpected `,`, expected end of line
- U7004: the package has no verification.lock; add one next to una.toml
- Consulted: una explain U7004; una explain U0101
- Enough: partly (U7004 yes, explain gives the lock content; U0101 explain does not cover several `uses`)
- Change: added verification.lock from the U7004 explain; split `uses` into one line per capability (guess to try).
- DOC: una-compact.md §Capabilities does not show how to declare more than one capability in a header.
- DOC: una-compact.md §Package mentions verification.lock but not its content; only `una explain U7004` shows it.

## Check run 2
- U1001: cannot find name `release` in this module, its imports or the prelude
- Consulted: una explain U1001; una explain U4003 (its second fixed example shows `import release (shown_name)` and calling the policy by name)
- Enough: yes
- Change: added `import release (shown)` and called the policy `shown(name)` instead of `release.copy(name)`.
- Note: one `uses` line per capability was accepted (run 1 U0101 fixed).
- DOC: una-compact.md §Labels shows the policy declaration but not how to import and call it (by policy name, not via function).

## Check run 3
- No diagnostics; exit 0.
- Consulted: n/a
- Enough: yes
- Change: none (passing). Also prints a value derived from the name (its length).

## una test
- `0 examples: 0 passed, 0 failed`, exit 0 (the task asks for no examples).

## una run
- Printed `Hello, <user>! Your user name has <n> letters.`, exit 0. Grants come from `[run] grants = ["console", "env"]`.
