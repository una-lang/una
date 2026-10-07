# Cross-review of JSON and HTTP for v0.3.0: plan

- **Decision:** N-37. The Beta is v0.3.0, edition 0.2. Before its tag, no one reviews their own work.
- **Reviewer:** Claude (Una-1). **Authors under review:** Una-2: JSON (#238, ADR 0073, `spec/23`) and HTTP (#239, ADR 0074, `spec/24`).
- **Candidate:** `5495b61d65508fe1d26785f335d861799a58757f`, `main` after the HTTP pull request (#298).
- **Time-box:** one day. Rechecks cover the delta of each fix only.
- **Findings** go to the coordinator as they are found, each with its case, and are collected in `findings.md`.

## Severity

As in the v0.2.0 pre-tag review (`../../v0.2-review/pretag/summary.md`):

- **Critical:** unsound, a leak of labeled data, a native divergence the specification does not allow, a credential sent where `spec/24 §3` forbids it, or a failing release gate.
- **Major:** a diagnostic or repair that changes meaning, lost trace or exit parity, a complexity bound of `spec/23 §6` or `spec/24 §4` that does not hold, or a failing documentation example.
- **Minor:** everything else.

The verdict is Not ready while any Critical or Major is open.

## Scope

JSON and HTTP on their own, and above all where they meet Map (`spec/21`) and Text (`spec/22`):

- `spec/23`, ADR 0073 and its amendments; `spec/24`, ADR 0074 and its amendments;
- the prelude of edition 0.2 (`crates/una-check/src/std/prelude_0_2.una`), the shared implementation in `una-core` and the HTTP client, and both engines;
- the conformance cases, tests and examples that #238 and #239 added, and the documentation of both.

Out of scope: Map and Text by themselves, which their own cross-review covers, and edition 0.1, except for isolation (area 7).

## Areas

Each area lists what is checked and how. Every program of the review is a case under `cases/<area>/`, a package of edition 0.2 with its expected outcome, so that a finding can become a regression test unchanged.

### 1. HTTP responses parsed as JSON

- A program that gets a `Response` from a local test server and parses its body with `parse_json`, and works on the result as `Json`, `Map` and `Text`.
- Every outcome of the request: each status, a redirect followed and one returned, `Denied` (no grant, a method the grant does not allow, a redirect to an origin it does not allow), `TooLarge` at `--max-response`, `TimedOut` at `--http-timeout`, `Protocol`. Then each failure of the parse on top of a successful request: `Syntax`, `TooDeep`, `DuplicateKey`, `NumberTooLong`.
- Handling: `if error fail`, `if error pass`, `attempt` on each step, and the `ensures` of the callee after an `Ok`.
- Credentials: `key_name` and the program's `Authorization` and `Cookie` across redirects to other origins and back (`spec/24 §3` rule 2). The key never appears in a value, a trace, an outcome or a diagnostic.

### 2. JSON built from Map and Text

- `Object(fields: m)` from maps built with `edit` loops, `with` and `set`; keys and values from `split`, `slice`, `replace`, `join` and `find`.
- Text at the edges: non-ASCII scalar values, astral characters, `\u` escapes and surrogate pairs on input, control characters, `"` and `\` in keys, the empty key.
- The round trip of `spec/23 §4` rule 3 on every value the area builds, and the fixed point on every text area 1 parses; `Decimal` texts kept as written; key order by the map, not the input; equality.
- `quote_json` against `to_json(Str(value: t))` on the same texts.

### 3. Labels across the four

A matrix of sources and sinks, each cell a case with the expected diagnostic or its absence:

- **Sources:** a labeled `Text`, a labeled map key and value, a `Response` (`internal`, `spec/24 §5` rule 2), a parse of a labeled text, a failure of each (N-27, area 6).
- **Through:** `Json` constructors (U4001 for a labeled value, `spec/23 §5` rule 2), `to_json`, `quote_json`, interpolation, the text functions, map operations whose shape carries labels (`spec/21`).
- **Sinks:** `Console.print`, `Files.write`, and every argument of every `Http` operation (`accepts internal`: a `secret` URL, header, body or key name is U4003, `spec/24 §5` rule 1).
- **Implicit flows:** branching on whether a parse fails and where (`at`), on `m.has(key: k)` with a labeled `k`, on a status, and on `find`'s result; the program counter under `together`.
- **Declassification:** a policy that lowers a parsed value, and the build report entry for it.

### 4. Parity of the engines

- Every runnable case of areas 1 to 3, in the interpreter and as a native executable: output, exit status, failure and trace, under the rules of `spec/16 §5` and `spec/19 §8` rule 4.
- `Http` trace events (`spec/24 §6`): `instance`, `op`, and redaction of arguments and outcome by the binder's label, identical in both engines.
- Examples cannot use `Http` (U3014); handlers installed by `given` stand in, in both engines.

### 5. Performance

- The bounds of `spec/23 §6` rule 1 and `spec/24 §4` rule 3, measured, not assumed: `parse_json` and `to_json` on inputs from 1 KiB to 64 MiB, flat and nested to depth 512, with objects of many keys; a response body up to `--max-response`; JSON built from a map of n keys and written. Doubling n must double the time within the margin the repository's scaling tests use, in both engines.
- No native recursion over depth (`spec/23 §6` rule 2): depth 512, and 513 failing with `TooDeep`, on a small stack.
- The memory budget (`spec/11 §7`) stops a parse that would exceed it with U6003, not a crash.

### 6. Failure labels (N-27)

- The failure of `parse_json(t)` carries the labels of `t`, and so do `attempt`'s `Ok` and `Err`, the payload (`at`), and the code that handles it (`spec/23 §5` rule 1).
- An `HttpError` from a request whose arguments are labeled: what it carries, against `spec/24 §5` and N-27.
- The two composed: a request, then a parse of its body, then a failure that crosses both, through relays (`fails of`), with `attempt` at each level.

### 7. Edition 0.1 and names

- In edition 0.1, `Json`, `Http`, `Header`, `Response` and their variants are ordinary names, and nothing of either chapter is reachable.
- In edition 0.2, the names of `spec/23 §1` rule 2 and `spec/24 §1` rule 2: user kinds, variants and capabilities named like them, and `import std.host (Http, Header, Response, HttpError)` once #16's follow-up adds them (N-33).

### 8. Documentation

- Every code block of `spec/23` and `spec/24`, of the guides that mention them, and of the examples, checks and runs as it says.
- The CHANGELOG lines, the README and the website pages for both.

## Deliverables

- `findings.md`: each finding with its severity, the rule it breaks, the smallest case, and a suggested fix.
- `cases/`: the packages of the review, by area.
- `summary.md`: the verdict per area, as in the v0.2.0 pre-tag summary.
