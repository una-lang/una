# In-place performance review, v0.3.0 Beta cross-review (N-37): plan

Reviewer: Una-3. Candidate: una `main` 5495b61. Read-only on una. Time-box: one day.

Scope:
- #261: text append in place;
- #262 / ADR 0072: arguments that move, borrow inference;
- #263: appends in `together` tasks;
- the debug runtime's copy counter.

## Builds

The candidate is exported with `git archive` into a scratch directory with its own target:
- `cargo build --release -p una-cli -p una-rt`;
- `cargo build -p una-rt` for the debug runtime, linked with `UNA_RT_LIB=<debug libuna_rt.a>`.

## Engines per case

| Engine | Observed |
| --- | --- |
| I, `una run` | stdout and exit code |
| I-json, `una --json run` | trace |
| N, release native with `--trace` | stdout, exit code, trace |
| D, native linked with the debug runtime | exit 3 on a use-after-free or a leak; the stderr note "copied: k" |

## Criteria

- **Semantics:** each case prints values whose correct result follows from the value semantics of spec/17. A mutation seen through another name, or a failed call that changes what the caller sees, is a finding.
- **Memory safety:** D exits with the same code as N, never 3, and has no fault and no leak.
- **Parity:** I and N give the same stdout, exit code and trace, byte for byte.
- **Copies:** for shapes the ADR says update in place, D reports 0 copies, or a count that does not grow with n. Aliased shapes must copy.
- **Scaling:** time at 3 sizes, interpreter and native. A shape that grows faster than linear and that the ADR covers is a finding. Shapes the ADR defers (closures, handlers, nested handled blocks) are recorded as known.

## Case families

1. **Aliases:** a `let` alias, a record field, a list element, a closure capture, a `when` scrutinee, a loop's list, a map value, the same value passed twice, and self-append (`t = "{t}{t}"`).
2. **Failure paths:** an `edit` argument or a binding read by `if error use` or after `attempt`; a callee that appends and then fails; failures 1 to 3 call levels deep.
3. **Borrow inference:** recursion, mutual recursion, function values and higher-order calls, generics with a record or a nested list as T, closures, handlers, and a loop state.
4. **`together`:** tasks that copy a captured value and append to it; `together for` building texts and lists; the outer binding stays unchanged.
5. **Scaling:** append through 0 to 3 call levels (`edit`, and the returned list); text through `edit` helpers; recursion; generic helpers; inside tasks; through a function value, which the ADR defers.
