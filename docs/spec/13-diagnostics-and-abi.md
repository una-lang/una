# 13. Diagnostics, Agent ABI and project files

Status: Normative, v0.1. The JSON shapes in this file are the Agent ABI, schema version 1: agents consume them, so changes follow the ADR process and bump the version in the `schema` strings. Schema versions freeze at the `v0.1.0` tag: before it, the shapes of version 1 may still change, and after it, any change bumps the version.

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
| `repairs` | Zero or more candidates, sorted by `escape_cost`, then `rank` |
| `edit` | `null`, or `{ "file", "start", "end", "replacement" }` when the repair is mechanical |

Diagnostics are sorted by file, start position, then code. Output is deterministic.

With `--json`, `una check` and `una fmt --check` print their diagnostics as one JSON object on stdout, followed by a newline (ADR 0012):

```json
{ "schema": "una.diagnostics/1", "diagnostics": [] }
```

`diagnostics` holds the diagnostics in the order above, each with its own `"schema": "una.diagnostic/1"` field; a new version of the diagnostic shape also bumps the list version. The array is empty when there are no diagnostics, and exit codes do not depend on `--json` (§5). Whitespace is not significant; the reference implementation prints compact JSON with object keys in the order shown in this section.

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

Other diagnostics MAY include repairs (for example, a missing label with its mechanical edit). Repairs are never applied automatically.

## 4. Code registry

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
| U4001 | security | Flow to a lower label |
| U4002 | security | Implicit flow through control, failure or call under raised pc |
| U4003 | security | Sink label exceeded |
| U4004 | security | `policy` outside a trusted module |
| U4005 | security | Invalid policy (public `via`, or type mismatch) |
| U4007 | security | Unknown label category |
| U5001 | verification | Obligation refuted |
| U5002 | verification | Obligation unknown |
| U5003 | verification | Solver missing or version mismatch |
| U6001 | runtime | `main` failed |
| U6003 | runtime | Call depth exceeded |
| U7001 | project | Non-canonical formatting |
| U7002 | project | Invalid `una.toml` |
| U7003 | project | Escape budget exceeded |
| U7004 | project | Missing or invalid `verification.lock` |
| U7005 | project | Trusted module not declared in both header and `una.toml` |
| U8001 | internal | Internal compiler error (always a bug) |

Codes are never reused. Retired codes stay in this table marked retired. Gaps in the numbering, such as U4006 and U6002, are unassigned numbers, not retired codes (ADR 0012). `una explain <code>` prints a paragraph and a minimal failing and fixed example for every code, but U8001: an internal error is a bug of the compiler, so its text has no example and says how to report it (ADR 0034).

## 5. Command line

| Command | Purpose |
| --- | --- |
| `una check [path]` | Tiers 0 and 1; `--tier 0` skips the solver; `--dump-core <module>` prints Core IR |
| `una fmt [path]` | Rewrite to canonical form; `--check` reports U7001 without writing |
| `una run [path]` | Run `main` of `[run] entry`; `--grant <spec>` repeated |
| `una test [path]` | Run all examples |
| `una obligations [path]` | List obligations with status |
| `una contract <qualified.name>` | Print the full header, declared or inferred, with doc comments and requirements |
| `una trace <REQUIREMENT_ID>` | Requirement to contract to obligation chain |
| `una report [path]` | Build report (§6) |
| `una explain <code>` | Explanation of a diagnostic code |

Global flags: `--json` (machine output on stdout, nothing else on stdout), `--color never|auto|always`. Exit codes (ADR 0036): `0` success; `1` the output holds at least one error diagnostic with a registered code, or reports an obligation that is not proven (`una obligations`, `una trace`, `una report`) or a failed example (`una test`); `2` a usage error, a problem of the invocation (its flags, grants, paths or names) printed to stderr without a code, with no diagnostic document on stdout; `3` internal compiler error (U8001). A problem of the project files is a diagnostic like any other, in human and `--json` output: an invalid `una.toml` (U7002) and a missing or invalid `verification.lock` (U7004) exit with `1`.

Output streams (ADR 0015): human-mode diagnostics go to stderr; command results go to stdout (the explain text, contracts, reports, obligation listings and the program output of `una run`). With `--json`, the JSON document goes to stdout and stderr stays empty, except for internal compiler errors (U8001). A failure to write output exits with code 2.

Grant specs: `console`, `clock`, `env`, `keys:<keyfile>`, `files:<path>:read`, `files:<path>:write`, `files:<path>:read,write`. They grant `Console[print]`, `Clock[now]`, `Env[get]`, `Keys[hmac]`, and the `Files` operations of the modes. When `una.toml` names a `[run] entry`, `una check` reports each `uses` clause of that module's `main` whose capability is not a host capability, since no grant can provide it (U3012); when `una.toml` also lists `[run] grants`, a host operation of `main` that no grant allows is U3012 at its `uses` clause. `una run` checks `main` against `[run] grants` and `--grant` flags together before the program starts (ADR 0028).

`una run [path]` runs `pub main() -> Unit` of the entry module: `[run] entry`, or the module of a single file (ADR 0032). It checks the package first, at tier 1, with the grants of `[run] grants` and of `--grant` flags together as final grants, so U3012 reports before execution every `uses` of `main` that they do not grant. A package without an entry, an entry without `pub main() -> Unit`, a `--grant` value that is not a grant spec, an unreadable or invalid key file, and a `--max-depth` whose stack cannot be allocated are usage errors (exit code 2). The interpreter's stack is sized from `--max-depth`, so the memory a run can use grows with the depth. Relative paths of `una.toml` grants resolve against the package root, and those of flags against the current directory. A `Files` path is relative to the grant's prefix: it is normalized lexically, its existing part is resolved through the file system, and a path that ends outside the prefix, absolute, through `..` or through a link, fails with `FileError.Denied`, as does an operation that the grant's mode does not allow. The key file of `keys:<file>` is a TOML table of key names to base64 keys, loaded before the run starts; the first grant that names a key wins, and an unknown name fails with `KeyError.UnknownKey`.

In human mode, the lines the program prints go to stdout as it prints them, and U6001 and U6003 go to stderr, with exit code 1. U6001 points at the name of `main` and names the variant, with the payload fields whose labels are `public`; U6003 points at the call that exceeds the depth. With `--json`, `una run` prints `{"schema":"una.run/1","tier":...,"status":...,"output":[...],"failure":...,"diagnostics":[...],"trace":[...]}` with keys in that order: `tier` is the tier the checks ran at; `status` is `ok`, `failed` (U6001), `aborted` (U6003 or U8001) or `rejected` (the checks reported errors, which `diagnostics` holds, and nothing ran); `failure` is `null` or `{"variant":...,"fields":{...}}` with the `public` fields only.

`una test [path]` prints, in human mode, one line per example on stdout, `example <module> "<name>" ... ok` or `... FAILED`, followed for a failed example by the file, line and text of the expectation to blame, when there is one, and the reason; then a blank line and `<n> examples: <p> passed, <f> failed`. It exits with code 1 when an example fails. With `--json` it prints `{"schema":"una.test/1","tier":...,"diagnostics":[...],"examples":[...],"passed":p,"failed":f}`: `diagnostics` holds the errors of the checks when nothing ran, or U6003 and U8001 of aborted examples; each example has `module`, `name`, `outcome` (`pass` or `fail`), `reason` (`null`, or why it fails), `expectation` (`null`, or `{"file":...,"line":...,"text":...}`), `result` (`{"kind":"ok","value":...}`, `{"kind":"fail","variant":...,"value":...}` or `{"kind":"aborted"}`) and `trace` (`spec/11 §5`).

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
  "requires": [],
  "ensures": [],
  "uses": [ { "cap": "registrations", "capability": "Registrations", "ops": ["find"], "inferred": false } ],
  "fails": { "kind": "CheckInError", "inferred": false },
  "derived_from": [ { "id": "EVENTS_3", "status": "active", "text": "..." } ]
}
```

`fails` is `null` for a function that cannot fail, `ops` lists the operations in the declaration order of the capability, and `docs` holds the doc comments without their `---` marker.

## 6. Build report

`una report --json` produces:

```json
{
  "schema": "una.report/1",
  "package": { "name": "events-example", "version": "0.1.0" },
  "lock": { "solver_version": "...", "hash": "..." },
  "proven": { "semantic": 14, "security": "by typing", "authority": "by typing" },
  "obligations": { "proven": 14, "refuted": 0, "unknown": 0, "cached": 14 },
  "authority": [ { "function": "events.checkin.check_in", "public": true, "uses": ["@registrations: Registrations[find]", "@audit: Audit[record]", "@keys: Keys[hmac]"], "inferred": false } ],
  "information": { "policy_calls": [ { "policy": "events.keys.attendee_key", "purpose": "...", "owner": "privacy", "site": { "file": "...", "start": { "line": 30, "col": 13 } } } ] },
  "escapes": { "checked": [ { "module": "events.types", "count": 1, "budget": 1 } ] },
  "assumptions": [ { "capability": "events.checkin.Registrations", "operations": ["find"], "ensures": [] } ],
  "tcb": { "trusted_modules": [ { "module": "events.keys", "lines": 17 } ], "host_capabilities": ["Keys"], "solver": "z3 <version>", "proof_cache": ".una/cache/" },
  "requirements": [ { "id": "EVENTS_3", "status": "active", "derived": ["events.seating.availability"] } ],
  "coverage": { "pub_functions": 3, "with_ensures": 1, "with_derived_from": 1, "trivial_ensures": 0 },
  "out_of_model": ["timing", "termination and divergence", "resource consumption and allocation", "interpreter stack exhaustion", "external systems beyond declared signatures", "races between the check and the use of a file path"]
}
```

`trivial_ensures` counts `ensures` clauses the solver proves valid without any hypothesis (for example `ensures true`); they are reported as missing contracts.

`una report [path]` checks the package at tier 1. When the checks report diagnostics, it prints them as `una check` does; otherwise it prints the report, and exits with code 1 when an obligation is not proven. Its sections, in the order of the example (ADR 0034):

- `package`: the name and version of `una.toml`; a single file is named after its module, with version `0.0.0`.
- `lock`: the solver version, and the hex SHA-256 of the lock's contents (for a single file, of the default lock's text).
- `proven`: the number of proven obligations as `semantic`; `security` and `authority` are `"by typing"`. `obligations`: the counts by status, then `cached`, how many of these results came from the proof cache of `spec/12 §9` rather than from the solver (ADR 0041).
- `authority`: every function of the package with `uses`, by qualified name, with `public`, its `uses` as written in a header, and whether they are inferred.
- `information.policy_calls`: every call of a policy, in source order, with the policy, its purpose and owner, and the site.
- `escapes.checked`: every module that uses `checked`, with the count and the budget of `una.toml`.
- `assumptions`: every capability the package declares, with its operations and their `ensures` clauses as `op: clause`.
- `tcb`: the trusted modules with their line counts, the host capabilities the package's functions use, the solver as `z3 <version>`, and `proof_cache`, the proof cache of the package, `.una/cache/`: in v0.1 its entries are trusted, not authenticated (ADR 0041).
- `requirements`: every requirement, by identifier, with its status and the functions derived from it.
- `coverage`: `pub_functions` counts the public functions, `with_ensures` those with at least one `ensures` clause that is not trivial, `with_derived_from` those with a `derived from` clause, and `trivial_ensures` the trivial clauses of public functions. An `ensures` clause is trivial when the solver proves it with no hypothesis, its parameters and `result` unconstrained: one more query per clause of a public function, cached like the others; a solver that cannot answer these queries gives U5003 or U8001 instead of the report.
- `out_of_model`: the channels outside the model of `spec/07 §1`, in this order: timing, termination and divergence, resource consumption and allocation, interpreter stack exhaustion, external systems beyond declared signatures, and races between the check and the use of a file path (`spec/14 §3`).

Without `--json`, the same sections print as labeled lines.

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

- `[package]`, required: `name`, `version` and `edition`, non-empty strings; `edition` is `"0.1"`.
- `[labels]`: `categories`, an array of category names (UpperCamelCase, U1005; unique, U1002).
- `[trusted]`: `modules`, an array of module paths, each a module of the package whose header says `trusted module` (U7005).
- `[escape_budget]`: `checked_per_module`, an integer of at least 0 (default 0).
- `[run]`: `entry`, the path of a module of the package, and `grants`, an array of the grant specs of §5.

A TOML syntax error, a missing required key, and a value of the wrong type or outside its range are U7002. The default settings are empty `[labels]`, `[trusted]` and `[run]` tables and a budget of 0.

`verification.lock` is a TOML document with exactly the keys of `spec/12 §9`: `encoding_version = "una-smt-1"`, `solver = "z3"`, `solver_version` a non-empty string, `logic = "QF_UFLIA"`, `step_budget` an integer of at least 1, and `random_seed` an integer of at least 0. A missing file, a TOML syntax error, a missing or unknown key and a wrong value are U7004; a solver of another version is U5003. A single file uses the default lock, which holds the values of the repository's `verification.lock`.

Project errors do not stop checking: the modules are still checked, with the default settings in place of an invalid `una.toml`.
