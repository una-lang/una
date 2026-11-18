# Summary

Toolchain: una 0.3.3 (editions 0.1, 0.2; Z3 5.1.0). All packages are edition 0.2.

| Task | `una check` runs | Passing state reached | Other runs |
| --- | --- | --- | --- |
| 1. greet | 1 | yes (run 1) | `una run` prints `Hello! Your user name has 10 characters.`; `una test`: 0 examples (none asked) |
| 2. orders | 1 | yes (run 1) | `una test`: 2/2 passed, including the exact input/output of the task |
| 3. stock | 3 | yes (run 2; run 3 after adding a stricter stand-in and a second example) | `una test`: 2/2 passed |
| 4. tally | 1 | yes (run 1) | `una test`: 3/3 passed; 3 obligations proven (`has` postconditions and a `has` precondition) |
| 5. sensors | 1 | yes (run 1) | `una test`: 4/4 passed |

Total: 7 `una check` runs, one failing (stock run 1: U0005, `\r` is not a text escape; fixed with `\u{D}` from the message and the escape list in una-for-agents.md §3). No task gave up.

Documentation notes (also in the task logs):
- stock: the `Http` operations `post`, `put` and `delete` have no documented parameter lists, and syntax-reference.md has no HTTP section (its host-capability table §7.7 lacks `Http`); the handler's parameter lists were guesses that the checker accepted.
- sensors: a one-field variant prints as `Offline("south")`, without the field name, while una-for-agents.md §4 shows printed records with field names; when names are shown is not stated.

What the docs made easy: the Env-to-console policy pattern (`una explain U4003`), the loop-forgets-facts rule with the `add_vote` helper pattern (§11), and the JSON and `together` sections were each enough to write a passing program on the first try.

Final confirmation: after all logs were written, one more plain `una check` of each of the five packages exited 0 (not counted in the table above, since nothing changed).
