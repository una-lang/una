# 08. Requirements and tests

Status: Normative, v0.1.

(v0.5, ADR 0137.) **Binding words.** In edition `0.5`, a binding of the surface language that this chapter writes `let` is written `const`, and one it writes `let edit` is written `var` (`spec/02 §6`, `spec/17 §2`); the rules are unchanged. A test's `do` line is an expression and binds nothing, in every edition; when a later record lets a step of a test bind a name (issue #1591), the step is written `do const name = e`, and `do var` where it can be assigned, and no edition has `do let` (ADR 0137 rule 8). A Core `let` (`spec/09`) stays. The chapter's Una blocks keep the words of edition `0.4` until issue #1630 moves those of edition `0.5`.

## 1. Requirements

A requirement records a human or business statement with its origin and owner, so contracts can be traced to intent.

```una
requirement EVENTS_3
  text "An event with no remaining seats is always shown as sold out"
  source policy "Box Office Handbook 2.1"
  owner box_office
  status active
```

| Field | Values |
| --- | --- |
| `text` | The statement, as written by its source |
| `source` | `human`, `policy "<document>"`, `regulation "<document>"`, `generated "<task reference>"` |
| `owner` | Team or person accountable (lower snake case) |
| `status` | `proposed` or `active` |

Rules:

1. Requirement identifiers are unique per package (U1002).
2. A requirement with `source generated` MUST have `status proposed` (U1007). Agents may propose requirements; activating one requires a person to change its source to `human`, `policy` or `regulation`, which the semantic diff (v0.3) will surface.
3. Every identifier in a `derived from` clause MUST be declared in the package (U1006).
4. Functions derived from `proposed` requirements compile normally and are flagged in the build report.

`una trace EVENTS_3` lists every function deriving from the requirement, its contract clauses and the verification status of each obligation. It reads the package in the current directory and checks it at tier 1, then prints the requirement (identifier, status, text, source, owner) and every function derived from it, by qualified name, with its `requires` and `ensures` clauses and the status of each of its obligations. With `--json` it prints `{"schema":"una.trace/1","requirement":{...},"functions":[{"function":...,"requires":[...],"ensures":[...],"obligations":[{"id":...,"kind":...,"status":...,"goal":...}]}]}`. The package's diagnostics, those of its project files included, are reported before the identifier is resolved, with exit code 1 (ADR 0036); in a package without diagnostics, an unknown identifier is a usage error (exit code 2), and an obligation that is not proven gives exit code 1 (ADR 0034).

## 2. Tests

Tests are executable evidence. They live next to the code they exercise and run with `una test`. (v0.3, ADR 0105.) In edition `0.3` the declaration is written `test`; editions `0.1` and `0.2` write it `example`, with the same body, and every rule of this section holds of a declaration under either keyword. (Edition 0.4, ADR 0105 amendment of 2026-11-05.) Edition `0.4` writes it `test` too, and there `example` is an identifier, not a declaration keyword (`spec/01 §5`). This chapter says "test" for both.

```una
test "an unknown attendee fails and is not audited"
  given @registrations = no_registrations()
  given @audit = audit_log()
  given @keys = fake_keys()
  do check_in(sample_phone())
  expect fails NotRegistered
  expect not performed @audit.record
```

| Line | Meaning |
| --- | --- |
| `given @x = h(args)` | Install handler `h` as capability `@x` for this test |
| `do e` | The expression under test. It MAY be effectful or fallible; it needs no handling clauses |
| `expect e` | `e` is a `Bool` expression over `result`; it MUST evaluate to `true` |
| `expect fails V` | `do` failed with variant `V` |
| `expect performed @x.op` | The trace contains at least one completed or failed `op` on binding `@x` |
| `expect not performed @x.op` | The trace contains no `op` on binding `@x` |
| `stop after N` | (Edition 0.4, ADR 0116 rule 19.) The last line, optional: each `for ever` instance ends at the start of its own iteration `N + 1`, so `do` may run forever (`spec/26 §8`) |

Rules:

1. Tests MUST NOT use host capabilities (U3014); every capability `do` needs comes from `given` (U3001 if missing). This makes tests deterministic.
2. `expect e` lines with `result` are allowed only when `do` completed normally; if `do` failed, such an expectation fails the test with the failure shown.
3. Test names are unique per module (U1002).
4. Tests are type-checked, information-flow checked and verified like any other code, except for the sink exemption in `spec/07 §4`.
5. The one-effect-per-line rule (U3013) does not apply to `do` lines: they may nest fallible and effectful calls, evaluated left to right, and any failure ends the test.
6. Each test runs in a fresh abstract machine. `una test --json` reports, per test, the outcome, the failed expectation if any and the full effect trace (`spec/11 §5`).
7. The expectations of a test are checked in order, and the first that does not hold fails it; then a `do` line that failed with no `expect fails` line fails it too. An `expect` line that does not mention `result` runs even when `do` failed. `expect performed @x.op` holds when the trace has a completed or failed `op` event of the instance of `given @x`, and `expect not performed @x.op` when it has none. An example's trace records the operations performed in quiet code too, but those of instances that quiet code installed (#1544), which a run's trace leaves out (`spec/11 §5`, `spec/26 §6` rule 4, S-42). A test whose run aborts fails (ADR 0032).
