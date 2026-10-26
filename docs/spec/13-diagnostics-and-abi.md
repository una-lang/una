# 13. Diagnostics, Agent ABI and project files

Status: Normative, v0.1. The JSON shapes in this file are the Agent ABI, schema version 1: agents consume them, so changes follow the ADR process and bump the version in the `schema` strings. Schema versions freeze at the `v0.1.0` tag: before it, the shapes of version 1 may still change. After it, versions follow ADR 0076. A key added to an object, at any position, keeps the version: consumers MUST ignore unknown keys and MUST NOT depend on the position of a key, and the order this file gives is the printed order. Removing or renaming a key, changing its type, or changing what a key or a value means bumps the version. Fields whose values come from a set of strings are closed, and adding a value to one bumps the version, except two open fields: the diagnostic `code` (§4), whose new codes have a `domain` and a `severity` from their closed sets, and the obligation `kind` (`spec/12 §3`), whose new kinds use the existing `status` values. Consumers MUST accept unknown values of these two fields, and decide what to do from `severity`, `domain` and `status`. (ADR 0095.) The `kind` of a repair's `escape` (§3) is open too: a consumer that meets an unknown kind still knows from `escape_cost` that the repair is an escape. (ADR 0097.) So is the `kind` of a repair's `widens` (§3): the key's presence says the repair widens.

## 1. Diagnostic model

Every problem the toolchain reports is a diagnostic with a registered code (§4). Human output (default) and JSON output (`--json`) are two renderings of the same value.

```json
{
  "schema": "una.diagnostic/1",
  "code": "U5001",
  "severity": "error",
  "domain": "verification",
  "message": "postcondition of availability is refuted",
  "primary": { "file": "src/events/seating.una", "start": { "line": 27, "col": 18 }, "end": { "line": 27, "col": 21 } },
  "related": [
    { "file": "src/events/seating.una", "start": { "line": 23, "col": 11 }, "end": { "line": 23, "col": 52 }, "message": "ensures clause" }
  ],
  "obligation": { "id": "ob-3f09a1c2e7b4", "kind": "postcondition", "goal": "e.remaining < 1 implies result is SoldOut", "status": "refuted" },
  "counterexample": { "bindings": [ { "name": "e.remaining", "value": 0 }, { "name": "result", "value": "Few" } ] },
  "repairs": [
    { "rank": 1, "escape_cost": 0, "description": "add the arm 'in 0..0 -> SoldOut' before 'in 0..<20' or adjust the ensures clause", "edit": null }
  ],
  "explain": "una explain U5001"
}
```

| Field | Rule |
| --- | --- |
| `severity` | `error`, `warning` or `note`. v0.1 defines errors only. |
| `domain` | `syntax`, `names`, `types`, `authority`, `security`, `verification`, `runtime`, `project`, `internal` |
| `primary` | Required. A diagnostic about something that has no position in any file is located at the empty range at line 1, column 1 of the file that declares what is checked: the package's `una.toml`, or the checked file itself for a package of one file (ADR 0022). A missing `verification.lock` is thus U7004 at `una.toml` 1:1 |
| `file` | Relative to the package root, with `/` separators. A file checked on its own, such as a single-module conformance case, has its directory as package root, so `file` is its bare name (ADR 0015) |
| Positions | 1-based line, 1-based column counted in Unicode scalar values after decoding; `end` is exclusive. Each maximal ill-formed subsequence of invalid UTF-8 counts as one column (Unicode §3.9); a byte order mark and a tab count as one column each (ADR 0015) |
| `obligation`, `counterexample` | Present only for U5001 and U5002 |
| Counterexample binding | `{ "name", "value" }`, and (v0.2, ADR 0078) `"label"`, as `"secret"` or `"secret[PII]"`, when the label of the name's declared type is above `public` (`spec/11 §5`). The key is omitted for a public binding; it is additive, and the schema stays `una.diagnostic/1`. The human output prints the bindings without their labels |
| Counterexample `unavailable` | (ADR 0101.) Present only when the solver refuted the obligation but gave no values for its model: the solver's message, and `bindings` is then empty. The obligation stays refuted. The key is omitted otherwise; it is additive, and the schema stays `una.diagnostic/1`. The human output prints `counterexample: not available (the solver gave no values: <message>)` |
| `repairs` | Zero or more candidates, sorted by `escape_cost`, then `rank`. (ADR 0095.) `rank` is the position in that order, 1 to n without gaps. (ADR 0097.) Among repairs of equal `escape_cost`, those that widen come after the others |
| `escape` | (ADR 0095.) Present exactly when `escape_cost` is above 0, after `escape_cost`: `{ "kind", "cost" }`, where `kind` is `checked`, `escape_budget` or `trusted` (§3) and `cost` is one sentence that says what the guarantee loses. It is additive, and the schema stays `una.diagnostic/1`. The human output prints a repair as `repair N: ...`, or `repair N (escape: <kind>; <cost>): ...` |
| `widens` | (ADR 0097.) Present exactly when the repair widens, after `escape` when both are present: `{ "kind", "cost" }`, where `kind` is `authority`, `visibility` or `grant` (§3) and `cost` is one sentence that says what the widening costs. A widening keeps `escape_cost` 0. It is additive, and the schema stays `una.diagnostic/1`. The human output prints such a repair as `repair N (widens <kind>: <cost>): ...` |
| `edit` | `null`, or `{ "file", "start", "end", "replacement" }` when the repair is mechanical |

Diagnostics are sorted by file, start position, then code. Output is deterministic.

With `--json`, `una check` and `una fmt --check` print their diagnostics as one JSON object on stdout, followed by a newline (ADR 0012):

```json
{ "schema": "una.diagnostics/1", "diagnostics": [], "fixable": 0 }
```

`diagnostics` holds the diagnostics in the order above, each with its own `"schema": "una.diagnostic/1"` field; a new version of the diagnostic shape also bumps the list version. The array is empty when there are no diagnostics, and exit codes do not depend on `--json` (§5). Whitespace is not significant; the reference implementation prints compact JSON with object keys in the order shown in this section.

(ADR 0096.) `fixable` counts the diagnostics whose first repair `una fix` applies (§5): its `edit` is not `null`, its `escape_cost` is 0, it widens nothing, and it edits a module of the package. A U7001 with an edit counts too, since `una fix`'s format step applies it. The count is of the first round only: a file that parses after it reports its later phases. The key was added without a version change (ADR 0076 rule 1). In human mode, when the count is above 0, the last line on stderr is ``<n> of <m> diagnostics have an edit that `una fix` applies (`una fix --dry-run` lists them)``.

## 2. Obligation listing

`una obligations --json` returns every obligation, proven or not:

```json
{
  "schema": "una.obligations/1",
  "obligations": [
    {
      "id": "ob-3f09a1c2e7b4",
      "kind": "postcondition",
      "domain": "semantic",
      "status": "proven",
      "source": { "file": "src/events/seating.una", "start": { "line": 23, "col": 11 }, "end": { "line": 23, "col": 52 } },
      "declaration": "events.seating.availability",
      "goal": "e.remaining < 1 implies result is SoldOut",
      "hypotheses": ["0 <= e.remaining", "e.remaining <= 5000"],
      "cached": true
    }
  ]
}
```

`source` is the declaring clause for a return obligation, and the site otherwise. `hypotheses` are the refinements of record fields that the query assumes, then the path context at the site, or at the function's entry for a return obligation, printed in the surface syntax. `domain` is `semantic` for every obligation of v0.1 (ADR 0033).

## 3. Repair candidates

For U5001 and U5002 the compiler proposes, when applicable, in this order:

| Candidate | `escape_cost` |
| --- | --- |
| Strengthen a parameter or field type to a refined kind that implies the goal | 0 |
| Add a `requires` clause equal to the missing hypothesis (only for non-`pub` functions or with a note that callers change) | 0 |
| Add a guard (`if` or `when` arm) that handles the failing case | 0 |
| Replace the value with `checked(value, goal) else fail <Variant>` | 1 |

By kind (ADR 0033): a refuted or unknown `refinement` or `precondition` proposes, in order, a stronger parameter type when the value is a parameter, a `requires` clause for a private function, a guard, and `checked`; a `postcondition` proposes a guard or a changed `ensures` clause; a `division` a guard; an `exhaustive` an `else` arm. These repairs have no mechanical edit (`edit` is `null`).

A `refinement` obligation of a function value flowing into a function type (`spec/12 §3`, ADR 0037) proposes only a stronger type at the position of a function type: for a parameter, of the expected function type ("strengthen parameter 1 of `fn(Int) -> Int` to a refined kind that implies `x > 0`"); for the result, of the value's function type ("strengthen the result of `fn(Int) -> Int` to a refined kind that implies `result > 0`"). No `requires` clause, guard or `checked` is proposed, since none of them reaches a value that exists only when the function is called. The result obligation of a lambda, whose result type is not written, proposes a guard in the lambda's body.

(ADR 0095.) **Escapes rank last.** A repair is an escape when it takes one of these routes: `checked`, where a runtime test replaces a static proof (`spec/06 §6`); `escape_budget`, which raises `[escape_budget] checked_per_module`; or `trusted`, which declares or lists a trusted module or declassifies through a policy (`spec/07 §5, §6`). An escape has `escape_cost` 1 and carries the `escape` key (§1); every other repair has `escape_cost` 0. A repair is one route: it never joins a fix that keeps the guarantee and an escape. Every escape comes after every repair that keeps the guarantee, so when one exists, rank 1 is never an escape. A message states the problem; the routes out of it, escapes included, are repairs. In particular:

| Code | Repairs that keep the guarantee | Escape, last |
| --- | --- | --- |
| U7003 | state the fact in a refined type or a `requires` clause and remove the `checked`; handle the failing values with a guard and remove the `checked` | raise `checked_per_module` |
| U2016 | in code, test with a function, not a predicate, with the predicate's body, when that body calls no other opaque predicate and the predicate belongs to the package | `checked` |
| U4003 | send only data, or interpolate only fields, at the accepted label, or send the value to an operation that accepts its label | declassify through a policy |
| U4004 | move the policy and its `via` function into a module that is already trusted, when one exists | declare this module trusted in both places |
| U7005 | remove `trusted` from the header, or the entry from `[trusted] modules`; when the module declares a policy, the repair says that its policies are then U4004 | complete the declaration in the other place |

(ADR 0097.) **Widenings rank after the repairs that keep authority.** A repair widens when it adds operations to a `uses` clause or declares one (`authority`), makes a declaration `pub` (`visibility`), or grants a host capability's operations in `[run] grants` or with `--grant` (`grant`). A widening is fully checked, so it is not an escape and keeps `escape_cost` 0; it carries the `widens` key (§1), and output ranks it after every repair of equal `escape_cost` that keeps authority. `una fix` never applies a widening (§5, ADR 0084). A message states the problem, and the routes out of it are repairs:

| Code | Repairs that keep authority | Widening, after them |
| --- | --- | --- |
| U3002 at an operation | use another binding in scope that permits the operation | add the operation to the `uses` clause |
| U3002 at a call | rebind with `with @need = @other`, when it applies | add the operations to the `uses` clause |
| U3001, U3003, U3004 | none | declare the binding with `uses` |
| U1004 | none | declare the declaration `pub` |
| U3012, host capability | none | grant the operations in `[run] grants` |
| U3012, other capability | install a handler with `with` inside `main` | none |

Other diagnostics MAY include repairs (for example, a missing label with its mechanical edit). `una check` and the other commands never apply repairs. (v0.2, ADR 0084.) `una fix` applies mechanical ones on request (§5). A repair whose edit widens authority or visibility, by adding operations to a `uses` clause, declaring a `uses` clause or making a declaration `pub`, is marked so where it is built, and `una fix` never applies it.

(ADR 0096.) A syntax error (U0001, U0101) whose source has the shape of another language's form stays an error, since Una has one syntax (charter P4). Its first repair names Una's form, and carries an exact edit when rewriting is exact. Exact means four things: the edit changes only the declaration the error is reported in; once it is applied, the file has no syntax error in or before the edited range; the result is the stated reading in Una's form, token for token; and the edit is deterministic. The shapes and the condition of each edit are the table of ADR 0096 rule 3. Among them are an `if … then … else` wrapped across lines, `=` at the end of a function header, a name that starts with `_`, a `when` with its arms on one line, and a guard without `else`. A rename takes a name that is visible nowhere it would be written (ADR 0096 rule 3a).

## 4. Code registry

(v0.2, ADR 0056.) The codes U2017 to U2021, of the memory model (`spec/17 §10`), are in the table. (v0.2, ADR 0057.) U2022, of fixed-width integers (`spec/18 §8`), is in the table. (v0.2, ADR 0065.) U2023 and U2024, of structured concurrency (`spec/19 §10`), are in the table. (v0.2, ADR 0066.) U2025 to U2027, of constraints on generics (`spec/20 §10`), are in the table.

| Code | Domain | Meaning |
| --- | --- | --- |
| U0001 | syntax | Invalid character, byte order mark or lone carriage return |
| U0002 | syntax | Tab in indentation |
| U0003 | syntax | Inconsistent indentation |
| U0004 | syntax | Unterminated text literal |
| U0005 | syntax | Invalid escape sequence |
| U0006 | syntax | Text literal inside interpolation |
| U0007 | syntax | Malformed literal |
| U0101 | syntax | Unexpected token |
| U0102 | syntax | Clause out of order or repeated |
| U0103 | syntax | Reserved keyword not available in v0.1 |
| U1001 | names | Unresolved name |
| U1002 | names | Duplicate definition |
| U1003 | names | Import cycle |
| U1004 | names | Use of a private declaration from another module |
| U1005 | names | Naming convention violation |
| U1006 | names | Undeclared requirement in `derived from` |
| U1007 | names | Generated requirement marked active |
| U1008 | names | Module path does not match file path |
| U1009 | names | Reserved module name |
| U1010 | names | A module whose first path segment is a dependency's local name |
| U2001 | types | Type mismatch |
| U2002 | types | Unknown or ill-formed type |
| U2003 | types | Wrong number of type arguments |
| U2004 | types | Missing argument label |
| U2005 | types | Unknown argument label |
| U2006 | types | Duplicate argument |
| U2007 | types | Non-exhaustive `when` over variants |
| U2008 | types | Unreachable arm or misplaced `else` |
| U2009 | types | Missing or unknown field in constructor |
| U2010 | types | Cannot infer lambda parameter type |
| U2011 | types | Statement value is not `Unit` |
| U2012 | types | `result` outside `ensures` or `expect` |
| U2013 | types | `it` or `error` outside its allowed position |
| U2014 | types | Expression outside the refinement language |
| U2015 | types | Invalid predicate body (impure, failing, recursive or outside RL for `reflect`) |
| U2016 | types | Opaque predicate called outside `checked` |
| U2017 | types | Use of a binding after it was passed by `take` |
| U2018 | types | An `edit` argument together with another use of its binding in the same call |
| U2019 | types | Assignment to a binding that is not editable |
| U2020 | types | A parameter convention (`edit`, `take`) not matched at the call site, or declared where none is allowed |
| U2021 | types | A function with `edit` or `take` parameters used as a value |
| U2022 | types | Integer literal does not fit its type |
| U2023 | types | A binding declared outside a `together` branch assigned, or passed as `edit` or `take`, inside it |
| U2024 | types | A `together` body with a statement other than `let`, or fewer than two branches |
| U2025 | types | A type argument does not satisfy the constraint of its parameter |
| U2026 | types | An operation on a type parameter that its constraint does not allow |
| U2027 | types | An unknown constraint name |
| U2028 | types | Values of different units meet, or a measured and a unitless value |
| U2029 | types | An invalid unit declaration or unit expression |
| U2030 | types | A unit conversion without an expected unit, or not exact |
| U2031 | types | A float literal rounds to an infinity, or is not zero and rounds to zero |
| U2032 | types | A float literal other than zero, or a float range, as a pattern |
| U2033 | types | A comparison of an `F64` with a float literal zero |
| U2034 | types | `stop` or `skip` outside a loop body, or across a task boundary |
| U2037 | types | A decimal scale out of place |
| U2040 | types | A statement after `stop` or `skip` in the same block, which never runs |
| U3001 | authority | Capability not in scope |
| U3002 | authority | Operation not permitted by capability type |
| U3003 | authority | Effects exceed declared `uses` |
| U3004 | authority | Capability used in a pure function |
| U3005 | authority | Capability reference used as a value |
| U3006 | authority | Effectful or failing lambda escapes |
| U3007 | authority | Fallible expression without handling |
| U3008 | authority | `if error pass` with a different error kind |
| U3009 | authority | Failure variant not in the declared error kind |
| U3010 | authority | Handler does not match its capability |
| U3011 | authority | Handler missing an operation |
| U3012 | authority | Host capability in `main` not granted by `una run` |
| U3013 | authority | Effectful or fallible call nested inside an expression |
| U3014 | authority | Host capability used in an example |
| U3015 | authority | `std.testing` used outside a handler or example |
| U3016 | authority | Second-class type used as a type argument |
| U3017 | authority | `of` clause that names no parameter |
| U3018 | authority | `of` clause that names a parameter not of function type |
| U3019 | authority | `of` clause that names a function type with its own `uses` or `fails` |
| U3020 | authority | Call into a boundary module whose capability is not granted with `with` |
| U3021 | authority | Call that grants a boundary module more than its ceiling |
| U3023 | authority | Header that grants more than its lock in `una.toml` |
| U3024 | authority | Call into a boundary module from which a release is reachable |
| U3025 | authority | `if error` clause on an expression that cannot fail |
| U3026 | authority | `attempt` of a call that cannot fail |
| U3027 | authority | Code that may run forever where it is not allowed to |
| U3028 | authority | A function that says `diverges`, used as a value |
| U3029 | authority | A failure that must be handled where it is not |
| U4001 | security | Flow to a lower label |
| U4002 | security | Implicit flow through control, failure or call under raised pc |
| U4003 | security | Sink label exceeded |
| U4004 | security | `policy` outside a trusted module |
| U4005 | security | Invalid policy (public `via`, or type mismatch) |
| U4007 | security | Unknown label category |
| U4008 | security | A text literal with interpolation in a pattern |
| U5001 | verification | Obligation refuted |
| U5002 | verification | Obligation unknown |
| U5003 | verification | Solver missing or version mismatch |
| U6001 | runtime | `main` failed |
| U6003 | runtime | Call depth or memory limit exceeded |
| U6004 | runtime | A header value contains a control character |
| U6005 | runtime | A host value is not valid UTF-8 |
| U6006 | runtime | The floating-point environment of the platform is not the default one |
| U6007 | runtime | The host stopped the run before it finished |
| U6009 | runtime | The host gave no random bytes for keyed hashing |
| U7001 | project | Non-canonical formatting |
| U7002 | project | Invalid `una.toml` |
| U7003 | project | Escape budget exceeded |
| U7004 | project | Missing or invalid `verification.lock` |
| U7005 | project | Trusted module not declared in both header and `una.toml` |
| U7008 | project | Invalid `ita.una` or `itaty.una` |
| U7009 | project | Missing, invalid or inconsistent `ita.lock` |
| U7010 | project | A package, archive, statement, root, checkpoint or log proof that fails verification |
| U7011 | project | A locked package that is neither in the vendor directory nor in Itaoca |
| U7012 | project | A dependency the root may not use as written |
| U7013 | project | No resolution: no release meets a need, or a bound of resolution is exceeded |
| U7014 | project | A locked release yanked, under an advisory or signed after a stated compromise, or a pending recovery, a key change or a transfer of its owner |
| U8001 | internal | Internal compiler error (always a bug) |

Codes are never reused. Retired codes stay in this table marked retired. Gaps in the numbering, such as U4006 and U6002, are unassigned numbers, not retired codes (ADR 0012). `una explain <code>` prints a paragraph and a minimal failing and fixed example for every code, but five. U8001 is an internal error, a bug of the compiler, so its text has no example and says how to report it (ADR 0034). U6005 comes from the host's data, which no program text causes, so its text has no example and says how to fix the data (ADR 0086). U6006 comes from the platform's floating-point environment, which no program text causes either (ADR 0114). U7013 and U7014 come from `una ita` when it resolves, U7013 also from `una ita verify` for a package it cannot verify yet, and `una check` never reports them, so their texts have no example either (ADR 0122).

## 5. Command line

| Command | Purpose |
| --- | --- |
| `una check [path]` | Tiers 0 and 1; `--tier 0` skips the solver; `--dump-core <module>` prints Core IR |
| `una fmt [path]` | Rewrite to canonical form; `--check` reports U7001 without writing |
| `una fix [path]` | (v0.2, ADR 0084) Apply the mechanical repairs that `una check` reports, then format; `--dry-run` writes nothing; `--tier 0` skips the solver in the final check |
| `una run [path]` | Run `main` of `[run] entry`; `--grant <spec>` repeated |
| `una build [path]` | (v0.2, ADR 0055) Compile `main` of `[run] entry` into a native executable; `--out <file>` (`spec/16 §2`) |
| `una test [path]` | Run all examples |
| `una obligations [path]` | List obligations with status |
| `una contract <qualified.name>` | Print the full header, declared or inferred, with doc comments and requirements |
| `una trace <REQUIREMENT_ID>` | Requirement to contract to obligation chain |
| `una core [path]` | (v0.3, ADR 0083) Check at tier 0 and print the package's Core IR: with `--json`, one `una.core/1` document (§8); otherwise the text syntax of `spec/09`, every module in turn. `--with-rejected` (ADR 0091), with `--json` only, adds the declarations rejected for information flow, apart from Core (§8); without `--json` it is a usage error |
| `una report [path]` | Build report (§6) |
| `una explain <code>` | Explanation of a diagnostic code |
| `una lsp` | (v0.2, ADR 0063) The language server: LSP 3.17 over stdin and stdout, for editors. It checks, formats and solves; it never runs programs |

Global flags: `--json` (machine output on stdout, nothing else on stdout), `--color never|auto|always`. (v0.2, #305.) `una --version` prints the version of `una`, the editions it serves and the Z3 version its default lock pins, one per line, as `una 0.3.16`, `editions: 0.1, 0.2, 0.3, 0.4`, `z3: 5.1.0` for v0.3.15. (v0.3, ADR 0056 amendment.) A version that serves edition `0.3` lists it: `editions: 0.1, 0.2, 0.3, 0.4`. With `--json` it prints `{"schema":"una.version/1","version":...,"editions":[...],"z3":...}`, with keys in that order. Neither names a commit, so a build of a version prints the same text from any checkout. `una` with neither a command nor `--version` is a usage error. Exit codes (ADR 0036): `0` success; `1` the output holds at least one error diagnostic with a registered code, or reports an obligation that is not proven (`una obligations`, `una trace`, `una report`) or a failed example (`una test`); `2` a usage error, a problem of the invocation (its flags, grants, paths or names) printed to stderr without a code, with no diagnostic document on stdout; `3` internal compiler error (U8001). A problem of the project files is a diagnostic like any other, in human and `--json` output: an invalid `una.toml` (U7002) and a missing or invalid `verification.lock` (U7004) exit with `1`.

Output streams (ADR 0015): human-mode diagnostics go to stderr; command results go to stdout (the explain text, contracts, reports, obligation listings and the program output of `una run`). With `--json`, the JSON document goes to stdout and stderr stays empty, except for internal compiler errors (U8001). A failure to write output exits with code 2. (v0.2, #259.) Writing to a closed stdout or stderr is not a failure to write: the output is discarded, as `/dev/null` discards it, and the command exits as it would with the stream open. A broken pipe (EPIPE), a reader that went away, is a failure to write, exit code 2. A `print` whose output is discarded is traced as `ok` (`spec/11 §5`).

Grant specs: `console`, `clock`, `env`, `keys:<keyfile>`, `files:<path>:read`, `files:<path>:write`, `files:<path>:read,write`. (v0.2, ADR 0074.) In edition `0.2`, also `http:<origin>` and `http:<origin>:<methods>`, which grant `Http` operations (`spec/24 §2`), and the flags `--max-response <MiB>` and `--http-timeout <seconds>` of `una run` and of executables (`spec/24 §4`). They grant `Console[print]`, `Clock[now]`, `Env[get]`, `Keys[hmac]`, and the `Files` operations of the modes. When `una.toml` names a `[run] entry`, `una check` reports each `uses` clause of that module's `main` whose capability is not a host capability, since no grant can provide it (U3012); when `una.toml` also lists `[run] grants`, a host operation of `main` that no grant allows is U3012 at its `uses` clause. `una run` checks `main` against `[run] grants` and `--grant` flags together before the program starts (ADR 0028).

`una run [path]` runs `pub main() -> Unit` of the entry module: `[run] entry`, or the module of a single file (ADR 0032). It checks the package first, at tier 1, with the grants of `[run] grants` and of `--grant` flags together as final grants, so U3012 reports before execution every `uses` of `main` that they do not grant. A package without an entry, an entry without `pub main() -> Unit`, a `--grant` value that is not a grant spec, an unreadable or invalid key file, a `--max-depth`, `--max-memory`, `--max-response` or `--http-timeout` that is not a number, a `--max-memory` of `0` (#185), a `--max-response` or `--http-timeout` of `0`, an `--http-timeout` over 86400 (a day), a `--max-depth` whose stack cannot be allocated, and a `--workers`, `--task-depth`, `--io-threads` or `--file-threads` that is not a number or is out of its range (`spec/27 §3`) are usage errors (exit code 2), on `una run` and `una test`. (v0.2, ADR 0061.) The interpreter's stack is sized from `--max-depth` and capped at 1 GiB, and `--max-memory <MiB>` (default 4096) bounds the heap a run holds (`spec/11 §7`); `una test` takes both flags. Relative paths of `una.toml` grants resolve against the package root, and those of flags against the current directory. A `Files` path is relative to the grant's prefix: it is normalized lexically, its existing part is resolved through the file system, and a path that ends outside the prefix, absolute, through `..` or through a link, fails with `FileError.Denied`, as does an operation that the grant's mode does not allow. The key file of `keys:<keyfile>` is a TOML table of key names to base64 keys, loaded before the run starts; the first grant that names a key wins, and an unknown name fails with `KeyError.UnknownKey`. (Edition 0.4 preview, ADR 0116.) When the package's `ita.una` has a `drive` section, `una run` runs the driver of `spec/26 §2` instead of `main`, and a driven module without the shapes of `spec/26 §2` rule 2 is a usage error. `una run` and executables take `--stop-after-iterations <n>`, an integer from 1 to 2^63 − 1, the run's own host stop request once `n` iterations of `for ever` loops, the driver's included, have started; `--deadline <seconds>`, from 1 to 31,536,000, a host stop request after that many seconds; and `--stop-grace <seconds>`, from 1 to 86,400 (default 10), the time from the first host stop request to the hard stop (`spec/26 §6`). Another value of any of them is a usage error. A run that the hard stop cancels ends with U6007, exit code 1, with the trace of an abort; with `--json`, its `status` is `aborted`. `una run` and executables take `--virtual-time`, a switch with no value: the run's clock is then virtual, from 0 at the start of the run, and only the run's own waits move it, so `Clock.now` and the driver's ticks are the same in every run and in both engines (`spec/26 §8`). A value given to it, as in `--virtual-time=yes`, is a usage error. `una test` takes no such flag: time in a test is always virtual (`spec/26 §8`). `--deadline` counts seconds of the wall clock under `--virtual-time` too: it is a request of the host, not a time of the program.

In human mode, the lines the program prints go to stdout as it prints them, and U6001, U6003, U6004, U6005 and U6009 go to stderr, with exit code 1. U6001 points at the name of `main` and names the variant, with the payload fields whose labels are `public`; U6003 points at the call that exceeds the depth, or that finds the run over its memory budget. (v0.2, ADR 0113, 2026-10-15.) A native executable whose program has a task construct or a `for ever` reports U6003 with no position before `main`, `the threads and stacks of this run's tasks could not be made`, when the system refuses its threads or the 64 task stacks it reserves, as Linux can under strict overcommit (`vm.overcommit_memory=2`); a lower `--max-depth` shrinks that reservation (`spec/27 §1` rule 4). A native executable's U6003 for a reference count at its limit has no position: its one line on stderr names the code and the message, and no trace file is written. The interpreter never reports it, so `una run --json` has no shape for it. U6004 points at the operation of `Http` given a header value with a control character (`spec/24 §1`). U6005 points at the `Files.read` or `Env.get` whose file content or variable value is not valid UTF-8 (`spec/14 §3`, ADR 0086). With `--json`, `una run` prints `{"schema":"una.run/1","tier":...,"status":...,"output":[...],"failure":...,"diagnostics":[...],"trace":[...]}` with keys in that order: `tier` is the tier the checks ran at; `status` is `ok`, `failed` (U6001), `aborted` (U6003, U6004, U6005, U6009 or U8001) or `rejected` (the checks reported errors, which `diagnostics` holds, and nothing ran); `failure` is `null` or `{"variant":...,"fields":{...}}` with the `public` fields only. U6009 is reported at the start of a run whose reachable code can make a hash table, before `main` and before any effect, when the host's random source gives no bytes for the process key of keyed hashing (`spec/28 §1`, ADR 0119 rule 9); no fixed or guessable key replaces it.

`una test [path]` prints, in human mode, one line per example on stdout, `example <module> "<name>" ... ok` or `... FAILED`, followed for a failed example by the file, line and text of the expectation to blame, when there is one, and the reason; then a blank line and `<n> examples: <p> passed, <f> failed`. It exits with code 1 when an example fails. With `--json` it prints `{"schema":"una.test/1","tier":...,"diagnostics":[...],"examples":[...],"passed":p,"failed":f}`: `diagnostics` holds the errors of the checks when nothing ran, or U6003 and U8001 of aborted examples; each example has `module`, `name`, `outcome` (`pass` or `fail`), `reason` (`null`, or why it fails), `expectation` (`null`, or `{"file":...,"line":...,"text":...}`), `result` (`{"kind":"ok","value":...}`, `{"kind":"fail","variant":...,"value":...}` or `{"kind":"aborted"}`) and `trace` (`spec/11 §5`). (v0.2, ADR 0078.) The document also holds `redacted`, after `tier`: `true` when the results and reasons of its examples redact labeled fields, as in a package of edition `0.1`, and `false` when they show every field, as in a package of edition `0.2` (`spec/11 §5`). The key is additive, and the schema stays `una.test/1`.

(v0.2, ADR 0084.) `una fix [path]` applies, in rounds, the mechanical repairs of the package.
- **Candidates.** Of each diagnostic, only its first repair (by `escape_cost`, then `rank`) is a candidate. It is applied when all of these hold:
  - its `edit` is not `null`;
  - its `escape_cost` is 0;
  - it does not widen authority or visibility (§3), with no flag to change that;
  - its file is a module of the package, never `una.toml` or `verification.lock`, and holds valid UTF-8;
  - it is not U7001.
  - (ADR 0084, amendments of 2026-10-14 and 2026-10-15.) Of tier 1, only the first repair of a refuted `effect_condition` obligation (U5001) or of a refuted `contract_lock` obligation (U3023) can be a candidate (`spec/05 §3`); never an unknown one (U5002), and never a U3023 of tier 0.
- **Rounds.** Each round checks the package at tier 0, in memory. (ADR 0084, amendment of 2026-10-14.) When that check reports nothing and the requested tier is 1, the round also checks tier 1 and takes its candidates; with `--tier 0`, no round does. Within a file it takes candidates in order of start, end and diagnostic. An edit that overlaps one already taken, or touches it at a point where either inserts, waits for the next round; an identical edit is merged. Rounds stop when one takes no edit (`done`), when the texts return to an earlier state (`cycle`), or after 16 rounds (`limit`).
- **Format and write.** Every module that parses is then formatted, as `una fmt` does. Each changed file is written once, through a temporary file and a rename; `--dry-run` writes nothing.
- **Final check.** The package is checked at the requested tier, 1 by default, and its diagnostics are those that remain.
- **Human output.** On stdout, one line per applied edit, `fixed <file>:<line>:<col> <code> <description>`, at its position in the text of its round. Then one line per first repair with an edit that remains and was not applied, `skipped <file>:<line>:<col> <code> <description>: <reason>`. Then, when the rounds did not finish, a `stopped:` line. The remaining diagnostics go to stderr.
- **`--json`.** It prints `{"schema":"una.fix/1","dry_run":...,"rounds":n,"stopped":"done"|"cycle"|"limit","applied":[...],"skipped":[...],"formatted":[...],"diagnostics":[...]}`, with keys in that order:
  - each applied edit is `{"round","file","code","description","start","end","replacement"}`;
  - each skipped repair is `{"file","code","start","description","reason"}`, where `reason` is `widens authority`, `escape cost`, `not a module`, `invalid UTF-8` or `round limit`;
  - `formatted` lists the files the format step changed;
  - `diagnostics` holds the remaining diagnostics.
- **Exit codes.** As for `una check`, from the remaining diagnostics.

`una obligations [path]` prints one line per obligation, `<status> <id> <kind> <file>:<line>:<col> <goal>`, with `(cached)` for a cached result, and exits with code 1 when an obligation is not proven (ADR 0033).

Every command prints one JSON document with `--json`, its `schema` key first (ADR 0034). `una explain --json <code>` prints `{"schema":"una.explain/1","code":...,"domain":...,"meaning":...,"text":...}`. When the checks reject the package, `una contract`, `una trace`, `una obligations` and `una report` print the diagnostic list of `una check` and exit with code 1; `una run` and `una test` carry the diagnostics in their own documents. Usage errors stay on stderr, with exit code 2.

`una contract <qualified.name>` reads the package in the current directory and checks it at tier 0 (ADR 0029). The name is the module path followed by the function's name, as in `events.checkin.check_in`, and names a function. When the package has diagnostics, those of its project files included, `una contract` reports them as `una check` does and exits with code 1 before it resolves the name, since an inferred header is defined only for a package that checks (ADR 0036); in a package without diagnostics, an unknown name is a usage error (exit code 2). Otherwise it prints the doc comments, then the header in canonical form with every clause, declared or inferred, in canonical order, each inferred clause followed by the comment `-- inferred`, then one line per requirement of `derived from`: `-- requirement ID (status): "text"`. With `--json` it prints `una.contract/1`, compact, with keys in this order:

```json
{
  "schema": "una.contract/1",
  "function": "events.checkin.check_in",
  "public": true,
  "docs": ["Looks up a registration by phone, shows its availability, and records it."],
  "header": "pub check_in(phone: secret[PII] Phone) -> Availability",
  "constraints": [],
  "requires": [],
  "ensures": [],
  "uses": [ { "cap": "registrations", "capability": "Registrations", "ops": ["find"], "inferred": false } ],
  "fails": { "kind": "CheckInError", "inferred": false },
  "diverges": false,
  "derived_from": [ { "id": "EVENTS_3", "status": "active", "text": "..." } ]
}
```

`fails` is `null` for a function that cannot fail, `ops` lists the operations in the declaration order of the capability, and `docs` holds the doc comments without their `---` marker. (v0.2, ADR 0066.) The document also holds `constraints`, after `header`: the constrained type parameters, as `{"param": "T", "constraint": "Ord", "inferred": false}`, or `[]` (`spec/20 §8`). The key is additive, and the schema stays `una.contract/1`. (v0.2, ADR 0067.) The document also holds `result_labels`, after `constraints`: for a generic function, each position of its result whose summary is not empty, as `{"position": "top", "from": ["T"]}`, in the order of the positions in the declared type, or `[]` (`spec/07 §7` rule 8). A position is written as its path: `top`, `element`, `arg N`, `param N` and `result`, joined by `.`. The human output prints them on one line after the header, positions separated by `; ` and type parameters by `, `, as `-- result labels: top from T; element.arg 1 from T, U`; a function with none prints no line. The key is additive, and the schema stays `una.contract/1`. (v0.2, ADR 0067 amendment.) The document also holds `of_raises`, after `result_labels`: each parameter that an `of` clause names whose calls depend on decisions on type parameters, as `{"param": "k", "from": ["T"]}`, or `[]`; callers run their argument for it under the labels of those type arguments (`spec/07 §7` rule 8). The human output prints them after the result labels as `-- of raises: k from T`. The key is additive. (Edition 0.4 preview, ADR 0116.) The document also holds `diverges`, after `fails`: `true` for a function whose header says `diverges`, else `false`. The key is additive, and the schema stays `una.contract/1`.

## 6. Build report

`una report --json` produces:

```json
{
  "schema": "una.report/2",
  "package": { "name": "events-example", "version": "0.1.0" },
  "lock": { "solver_version": "...", "hash": "..." },
  "proven": { "semantic": 14, "security": "by typing", "authority": "by typing" },
  "obligations": { "proven": 14, "refuted": 0, "unknown": 0, "cached": 14 },
  "authority": [ { "function": "events.checkin.check_in", "public": true, "uses": ["@registrations: Registrations[find]", "@audit: Audit[record]", "@keys: Keys[hmac]"], "inferred": false } ],
  "handler_authority": [],
  "dependencies": [],
  "information": { "policy_calls": [ { "policy": "events.keys.attendee_key", "purpose": "...", "owner": "privacy", "site": { "file": "...", "start": { "line": 30, "col": 13 } } } ] },
  "escapes": { "checked": [ { "module": "events.types", "count": 1, "budget": 1 } ] },
  "assumptions": [ { "capability": "events.checkin.Registrations", "operations": ["find"], "ensures": [] } ],
  "tcb": { "trusted_modules": [ { "module": "events.keys", "lines": 17 } ], "host_capabilities": ["Keys"], "solver": "z3 <version>", "proof_cache": ".una/cache/", "proof_cache_key": "~/.una/proof-cache.key", "registries": [], "dependency_trust": [] },
  "requirements": [ { "id": "EVENTS_3", "status": "active", "derived": ["events.seating.availability"] } ],
  "coverage": { "pub_functions": 3, "with_ensures": 1, "with_derived_from": 1, "trivial_ensures": 0 },
  "native": { "target": "aarch64-apple-darwin", "arbitrary_precision": ["events.seating.clamp_seats.raw"] },
  "out_of_model": ["timing", "termination and divergence", "resource consumption and allocation", "stack exhaustion", "external systems beyond declared signatures", "races between the check and the use of a file path", "hard links", "order of effects across concurrent tasks", "the time of a host stop", "interference of the host's load with the timing of an expression", "uncatchable termination"]
}
```

`trivial_ensures` counts `ensures` clauses the solver proves valid without any hypothesis (for example `ensures true`); they are reported as missing contracts.

`una report [path]` checks the package at tier 1. When the checks report diagnostics, it prints them as `una check` does; otherwise it prints the report, and exits with code 1 when an obligation is not proven. Its sections, in the order of the example (ADR 0034):

- `package`: the name and version of `una.toml`; a single file is named after its module, with version `0.0.0`.
- `lock`: the solver version, and the hex SHA-256 of the lock's contents (for a single file, of the default lock's text).
- `proven`: the number of proven obligations as `semantic`; `security` and `authority` are `"by typing"`. `obligations`: the counts by status, then `cached`, how many of these results came from the proof cache of `spec/12 §9` rather than from the solver (ADR 0041).
- `authority`: every function of the package with `uses`, by qualified name, with `public`, its `uses` as written in a header, and whether they are inferred. (v0.2, ADR 0062.) A `uses of p` clause is listed as written, as `"of each"`. The effects of a lambda count for the function that writes it, within its `uses`.
- `handler_authority` (v0.2, ADR 0081), after `authority`: every handler of the package with `uses` clauses, by qualified name, with `capability`, the capability it implements by its qualified name, `public`, and its `uses` as written in its header, in the form of `authority`. A handler's `uses` are always written, so an entry has no `inferred`. The key is additive, and the schema stays `una.report/2` (ADR 0076).
- `dependencies` (edition `0.4`, ADR 0122 rule 58), after `handler_authority`: every package of the program other than the root, in name order, with its source, digest, the ceiling each dependent writes for it, its effective ceiling, the authority its code uses, and the trust it adds (`spec/30 §12` rule 8). A package with no dependencies has `[]`. The key is additive, and the schema stays `una.report/2`.
- `information.policy_calls`: every call of a policy, in source order, with the policy, its purpose and owner, and the site.
- `information.timing` (edition 0.4, ADR 0116 rule 21), after `policy_calls`: each `for ever within`, in source order, as `{"construct": "for ever within", "function": "m.f", "label": "internal", "site": {...}}`, with the timing label of its body (`spec/26 §7`), and (ADR 0117 rule 13) each `e within d` whose timing label is above `public`, as `{"construct": "within", ...}`, with the timing label of `e` (`spec/27 §5`), all in source order. The key is absent when the list is empty, so the reports of earlier editions do not change, and the schema stays `una.report/2`.
- `escapes.checked`: every module that uses `checked`, with the count and the budget of `una.toml`.
- `assumptions`: every capability the package declares, with its operations and their `ensures` clauses as `op: clause`.
- `tcb`: the trusted modules with their line counts, the host capabilities the package's functions and handlers use (handlers since ADR 0081), the solver as `z3 <version>`, and `proof_cache`, the proof cache of the package, `.una/cache/`. In v0.1 its entries were trusted, not authenticated (ADR 0041). (v0.2, ADR 0064.) `proof_cache_key` follows: the key file that authenticates the cache's entries and is in the Trusted zone. It is `~/.una/proof-cache.key` for the default, or the value of `UNA_PROOF_KEY` as given, and `null` when the run used no cache. `registries` and `dependency_trust` follow (ADR 0122 rule 58, `spec/30 §12` rule 8): each registry source of the program with the digest of its root metadata, and the trusted modules of dependencies that the root consented to, with their line counts, by package; `trusted_modules` keeps the root's own.
- `requirements`: every requirement, by identifier, with its status and the functions derived from it.
- `coverage`: `pub_functions` counts the public functions, `with_ensures` those with at least one `ensures` clause that is not trivial, `with_derived_from` those with a `derived from` clause, and `trivial_ensures` the trivial clauses of public functions. An `ensures` clause is trivial when the solver proves it with no hypothesis, its parameters and `result` unconstrained: one more query per clause of a public function, cached like the others; a solver that cannot answer these queries gives U5003 or U8001 instead of the report.
- `native` (v0.2, ADR 0055): `target`, the host target of `una build` (`spec/16 §1`), and `arbitrary_precision`, the bindings whose `Int` the native backend keeps at arbitrary precision: every parameter and result of type `Int` of the package's functions whose refinements do not prove a range within a machine word, as `module.function.parameter` and `module.function.result` (NB-3, FW-1). A parameter's refinements are proven by each call and a result's by the function, so `clamp_seats`, whose result is `Seats = Int in 0..5000`, lists only its parameter.
- `generic_results` (v0.2, ADR 0067), after `native`: every public generic function of the package, by qualified name, with its `result_labels` as `una contract` prints them, or `[]` for one whose summaries are all empty (`spec/07 §7` rule 8), and its `of_raises`, as `una contract` prints them. The key is additive, and the schema stays `una.report/2`.
- `out_of_model`: the channels outside the model of `spec/07 §1`, in this order: timing, termination and divergence, resource consumption and allocation, stack exhaustion, external systems beyond declared signatures, races between the check and the use of a file path, hard links, order of effects across concurrent tasks, the time of a host stop, interference of the host's load with the timing of an expression, and uncatchable termination (`spec/14 §3` for hard links; v0.2, ADR 0065 and `spec/19 §3` for the order of effects, which the list gained with the release that implements `spec/19`; ADR 0116 rule 21 and `spec/26 §6` for the last three, additions within `una.report/2`).

Without `--json`, the same sections print as labeled lines.

(v0.2, ADR 0055.) The report is `una.report/2` from the first release that ships `una build` (`spec/16`), `v0.2.0-alpha.1`: it added the section `native`, and the `out_of_model` entry "interpreter stack exhaustion" became "stack exhaustion". Releases up to `v0.1.2` write `una.report/1`.

## 7. Project files

Package layout: `una.toml` and `verification.lock` at the root, sources under `src/`. Module `a.b.c` lives in `src/a/b/c.una` (U1008). Module paths whose first segment is `std` or `prelude` are reserved for the standard library, in a package and in a single file (U1009, `spec/10 §11` rule 8).

```toml
[package]
name = "events-example"
version = "0.1.0"
edition = "0.1"

[labels]
categories = ["PII", "Location"]

[trusted]
modules = ["events.keys"]

[escape_budget]
checked_per_module = 1        # default 0

[run]
entry = "events.main"
grants = ["console"]
```

Commands take a file or a directory, `.` by default (ADR 0021):

- A directory is a package only if it contains `una.toml` and `src/`; otherwise the command stops with a usage error (exit code 2). Its modules are the files `src/**/*.una`, hidden entries excepted, and the module in `src/a/b/c.una` MUST be declared `module a.b.c` (U1008).
- A file is a package of one module, whose root is the file's directory. It has the default settings, the default lock and no module path check.

`una.toml` is a TOML document with these tables and keys, and no others (U7002):

- `[package]`, required: `name`, `version` and `edition`, non-empty strings; `edition` is `"0.1"`. (v0.2, ADR 0056.) `edition` is `"0.1"` or `"0.2"`. The constructs of `spec/17`, `spec/18`, `spec/19` and `spec/20` and the keywords of edition `0.2` exist only in packages of edition `"0.2"` and later, and a single file has edition `"0.1"`, unless a manifest above it names another (`spec/30`, #1175).
  - (v0.3, ADR 0056 amendment of 2026-10-13.) `edition` is `"0.1"`, `"0.2"` or `"0.3"`. **Editions are cumulative.** Edition `"0.3"` has every construct, keyword and rule of edition `"0.2"`. A rule this specification states for edition `0.2` holds in every later edition, unless it says otherwise. The rules that target edition `0.3` (ADRs 0092, 0093 and 0094) apply only in packages of edition `"0.3"`.
  - **Edition `0.3` is a preview** until every rule that targets it has landed. Until then, a later `0.3.x` release MAY add obligations or errors to an edition `0.3` package: the edition only ever gets stricter, never looser. (ADR 0056 amendment of 2026-10-13, second.) Stricter concerns the programs that edition `0.2` can express: such a program keeps its result or gains obligations or errors. A later `0.3.x` release MAY also accept a construct that a rule targeting edition `0.3` adds and that earlier releases reject as a syntax error, such as the condition on an operation of ADR 0092. Each such construct MUST only narrow what a program may do: erasing it gives a program that edition `0.2` accepts, with no fewer allowed behaviors. Editions `0.1` and `0.2` keep their stability promise: a later release MUST NOT change what they accept or reject, except to fix a soundness or security error. (v0.3, ADR 0104.) A later `0.3.x` release MAY also give the methods `measured()`, `magnitude()` and `convert()` of `spec/25 §5` their meaning; every use of them where neither the receiver nor the expected type carries a unit is an error. (ADR 0103.) A later `0.3.x` release MAY also accept refined type arguments, and the facts on access they give, in a package of edition `0.3` (`spec/03 §3`); such a package can then be accepted where edition `0.2` rejects it, on facts the obligations of `spec/12 §3` prove. (ADR 0114.) A later `0.3.x` release MAY also accept the type `F64` and the constructs of `spec/29`, in a package of edition `0.3`; they do not erase to edition `0.2`. A program that edition `0.2` can express gains from them only U1002, for a declaration named `F64` and for a package function `to_f64` on integers, which the built-in conversion resolves ahead of.
  - (v0.4, ADR 0116 rule 0, and ADR 0122 Question 8.1.) Edition `0.4` is named in `ita.una` (`spec/30`), not here: `edition = "0.4"` in `una.toml` is U7002, with `una migrate` as its repair, and the package is still checked in edition `0.4`. Edition `"0.4"` has every construct, keyword and rule of edition `"0.3"`, and is a preview under the same terms until the rules that target it have landed. The constructs of `spec/26` exist only in packages of edition `"0.4"`.
- `[labels]`: `categories`, an array of category names (UpperCamelCase, U1005; unique, U1002).
- `[trusted]`: `modules`, an array of module paths, each a module of the package whose header says `trusted module` (U7005).
- `[escape_budget]`: `checked_per_module`, an integer of at least 0 (default 0).
- `[run]`: `entry`, the path of a module of the package, and `grants`, an array of the grant specs of §5.
- (v0.3, ADR 0092 rule 8 and its third amendment.) `[contracts]`, in edition `0.3` only: each key the qualified name of a public function of the package, each value an array of `uses` clauses written as a header writes them, operations listed in brackets, with `spec/05 §3` rule 4's conditions: `"pricing.update.update_prices" = ["@catalog: Catalog[price, set_price where not dry_run]"]`. A key that names no public function, an entry that does not parse or lists no operations, a binding locked twice, a capability that its name does not resolve to in the function's module, an operation the capability does not declare, and a condition that is not a `Bool` of the refinement language over the function's parameters are U7002 at the entry. `spec/05 §3` rule 5 checks the header against it. In editions `0.1` and `0.2`, `[contracts]` is an unknown table.
- (v0.3, ADR 0093.) `[boundaries]`, in edition `"0.3"` only; in editions `"0.1"` and `"0.2"` it is an unknown table (U7002). Each key is the path of a module of the package, and covers that module and every module below it by whole path segments: `a.b` covers `a.b.c`, not `a.bc`. Each value is the module's ceiling: an array of names-only strings `C[op, ...]`, with at least one operation, each once, and no condition or label, where `C` is a capability that the key's module sees and each `op` one it declares. Each of these is U7002: a key that is not a module of the package, a key inside another key's extent, a malformed ceiling string, a capability named twice in one ceiling, an unknown capability or operation, a module of `[trusted] modules` inside a boundary, and a `[run] entry` inside a boundary. Calls into a boundary are checked as `spec/05 §4` rule 5 states (U3020, U3021, U3024).

A TOML syntax error, a missing required key, and a value of the wrong type or outside its range are U7002. The default settings are empty `[labels]`, `[trusted]` and `[run]` tables and a budget of 0.

`verification.lock` is a TOML document with exactly the keys of `spec/12 §9`: `encoding_version = "una-smt-1"`, `solver = "z3"`, `solver_version` a non-empty string, `logic = "QF_UFLIA"`, `step_budget` an integer of at least 1, and `random_seed` an integer of at least 0. A missing file, a TOML syntax error, a missing or unknown key and a wrong value are U7004; a solver of another version is U5003. A single file uses the default lock, which holds the values of the repository's `verification.lock`.

Project errors do not stop checking: the modules are still checked, with the default settings in place of an invalid `una.toml`.

(v0.4, ADR 0122.) **Edition `0.4` and later.** A package of edition `0.4` describes itself in `ita.una` and is pinned by `ita.lock`, in Una's own lexical syntax, by the closed grammars of `spec/30`; a directory with `ita.una` is a package with that file in place of `una.toml`, and one with both is U7008. Every error that this section makes U7002 for a key of `una.toml` is U7008 at the corresponding line of `ita.una`; every error of `ita.lock` is U7009 where `verification.lock` has U7004. `una fmt ita.una` formats the file, `una fmt` of the package leaves it alone, and `una fix` never edits `ita.una`, `ita.lock` or `itaty.una`.

## 8. Core export

(v0.3, ADR 0083.) `una --json core [path]` prints the package's Core IR (`spec/09`) as one `una.core/1` document, for programs outside the compiler, such as the checker of the formal track. The text form of `spec/09 §4` stays a debug output; this document is the stable form.

```json
{
  "schema": "una.core/1",
  "edition": "0.2",
  "complete": true,
  "modules": [ { "path": "app", "decls": [ ... ], "examples": [ ... ] } ],
  "library": [ ... ],
  "host": [ ... ]
}
```

- `complete` is `true` when the package checked clean at tier 0, so Core holds every declaration; otherwise Core holds only the declarations without diagnostics, which go to stderr, and the exit code follows them.
- `modules` are in the order of Core IR; `library` holds the prelude's signatures and `host` the host capabilities' declarations.
- (ADR 0091.) With `--with-rejected`, the document has one more key, `rejected`, after `file_state`: a list of `{"module": "app", "decl": { ... }, "diagnostics": [{"code": "U4003", "span": { ... }}]}`, in path order, then declaration order. It holds each declaration with at least one diagnostic, every one of which is in the `security` domain, and whose elaboration is complete; `decl` is its Core, encoded as `modules[].decls` are, and `diagnostics` are its diagnostics, each with its code and its primary location, which the declaration holds. These declarations are never in `modules`, and `complete` keeps its meaning. Without the flag, the key is absent and the document is unchanged; a consumer that ignores unknown keys reads the same document either way. No other command reads these declarations.

**Encoding.** The document is a mechanical image of `una_core::ir`:
1. A value of an enum is an object whose first key is `kind`, the name of its variant in snake case (`LetTuple` is `let_tuple`), followed by the variant's fields under their Rust names, in declaration order. A field named `kind` is written `kind_of`. A tuple variant's one field is `value`, and its several fields `values`, an array.
2. A struct is an object of its fields under their Rust names, in declaration order.
3. A tuple is an array, an `Option` its value or `null`, a `Box` its content, a `usize` or `u32` a number, a name or an integer literal a string, as Core holds it (`x%2`, `-12`).
4. A label is `{"level": "secret", "categories": ["PII"], "symbols": ["T", "T*"]}`: its level, then its categories and its symbolic labels in their sorted order (`spec/07 §7` rule 8).
5. A source span is `{"file": "src/app.una", "start": [3, 5], "end": [3, 9]}`, lines and columns from 1, as the locations of diagnostics are.
6. A path is `{"module": "app", "name": "main"}`.

Keys are printed in this order, and two runs over the same sources print the same bytes.

**The kinds.** The variants of each enum, which a test compares with the source of `una_core::ir`:

| Enum | Kinds |
| --- | --- |
| `Atom` | `var`, `int`, `f64`, `decimal`, `bool`, `text`, `unit` |
| `Base` | `int`, `fixed`, `f64`, `decimal`, `bool`, `text`, `unit`, `hashed`, `named`, `param`, `tuple`, `unknown` |
| `BinOp` | `add`, `sub`, `mul`, `div`, `mod`, `eq`, `ne`, `lt`, `le`, `gt`, `ge`, `and`, `or`, `implies` |
| `Bound` | `rhs`, `expr` |
| `Coll` | `stack`, `queue`, `set`, `hash_map`, `hash_set`, `priority_queue`, `bit_set` |
| `Constraint` | `none`, `eq`, `ord`, `integer` |
| `DecimalOp` | `add`, `sub`, `mul`, `neg`, `eq`, `ne`, `lt`, `le`, `gt`, `ge`, `widen`, `rescale`, `rescale_exact`, `divide`, `remainder`, `to_int`, `abs`, `coefficient`, `from_int`, `from_f64`, `to_f64`, `to_text`, `parse`, `from_json`, `to_json` |
| `Decl` | `alias`, `data`, `pred`, `cap`, `handler`, `fun`, `policy` |
| `Expectation` | `holds`, `fails`, `performed` |
| `ExitTag` | `next`, `stop` |
| `Expr` | `atom`, `let`, `let_tuple`, `loop`, `exit`, `together`, `together_for`, `if`, `case`, `fail`, `handle`, `attempt`, `within`, `with`, `unreachable` |
| `FixedOp` | `wrapping_add`, `wrapping_sub`, `wrapping_mul`, `saturating_add`, `saturating_sub`, `saturating_mul`, `bit_and`, `bit_or`, `bit_xor`, `bit_not`, `shift_right`, `wrapping_shift_left` |
| `FloatOp` | `add`, `sub`, `mul`, `div`, `neg`, `eq`, `ne`, `lt`, `le`, `gt`, `ge`, `abs`, `sqrt`, `fma`, `rem`, `floor`, `ceil`, `trunc`, `round_half_even`, `round_half_away`, `copy_sign`, `ieee_min`, `ieee_max`, `clamp`, `ieee_eq`, `ieee_ne`, `ieee_lt`, `ieee_le`, `ieee_gt`, `ieee_ge`, `is_nan`, `is_finite`, `is_infinite`, `is_sign_negative`, `is_zero`, `is_positive_zero`, `is_negative_zero`, `to_bits`, `to_int_exact`, `from_int`, `to_text` |
| `IntType` | `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64` |
| `LoopSource` | `list`, `range`, `until`, `ever` |
| `MapOp` | `has`, `get`, `with`, `without`, `keys`, `values`, `entries` |
| `Prim` | `add`, `sub`, `mul`, `div`, `mod`, `neg`, `eq`, `ne`, `lt`, `le`, `gt`, `ge`, `not`, `len`, `concat`, `to_text`, `convert`, `fixed`, `map`, `seq`, `set`, `float`, `shape`, `decimal` |
| `Rhs` | `atom`, `tuple`, `prim`, `call`, `apply`, `op`, `construct`, `field`, `list`, `lambda`, `checked`, `declassify` |
| `SeqOp` | `push_front`, `push_back`, `pop_front`, `pop_back`, `peek_front`, `peek_back`, `items` |
| `SetOp` | `has`, `with`, `without`, `first`, `last`, `between`, `items`, `union`, `intersection`, `difference`, `is_subset` |
| `ShapeFun` | `rows`, `columns` |
| `Source` | `rebind`, `handler` |
| `Term` | `var`, `it`, `result`, `int`, `bool`, `text`, `unit`, `ctor`, `field`, `len`, `shape`, `element`, `has`, `proj`, `neg`, `not`, `binary`, `is`, `in`, `call`, `f64`, `float`, `to_f64` |
| `Type` | `base`, `fn` |

**What inference adds** (#443). Core holds what elaboration gives it. The document adds facts the checker infers, each as an additive key, in the order shown:

- **On the object of a function** (`fun`), after `public`, as `una contract` prints them (`§5`):
  - `uses_of`, the parameters its `uses of` clauses name, written or inferred;
  - `fails_of`, the parameter its `fails of` clause names, or `null`;
  - `result_labels`, its result summary, as `{"position": "top", "from": ["T"]}`;
  - `of_raises`, as `{"param": "k", "from": ["T"]}` (ADR 0062, ADR 0067).
- **On a `let` that binds a lambda given to an `of` parameter**, after `written`: `of_lambda`, as `{"effects": true, "fails": false}`. It says whether the lambda's body performs effects and whether it can fail. Its function type in Core carries neither, since they are its caller's (ADR 0062).
- **On the document**, after `host`: `file_state`, the package's file-state label (ADR 0085 decision 1), as a label. It joins what decides the package's `Files.write` calls, and it labels the failure of every `Files` operation. It is `public` when the package writes no file. (#467.)

**What a consumer may rely on.** Every fact the formal track's re-check reads from these keys may only raise a signature: more labels, more effects, more failures. The re-check then checks the body against the raised signature, so a wrong fact gives a rejection, never an accept. A future key that could lower a label or drop an effect MUST NOT be added on this assumption. Such a key needs its own argument of soundness, or the re-check reports what depends on it as not re-checked.

`file_state` is such a key: a lower label would label `Files` failures lower. Its argument of soundness is that the re-check does not trust it. Every write the package can perform, wherever it appears, is checked against it, or no `Files`-using declaration of the document is accepted. The re-check MUST reject a write whose path or program counter is above it, and MUST NOT re-check a document whose `file_state` is above `internal`. A write it does not check counts as unchecked: one in a declaration it rejects or does not re-check, in a handler, or in a node it cannot read. So a value too low gives a rejection or a declaration not re-checked, never an accepted one. A value too high only labels failures higher, which can reject, never accept, what the true label would reject.

(ADR 0114.) An `f64` atom, a float literal, has one key after `kind`, `bits`: its bits as 16 lowercase hex digits. An operation of `F64` is a `float` primitive whose `value` is its `FloatOp`, so no existing kind changes meaning.

(ADR 0120.) A `decimal` base has one key after `kind`, `scale`. A `decimal` atom, a decimal literal, has two, `coefficient`, in decimal with an optional `-`, and `scale`: its value is the coefficient times ten to the minus scale. An operation of `Decimal` is a `decimal` primitive whose `value` is its `DecimalOp`, with a key `scales` after `kind`: the static scales its meaning depends on, in the order `spec/29 §16` gives them.

**Versioning** (ADR 0076). The `kind` of a node is an **open** field: Core gains forms as the language grows, and adding one keeps `una.core/1`. A consumer MUST report a declaration that holds a `kind` it does not know as not understood, never as accepted or checked. A label's `level` is closed. Any other change follows ADR 0076.
