# 0032. Runtime details the specification leaves open

- Status: Provisional (implemented, pending human review)
- Date: 2026-09-29
- Deciders: Claude (overnight run), pending review by Leonardo

## Context

M7 elaborates checked programs to Core IR (`spec/09`), runs them on the Una Abstract Machine (`spec/11`) with the host capabilities of `spec/14 §3`, and adds `una run` and `una test` (`spec/13 §5`). The specification defines Core IR, the evaluation rules and the traces, but not how elaboration obtains the checker's resolutions, the text of `--dump-core`, how values appear in traces and outputs, the JSON of `una run` and `una test`, how grants combine and where their paths resolve, how `Files` normalizes paths, the format of the `Keys` key file, a few semantics of the prelude, or what the call depth counts.

Decision log entries: D-098 to D-105 (`docs/reviews/open-decisions.md`).

## Decision

1. **Elaboration.** Pass 2 of the type checker records its resolutions by the span of each expression: callees and their type arguments, the parameter of each argument, fields, operations, variants, capability bindings, and the labeled type of each expression and binder. The elaborator applies `spec/09 §3` to the syntax tree with these records. Only declarations without diagnostics are elaborated. `una-core` depends on `una-syntax` for spans.
2. **The Core dump.** `una check --dump-core <module> [path]` prints, on stdout, the Core IR of the module's declarations in the text syntax of `spec/09 §2`, with every binder's type, label and refinements; the diagnostics of the package still go to stderr, and the exit code follows them.
3. **Values in traces and outputs.** A value is a JSON string holding its Una form: `"42"`, `"\"Ana\""`, `"true"`, `"unit"`, `"Open"`, `"Some(3)"`, `"Registration(name: \"Ana\", event: ...)"`, `"[1, 2]"`; a `Hashed` value is `"hashed:<hex>"`, and a function `"<function>"`. An argument whose static label is above `public` is `"<redacted:level>"`.
4. **`una run`.** In human mode the program's printed lines go to stdout, and U6001 and U6003 to stderr as diagnostics, with exit code 1. U6001 points at `main`'s name and names the variant, with the payload fields whose labels are `public`; U6003 points at the call that exceeds the depth. `--json` prints `{"schema":"una.run/1","tier":0,"status":"ok"|"failed"|"aborted","output":[...],"failure":...,"diagnostics":[...],"trace":[...]}` with keys in that order; `failure` is `null` or `{"variant":...,"fields":{...}}`.
5. **`una test`.** In human mode, one line per example, `ok` or `FAILED`, with the failed expectation and the failure or result, then a summary; exit code 1 when an example fails. `--json` prints `{"schema":"una.test/1","tier":0,"examples":[...],"passed":n,"failed":m}`; each example has `module`, `name`, `outcome` (`pass` or `fail`), `expectation` (`null`, or the line and text of the first failed one), `result` (`{"kind":"ok","value":...}` or `{"kind":"fail","variant":...}`) and `trace`.
6. **Grants.** The grants of `[run] grants` and of `--grant` flags add up. Relative paths of `una.toml` grants resolve against the package root, and those of flags against the current directory. A `Files` path is relative to the grant's prefix: it is normalized lexically, its existing part is resolved through the file system, and a path that ends outside the prefix, absolute, through `..` or through a link, fails with `FileError.Denied`, as does an operation that the grant's mode does not allow. The key file of `keys:<file>` is a TOML table of key names to base64 keys; an unknown name fails with `KeyError.UnknownKey`.
7. **The prelude.** `.len` of `Text` counts Unicode scalar values; `is_digits` holds when every character is an ASCII digit, so it holds of the empty text; `trim` removes Unicode whitespace at both ends; `to_int` reads an optional `-` followed by ASCII digits; `starts_with` compares scalar values; `contains` and `==` are structural; `Text` orders by scalar values.
8. **Depth.** The call depth counts the calls of functions, predicates, lambdas, policies and handler operations in progress; the evaluator runs on a thread whose stack holds the default limit of 10,000.

### Changes to other documents on acceptance

- `spec/09 §4` states rule 2; `spec/11 §5` rule 3; `spec/13 §5` rules 4 to 6 and the two schemas; `spec/14 §1` rule 7; `spec/11 §7` rule 8.

## Consequences

- Elaboration never repeats the checker's resolution rules, so the two cannot disagree.
- Agents read `una run` and `una test` as JSON, with traces whose values they can parse as Una literals.
- `Files` grants hold against `..` and links, at the cost of resolving paths through the file system.

## Alternatives considered

- **An elaborator that resolves names and types again:** would duplicate the resolution of fields, methods and generic instantiations, and could drift from the checker.
- **Values as JSON objects:** a second representation of Una values for agents to learn, while the Una form is already defined.
- **Lexical `Files` confinement only:** a link inside the prefix could lead outside it.
