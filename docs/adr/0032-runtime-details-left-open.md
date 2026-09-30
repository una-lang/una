# 0032. Runtime details the specification leaves open

- Status: Accepted
- Date: 2026-09-29
- Deciders: Leonardo (accepted on 2026-09-29), on a proposal by Claude (overnight run)

## Context

M7 elaborates checked programs to Core IR (`spec/09`), runs them on the Una Abstract Machine (`spec/11`) with the host capabilities of `spec/14 §3`, and adds `una run` and `una test` (`spec/13 §5`). The specification defines Core IR, the evaluation rules and the traces, but not how elaboration obtains the checker's resolutions, the text of `--dump-core`, how values appear in traces and outputs, the JSON of `una run` and `una test`, how grants combine and where their paths resolve, how `Files` normalizes paths, the format of the `Keys` key file, a few semantics of the prelude, or what the call depth counts.

Decision log entries: D-098 to D-109 (`docs/history/v0.1/open-decisions.md`).

## Decision

1. **Elaboration.** Pass 2 of the type checker records its resolutions by the span of each expression: callees and their type arguments, the parameter of each argument, fields, operations, variants, capability bindings, and the labeled type of each expression and binder. The elaborator applies `spec/09 §3` to the syntax tree with these records. Only declarations without diagnostics are elaborated. `una-core` depends on `una-syntax` for spans. A Core `let` binds a right-hand side or an expression, so the value of a handled statement or of a branching construct can be bound; a `when` over literals or ranges without `else` takes its last arm as the final branch, whose condition the exhaustiveness obligation proves.
2. **The Core dump.** `una check --dump-core <module> [path]` prints, on stdout, the Core IR of the module's declarations in the text syntax of `spec/09 §2`, with every binder's type, label and refinements; the diagnostics of the package still go to stderr, and the exit code follows them.
3. **Values in traces and outputs.** A value is a JSON string holding its Una form: `"42"`, `"\"Ana\""`, `"true"`, `"unit"`, `"Open"`, `"Some(3)"`, `"Registration(name: \"Ana\", event: ...)"`, `"[1, 2]"`; a `Hashed` value is `"hashed:<hex>"`, and a function `"<function>"`. An argument whose static label is above `public` is `"<redacted:level>"`, with the label's level only. Inside any rendered value, a field whose declared type is labeled above `public` is `<redacted:level>`; an operation's outcome is redacted by the label of the binder of its result; `unit` carries no information and is never redacted. An event's outcome is `{"kind":"ok","value":...}`, `{"kind":"fail","variant":...,"value":...}` or `{"kind":"aborted"}`.
4. **`una run`.** In human mode the program's printed lines go to stdout as it prints them, and U6001 and U6003 to stderr as diagnostics, with exit code 1. U6001 points at `main`'s name and names the variant, with the payload fields whose labels are `public`; U6003 points at the call that exceeds the depth. `--json` prints `{"schema":"una.run/1","tier":0,"status":...,"output":[...],"failure":...,"diagnostics":[...],"trace":[...]}` with keys in that order; `status` is `ok`, `failed` (U6001), `aborted` (U6003, or U8001 by rule 11) or `rejected` (the checks reported errors, which `diagnostics` holds, and nothing ran); `failure` is `null` or `{"variant":...,"fields":{...}}` with the `public` fields only.
5. **`una test`.** In human mode, one line per example on stdout, `example <module> "<name>" ... ok` or `... FAILED`, followed for a failed example by the file, line and text of the expectation that does not hold, when one is to blame, and the reason; then a blank line and `<n> examples: <p> passed, <f> failed`. Exit code 1 when an example fails. `--json` prints `{"schema":"una.test/1","tier":0,"diagnostics":[...],"examples":[...],"passed":p,"failed":f}`; `diagnostics` holds the errors of the checks, when nothing ran, or U6003 and U8001 of aborted examples; each example has `module`, `name`, `outcome` (`pass` or `fail`), `reason` (`null`, or why it fails), `expectation` (`null`, or `{"file":...,"line":...,"text":...}`), `result` (`{"kind":"ok","value":...}`, `{"kind":"fail","variant":...,"value":...}` or `{"kind":"aborted"}`) and `trace`.
6. **Grants.** The grants of `[run] grants` and of `--grant` flags add up. Relative paths of `una.toml` grants resolve against the package root, and those of flags against the current directory. A `Files` path is relative to the grant's prefix: it is normalized lexically, its existing part is resolved through the file system, and a path that ends outside the prefix, absolute, through `..` or through a link, fails with `FileError.Denied`, as does an operation that the grant's mode does not allow. The key file of `keys:<file>` is a TOML table of key names to base64 keys, loaded before the run starts; the first grant that names a key wins, and an unknown name fails with `KeyError.UnknownKey`. The binary, not the library, reads environment variables for `Env.get` (`compiler/conventions.md §1`).
7. **The prelude.** `.len` of `Text` counts Unicode scalar values; `is_digits` holds when every character is an ASCII digit, so it holds of the empty text; `trim` removes Unicode whitespace at both ends; `to_int` reads an optional `-` followed by ASCII digits; `starts_with` compares scalar values; `contains` and `==` are structural; `Text` orders by scalar values.
8. **Depth.** The call depth counts the calls of functions, predicates, lambdas, policies and handler operations in progress. The evaluator runs on a thread whose stack is sized from the maximum depth, and aborts with U6003 when that stack runs short before the depth does, so it never overflows.
9. **Examples.** The expectations of an example are checked in order, and the first that does not hold fails it; then a `do` line that failed with no `expect fails` line fails it too. An `expect` line that does not mention `result` runs even when `do` failed. `expect performed @x.op` holds when the trace has a completed or failed `op` event of the instance of `given @x`, and `expect not performed @x.op` when it has none. An example whose run aborts fails.
10. **Entry and startup.** The entry of `una run` is `[run] entry`, or the module of a single file. `una run` checks the package with the grants of `[run] grants` and `--grant` together as final grants, even when there are none, so U3012 reports before execution every `uses` of `main` that they do not grant. A package without `[run] entry`, an entry without `pub main() -> Unit`, a `--grant` value that is not a grant spec, an unreadable or invalid key file, and a `--max-depth` whose stack cannot be allocated are usage errors (exit code 2).
11. **Faults.** A fault that a checked and verified program cannot reach, such as a division by zero while obligations are not verified yet (ADR 0027), aborts the run with U8001 and exit code 3, at the name of the function in progress, or at 1:1 of the package root (ADR 0022).

### Changes to other documents on acceptance (made on 2026-09-29)

- `spec/09 §3` states the Core `let` and `when` of rule 1; `spec/09 §4` rule 2; `spec/11 §5` rule 3; `spec/13 §5` rules 4 to 6, rule 10 and the two schemas; `spec/14 §1` rule 7; `spec/11 §7` rule 8; `spec/08 §2` rule 9; `spec/11 §4` rule 11.

## Consequences

- Elaboration never repeats the checker's resolution rules, so the two cannot disagree.
- Agents read `una run` and `una test` as JSON, with traces whose values they can parse as Una literals.
- `Files` grants hold against `..` and links, at the cost of resolving paths through the file system.
- Traces never show labeled data, even inside a public record; they show less than the example of `spec/11 §5` suggests when results are labeled.
- A division by zero before M8 is reported as an internal error, which it becomes once tier 1 runs first.

## Alternatives considered

- **An elaborator that resolves names and types again:** would duplicate the resolution of fields, methods and generic instantiations, and could drift from the checker.
- **Values as JSON objects:** a second representation of Una values for agents to learn, while the Una form is already defined.
- **Lexical `Files` confinement only:** a link inside the prefix could lead outside it.
