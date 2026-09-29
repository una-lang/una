# 13. Diagnostics, Agent ABI and project files

Status: Normative, v0.1. The JSON shapes in this file are the Agent ABI, schema version 1: agents consume them, so changes follow the ADR process and bump the version in the `schema` strings.

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

## 3. Repair candidates

For U5001 and U5002 the compiler proposes, when applicable, in this order:

| Candidate | `escape_cost` |
| --- | --- |
| Strengthen a parameter or field type to a refined kind that implies the goal | 0 |
| Add a `requires` clause equal to the missing hypothesis (only for non-`pub` functions or with a note that callers change) | 0 |
| Add a guard (`if` or `when` arm) that handles the failing case | 0 |
| Replace the value with `checked(value, goal) else fail <Variant>` | 1 |

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

Codes are never reused. Retired codes stay in this table marked retired. Gaps in the numbering, such as U4006 and U6002, are unassigned numbers, not retired codes (ADR 0012). `una explain <code>` prints a paragraph and a minimal failing and fixed example for every code.

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

Global flags: `--json` (machine output on stdout, nothing else on stdout), `--color never|auto|always`. Exit codes: `0` success, `1` errors in the program or failed tests, `2` usage or configuration error, `3` internal compiler error.

Output streams (ADR 0015): human-mode diagnostics go to stderr; command results go to stdout (the explain text, contracts, reports, obligation listings and the program output of `una run`). With `--json`, the JSON document goes to stdout and stderr stays empty, except for internal compiler errors (U8001). A failure to write output exits with code 2.

Grant specs: `console`, `clock`, `env`, `keys:<keyfile>`, `files:<path>:read`, `files:<path>:write`, `files:<path>:read,write`.

## 6. Build report

`una report --json` produces:

```json
{
  "schema": "una.report/1",
  "package": { "name": "events-example", "version": "0.1.0" },
  "lock": { "solver_version": "...", "hash": "..." },
  "proven": { "semantic": 14, "security": "by typing", "authority": "by typing" },
  "obligations": { "proven": 14, "refuted": 0, "unknown": 0 },
  "authority": [ { "function": "events.checkin.check_in", "public": true, "uses": ["@registrations: Registrations[find]", "@audit: Audit[record]", "@keys: Keys[hmac]"], "inferred": false } ],
  "information": { "policy_calls": [ { "policy": "events.keys.attendee_key", "purpose": "...", "owner": "privacy", "site": { "file": "...", "start": { "line": 30, "col": 13 } } } ] },
  "escapes": { "checked": [ { "module": "events.types", "count": 1, "budget": 1 } ] },
  "assumptions": [ { "capability": "events.checkin.Registrations", "operations": ["find"], "ensures": [] } ],
  "tcb": { "trusted_modules": [ { "module": "events.keys", "lines": 17 } ], "host_capabilities": ["Keys"], "solver": "z3 <version>" },
  "requirements": [ { "id": "EVENTS_3", "status": "active", "derived": ["events.seating.availability"] } ],
  "coverage": { "pub_functions": 3, "with_ensures": 1, "with_derived_from": 1, "trivial_ensures": 0 },
  "out_of_model": ["timing", "termination", "resource consumption", "external systems beyond declared signatures"]
}
```

`trivial_ensures` counts `ensures` clauses the solver proves valid without any hypothesis (for example `ensures true`); they are reported as missing contracts.

## 7. Project files

Package layout: `una.toml` and `verification.lock` at the root, sources under `src/`. Module `a.b.c` lives in `src/a/b/c.una` (U1008).

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
