<p align="center">
  <img src="images/una-logo-wide.png" alt="Una: the verification-first programming language for AI agents. Safe, specified, verifiable, agent-ready." width="100%">
</p>

<p align="center">
  <strong>Agents write the implementation. The compiler proves it conforms to the specification. People review the specification.</strong>
</p>

<p align="center">
  <a href="https://github.com/una-lang/una/actions/workflows/ci.yml"><img src="https://github.com/una-lang/una/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/version-v0.1%20Una%20Core-7c5cff" alt="Version v0.1, Una Core">
  <img src="https://img.shields.io/badge/solver-Z3%205.1.0-4f46e5" alt="Solver: Z3 5.1.0">
  <img src="https://img.shields.io/badge/rust-1.98.1-3b3b58" alt="Rust 1.98.1">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-2e2e48" alt="License: Apache-2.0"></a>
</p>

Una is a verification-first programming language for machine-generated software. It turns agentic programming from open-ended code generation into optimization under verifiable constraints: an agent completes a task subject to types, authority, information-flow policy, contracts and escape budgets that the compiler checks, and every guarantee is stated against an explicit specification that people own and review.

> Rust makes unsafe code hard to write. Una makes it hard for an agent to write code whose intent, authority and invariants are not explicitly specified.

This repository holds the language specification, the design record and **Una Core**, the v0.1 reference compiler, written in Rust. Una proves **conformance to an explicit specification**, not correctness of intent: whether the specification says the right thing stays a human decision, and the compiler makes that specification small, explicit and reviewable.

## Contents

- [Project status](#project-status)
- [A first look](#a-first-look)
- [What Una proves, trusts and assumes](#what-una-proves-trusts-and-assumes)
- [The language at a glance](#the-language-at-a-glance)
- [The Agent ABI](#the-agent-abi)
- [Quick start](#quick-start)
- [Command line reference](#command-line-reference)
- [Example packages](#example-packages)
- [How the compiler works](#how-the-compiler-works)
- [Repository layout](#repository-layout)
- [Development](#development)
- [Documentation](#documentation)
- [Roadmap](#roadmap)
- [Contributing](#contributing)
- [License](#license)

## Project status

**v0.1, Una Core, is complete, reviewed and merged into `main`**: every milestone of `docs/plan/milestones-v0.1.md` is implemented and meets its acceptance criteria, and CI is green on Linux and macOS. The choices the specification left open were reviewed and kept on 2026-09-29: the ADRs that record them in `docs/adr/` are Accepted, the specification states their rules, and `docs/reviews/open-decisions.md` keeps each decision with its answer. The tag `v0.1.0`, which publishes the first release archives, comes next.

Una is a research language built to test a thesis (see [Roadmap](#roadmap)). It is not ready for production use.

| Milestone | What it delivers | Status |
| --- | --- | --- |
| M0 | Workspace, CI, pinned Rust toolchain and pinned solver | done |
| M1 | Lexer with significant indentation, parser, syntax diagnostics | done |
| M2 | Canonical formatter; non-canonical source is an error | done |
| M3 | Packages, `una.toml`, `verification.lock`, modules and name resolution | done |
| M4 | Types, kinds, generics, refinements, labeled calls | done |
| M5 | Capabilities, effects, failures, handlers, inferred private headers | done |
| M6 | Information flow: labels, program counter, sinks, declassification policies | done |
| M7 | Core IR, the Una Abstract Machine, `una run`, `una test`, effect traces | done |
| M8 | Verification with Z3: obligations, counterexamples, proof cache | done |
| M9 | Agent ABI: JSON for every command, `una trace`, the build report | done |
| M10 | Gate 1 readiness: example packages, the guide for agents, release archives | done |

| At a glance | |
| --- | --- |
| Rust crates | 10, about 33,000 lines of Rust |
| Specification | 15 normative chapters, 35 architecture decision records |
| Diagnostic codes | 65, each with an explanation, and all but the internal error with a failing and a fixed example |
| Tests | 320 Rust tests, 194 conformance cases, all passing |
| Example packages | 7, each verified at tier 1 with every obligation proven |
| Usage checkpoints | 4 of 4 reached, each with a demo script run in CI |

## A first look

A module of the reference example, `examples/events/src/events/seating.una`, exactly as the tests check it:

```una
module events.seating

requirement EVENTS_3
  text "An event with no remaining seats is always shown as sold out"
  source policy "Box Office Handbook 2.1"
  owner box_office
  status active

pub kind Seats = Int in 0..5000

pub kind Availability = SoldOut | Few | Open

pub kind Event
  name: Text
  remaining: Seats

pub kind Registration
  name: Text
  event: Event

--- Availability bands from the Box Office Handbook 2.1.
pub availability(e: Event) -> Availability
  ensures e.remaining < 1 implies result is SoldOut
  derived from EVENTS_3
=
  when e.remaining
    in 0..0 -> SoldOut
    in 1..<20 -> Few
    in 20..5000 -> Open

--- Clamps a raw seat count into the seat range.
pub clamp_seats(raw: Int) -> Seats = if raw < 0 then 0 else if raw > 5000 then 5000 else raw

example "an event without seats is sold out"
  do availability(Event(name: "Launch", remaining: 0))
  expect result is SoldOut

example "an event with many seats is open"
  do availability(Event(name: "Workshop", remaining: 820))
  expect result is Open

example "raw seat counts are clamped"
  do clamp_seats(9000)
  expect result == 5000
```

From this, `una check` proves with Z3 that `availability` meets its `ensures` clause on every path, that its three ranges cover every possible `Seats` value, and that `clamp_seats` always returns a value within `0..5000`. `una test` runs the three examples. `una trace EVENTS_3` connects the requirement to the function, its contract and the status of each proof.

When a contract is false, the compiler says where, why, and with which values. This program from the checkpoint 4 demo (`tests/demo/checkpoint-4/bands.una`) forgets the sold-out band:

```una
module bands

pub kind Availability = SoldOut | Few | Open

--- The bands of the box office: sold out at zero, few below 20, open from 20.
pub availability(remaining: Int) -> Availability
  requires remaining >= 0
  ensures remaining == 0 implies result is SoldOut
=
  if remaining < 20 then Few else Open
```

```text
$ una check tests/demo/checkpoint-4/bands.una
[U5001] Error: postcondition of `availability` is refuted
    ╭─[ bands.una:10:26 ]
    │
  8 │   ensures remaining == 0 implies result is SoldOut
    │           ────────────────────┬───────────────────
    │                               ╰───────────────────── ensures clause
    │
 10 │   if remaining < 20 then Few else Open
    │                          ─┬─
    │                           ╰─── postcondition of `availability` is refuted
    │
    │ Help 1: repair 1 (escape cost 0): add a guard or an arm so that the failing path returns a value for which `remaining == 0 implies result is SoldOut` holds
    │
    │ Help 2: repair 2 (escape cost 0): adjust the `ensures` clause
    │
    │ Note 1: obligation ob-09c568b9c3cf (postcondition) is refuted: remaining == 0 implies result is SoldOut
    │
    │ Note 2: counterexample: remaining = 0, result = Few
    │
    │ Note 3: run `una explain U5001` for an explanation
────╯
```

With `--json`, the same diagnostic is a JSON document with the obligation, the counterexample and ranked repairs, ready for an agent to act on.

## What Una proves, trusts and assumes

Every guarantee has the form: *the implementation satisfies its specification, assuming the trusted base behaves as specified and the stated assumptions hold.* The build report lists the four zones for every build (`docs/00-charter.md §3`):

| Zone | Contents in v0.1 | Who is responsible |
| --- | --- | --- |
| **Proven** | Type safety, effect and capability confinement, information-flow noninterference (termination-insensitive), refinement and contract obligations | The compiler and the verifier |
| **Trusted** | The compiler, the reference interpreter, the SMT solver, the standard library, `trusted` modules, host capability implementations | Una maintainers and module owners |
| **Assumed** | The behavior of external systems as declared in `capability` signatures | Capability authors |
| **Human** | That requirements and contracts capture the intent | Requirement owners and reviewers |

Out of the model, and stated as such in every report: timing, termination and resource side channels, and the behavior of external systems beyond their declared signatures.

The design follows ten principles (`docs/00-charter.md §2`), among them:

- **A function is an equation**: the header is the contract, the body after `=` is the implementation.
- **`@` marks every use of authority**: a line without `@` touches nothing outside the program.
- **Words over symbols**: `implies`, `if error fail`, `expect performed`.
- **One canonical form**: significant indentation and one formatter; non-canonical source is an error.
- **Visible escape hatches**: `checked`, `trusted` modules and declassification are counted and reported.
- **Unknown is not proven**: an obligation the solver cannot decide is an error, never a pass.

## The language at a glance

The full tour is [`docs/guide/una-for-agents.md`](docs/guide/una-for-agents.md), a compact reference whose every code block is checked by the test suite. The normative definition is in [`docs/spec/`](docs/spec/).

### Kinds and refinements

Records and sums are nominal types; aliases and refinements are their base type with a predicate that the solver proves wherever a value flows in.

```una
--- A list with at least one entry.
pub kind NonEmpty[T] = List[T] where it.len > 0
```

With it, `pub average(counts: NonEmpty[Int]) -> Int = counts.sum() / counts.len` needs no check at run time: the division obligation `counts.len != 0` is proven from the refinement (`examples/pantry`). Integers are mathematical, with no overflow; `Nat`, `Bool`, `Text`, `Unit`, `List`, `Option`, `Result` and `Hashed` complete the built-in types.

### Contracts and predicates

`requires` and `ensures` are written in a refinement language the solver decides (quantifier-free linear integer arithmetic with uninterpreted functions). `predicate reflect` bodies are visible to the solver; `predicate opaque` bodies are hidden from it and established at run time with `checked`, an escape hatch counted against the module's budget in `una.toml`. Obligations are one of five kinds: refinement, precondition, postcondition, division and exhaustiveness.

### Capabilities and effects

There is no ambient authority. A function acts outside the program only through a capability it declares in `uses`; capabilities pass down to callees by name, `with` rebinds them or installs a handler for one statement, and they are never values. From `examples/redacted`:

```una
--- Lends from the main shelf, or from the backup shelf when the main one cannot.
pub lend_anywhere(member: Member, name: Text, today: Day, days: LoanDays) -> redacted
  requires can_borrow(member)
  ensures result.due == today + days
  uses @main_shelf: Shelf[pick]
  uses @backup_shelf: Shelf[pick]
  fails LendError
=
  let first_try = attempt lend(member, name, today, days) with @shelf = @main_shelf
  when first_try
    is Ok(redacted) -> redacted
    is Err ->
      lend(member, name, today, days) with @shelf = @backup_shelf
        if error pass
```

Operations declare contracts too: an `ensures` on an operation is an assumption about the outside world that the build report lists, and a handler written in Una is verified against it (`examples/greenhouse`):

```una
--- A thermometer. What it reports is assumed, not proven: the build report lists it.
pub capability Sensor
  op read() -> Int
    ensures result >= -40 and result <= 80
```

The host provides `Console`, `Clock`, `Env`, `Files` and `Keys`, and only to `main`, as `una run` grants them.

### Errors are values

A function declares at most one error kind with `fails`, and every line that can fail is handled on the lines below it:

| Clause | Meaning |
| --- | --- |
| `if error fail V(...)` | any failure becomes `V` |
| `if error pass` | propagate the failure unchanged |
| `if error use e` | recover with the value `e` |
| `if none fail V(...)` | `None` becomes the failure `V` |
| `if none use e` | `None` becomes `e` |

One effect per line: a capability operation or a fallible call is the whole expression of its line, so every effect and every failure has a visible place. `attempt` turns a failure into a `Result` value; when it is `Ok`, the callee's `ensures` still hold.

### Information flow

Values carry labels, `public`, `internal`, `confidential`, `secret` or `restricted`, with categories declared in `una.toml`. Labels flow up and never down: branching on a labeled value labels what the branches produce, and each operation argument is a sink bounded by its `accepts` label (`Console.print` accepts `public`). The only way down is a declassification policy in a trusted module, and every call of it appears in the build report (`examples/journal`):

```una
trusted module journal.release

--- How long a text is, in whole hundreds of characters, and nothing of what it says.
pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how much was written without showing what"
  owner journal_owner

hundreds(text: confidential[Private] Text) -> public Nat = text.len / 100
```

The same discipline keeps the phone number of `examples/events` inside `check_in`: only its keyed hash leaves, through the `attendee_key` policy.

```una
--- Looks up a registration by phone, shows its availability, and records it.
--- The phone never leaves this function: only its keyed hash does.
pub check_in(phone: secret[PII] Phone) -> Availability
  uses @registrations: Registrations[find]
  uses @audit: Audit[record]
  uses @keys: Keys[hmac]
  fails CheckInError
=
  let key = attendee_key(phone)
    if error fail Unavailable(reason: "keys")
  let registration = @registrations.find(key)
    if error fail Unavailable(reason: "registrations")
    if none fail NotRegistered
  let status = availability(registration.event)
  @audit.record(key, status)
    if error fail Unavailable(reason: "audit")
  status
```

### Requirements and examples

A `requirement` records a statement with its source (`human`, `policy`, `regulation` or `generated`) and owner; `derived from` traces functions to it, and a generated requirement stays `proposed` until a person activates it. Examples live next to the code: `given` installs handlers, `do` runs one expression, and `expect` checks the result, a failure, or whether an effect was performed. `una test` runs them in the reference interpreter and reports each with its full effect trace, with labeled data redacted.

## The Agent ABI

Una is built to be driven by agents. Every command takes `--json` and prints exactly one JSON document on stdout, with a `schema` field naming its shape; stderr stays empty, and output is byte-identical from run to run.

| Command | Document | Contents |
| --- | --- | --- |
| `una check`, `una fmt --check` | `una.diagnostics/1` | Diagnostics sorted by file, position and code |
| `una obligations` | `una.obligations/1` | Every proof obligation with its status, goal and hypotheses |
| `una test` | `una.test/1` | Every example with its outcome and effect trace |
| `una run` | `una.run/1` | The program's output and effect trace |
| `una contract <module.function>` | `una.contract/1` | A function's header, declared or inferred |
| `una trace <REQUIREMENT_ID>` | `una.trace/1` | A requirement, its derived functions and their obligations |
| `una report` | `una.report/1` | The build report |
| `una explain <code>` | `una.explain/1` | The explanation of a diagnostic code |

Each diagnostic has a registered code (65 in nine domains: syntax, names, types, authority, security, verification, runtime, project, internal), precise spans and, where the compiler has them, ranked repair candidates. A repair with an `edit` is mechanical, so an agent can apply it without interpretation:

```json
{
  "schema": "una.diagnostics/1",
  "diagnostics": [
    {
      "schema": "una.diagnostic/1",
      "code": "U3003",
      "severity": "error",
      "domain": "authority",
      "message": "`stamp` uses `@clock: Clock[now]`, but this code declares no `uses`, so its effects would exceed its header",
      "primary": { "file": "stamp.una", "start": { "line": 13, "col": 11 }, "end": { "line": 13, "col": 18 } },
      "related": [],
      "repairs": [
        {
          "rank": 1,
          "escape_cost": 0,
          "description": "declare `uses @clock: Clock[now]` in the header",
          "edit": { "file": "stamp.una", "start": { "line": 11, "col": 19 }, "end": { "line": 11, "col": 19 }, "replacement": "\n  uses @clock: Clock[now]" }
        }
      ],
      "explain": "una explain U3003"
    }
  ]
}
```

Exit codes: `0` success; `1` errors in the program, an unproven obligation or a failed example; `2` a usage error, a problem of the invocation reported on stderr without a diagnostic document; `3` an internal compiler error (U8001), which is always a bug. Problems of `una.toml` (U7002) and `verification.lock` (U7004) are diagnostics and exit with `1`.

### The build report

`una report` gives a reviewer the whole package in one page: what is proven, what authority each function holds, where information is declassified and why, which escape hatches are used, what is assumed about the outside world, what code is trusted, how requirements are traced, and how much of the public surface has contracts. Contracts that say nothing, such as `ensures result == result`, are found by the solver and counted as missing.

```text
$ una report examples/events
package events-example 0.1.0
lock: z3 5.1.0, sha256 3ae01f54b3b40760c00b7537ac3ba510a2288a5a4dd42a583dab76ea66719c3c
proven: 11 semantic obligations; security and authority by typing
obligations: 11 proven, 0 refuted, 0 unknown
authority:
  events.checkin.check_in (pub): @registrations: Registrations[find], @audit: Audit[record], @keys: Keys[hmac]
  events.keys.hmac_phone (private): @keys: Keys[hmac]
  events.main.main (pub): @console: Console[print]
information:
  events.keys.attendee_key at src/events/checkin.una:30:13, purpose "attendee lookup and audit correlation", owner privacy
escapes:
  events.types: 1 checked, budget 1
assumptions:
  events.checkin.Audit: record
  events.checkin.Registrations: find
tcb:
  trusted modules: events.keys (17 lines)
  host capabilities: Console, Keys
  solver: z3 5.1.0
requirements:
  EVENTS_3 (active): events.seating.availability
coverage: 5 pub functions, 1 with ensures, 1 with derived from, 0 trivial ensures
out of model: timing, termination, resource consumption, external systems beyond declared signatures
```

## Quick start

### Requirements

- The Rust toolchain pinned in `rust-toolchain.toml` (1.98.1), installed through [rustup](https://rustup.rs).
- Z3 at the exact version pinned in `verification.lock` (5.1.0). `scripts/install-z3.sh DIR` downloads that release for Linux (x86_64, aarch64) or macOS (arm64, x86_64), checks its SHA-256 digest and installs it as `DIR/bin/z3`. Without the pinned Z3 on `PATH`, proofs fail with U5003: verification never passes silently.

### Build and try the examples

```bash
git clone https://github.com/una-lang/una.git && cd una
scripts/install-z3.sh "$HOME/.local/z3" && export PATH="$HOME/.local/z3/bin:$PATH"
scripts/check-solver.sh
cargo build --release -p una-cli && export PATH="$PWD/target/release:$PATH"

una check examples/events           # tiers 0 and 1: every check and every proof
una test examples/events            # run the examples, with effect traces
una run examples/hello              # prints "Hello, Una"
una obligations examples/redacted    # every proof obligation and its status
una report examples/journal         # the build report
una explain U5001                   # what a code means, with examples
```

Once `v0.1.0` is released, the archives `una-0.1.0-x86_64-unknown-linux-gnu.tar.gz` and `una-0.1.0-aarch64-apple-darwin.tar.gz` each hold the `una` binary, this readme and the license, with a `.sha256` digest beside them; Z3 is still installed with `scripts/install-z3.sh`.

### Your first package

A package is a directory with `una.toml`, `verification.lock` (copy the one at the repository root) and sources under `src/`, where module `my_app.main` lives in `src/my_app/main.una`:

```toml
[package]
name = "my-app"
version = "0.1.0"
edition = "0.1"

[run]
entry = "my_app.main"
grants = ["console"]
```

```una
module my_app.main

--- Seats left, from 0 to 500; the solver proves every value that flows in.
pub kind Seats = Int in 0..500

--- Half the seats, rounded down; still a Seats.
pub half(s: Seats) -> Seats
  ensures result <= s
=
  s / 2

pub main() -> Unit
  uses @console: Console[print]
=
  let left = half(300)
  @console.print("{left} seats left")

example "half of 7 seats is 3"
  do half(7)
  expect result == 3
```

```text
$ una check && una test && una run
example my_app.main "half of 7 seats is 3" ... ok

1 example: 1 passed, 0 failed
150 seats left
```

`una check` proved four obligations here: `s / 2` stays within `0..500` and is at most `s`, and the literals `300` and `7` are valid `Seats`. A single `.una` file is also a package, with default settings: `una check file.una`.

## Command line reference

```text
una [--json] [--color never|auto|always] <COMMAND>
```

| Command | Purpose | Options |
| --- | --- | --- |
| `una check [path]` | Tier 0 (every check) and tier 1 (every proof) | `--tier 0` skips the solver; `--dump-core <module>` prints Core IR; `--solver-wallclock <s>` |
| `una fmt [path]` | Rewrite files in canonical form | `--check` reports U7001 without writing |
| `una run [path]` | Run `main` of the `[run] entry` module | `--grant <spec>` (repeated), `--max-depth <n>`, `--solver-wallclock <s>` |
| `una test [path]` | Run every example | `--max-depth <n>`, `--solver-wallclock <s>` |
| `una obligations [path]` | List every proof obligation with its status | `--solver-wallclock <s>` |
| `una contract <module.function>` | Print a function's full header, declared or inferred, with its docs and requirements | run in the package directory |
| `una trace <REQUIREMENT_ID>` | Requirement to contract to obligation | run in the package directory; `--solver-wallclock <s>` |
| `una report [path]` | The build report | `--solver-wallclock <s>` |
| `una explain <code>` | Explanation of a diagnostic code | |

`[path]` is a package directory or a single file and defaults to `.`. Grants: `console`, `clock`, `env`, `keys:<file>`, and `files:<path>:read`, `files:<path>:write` or `files:<path>:read,write`; `una.toml` may list them under `[run] grants`, and `una run` refuses a program whose `main` uses a capability that is not granted (U3012).

Project files: `una.toml` has `[package]` (`name`, `version`, `edition = "0.1"`) and optionally `[labels] categories`, `[trusted] modules`, `[escape_budget] checked_per_module` and `[run]` (`entry`, `grants`). `verification.lock` pins the encoding, the solver and its version, the logic, the step budget and the random seed. Proofs are cached under `.una/cache/proofs/`, keyed by the lock and the query, so a second check answers from the cache.

## Example packages

Every package under `examples/` is canonical, checks at tier 1 with every obligation proven, and passes its examples; together they contain every construct of the v0.1 scope, which a test verifies.

| Package | Setting | What it shows |
| --- | --- | --- |
| [`hello`](examples/hello) | A greeting | The smallest program: `main` with `Console` |
| [`events`](examples/events) | Event check-in (the reference example) | Range refinements, a postcondition derived from a requirement, capabilities with handlers, a `secret[PII]` phone number that leaves only as a keyed hash through a policy in a trusted module, an opaque predicate established by `checked` |
| [`redacted`](examples/redacted) | A community tool-redacted shelf | Records and sums with payloads, reflect predicates in `requires` and `ensures`, `attempt` under a `with` rebinding, a handler instance in `main`, `Clock` |
| [`pantry`](examples/pantry) | The stock of a shared kitchen | Generic kinds and functions, a generic refinement proving a division, lambdas through method calls (`filter`, `map`, `fold`, `first_where`) |
| [`journal`](examples/journal) | A private journal | A label category, the program counter, a sink label, a declassification policy, `Files` and `Env` under an `internal` program counter |
| [`greenhouse`](examples/greenhouse) | A roof vent controller | Operation contracts, handlers verified against them, a handler with its own `uses`, `with @x = @y`, an inferred private header, a requirement from a regulation |
| [`shortlinks`](examples/shortlinks) | A link shortener | An opaque predicate and `checked` within the escape budget, `Result` in a signature, `if error use`, a generated requirement kept `proposed` |

## How the compiler works

```text
source files
  → una-syntax   lex (INDENT/DEDENT) → parse → syntax tree with trivia
  → una-fmt      canonical form (U7001)                                  tier 0
  → una-check    names, kinds and types, capabilities and effects,
                 failures, information flow, elaboration to Core IR      tier 0
  → una-verify   obligations over Core IR → SMT-LIB → Z3 → statuses      tier 1
  → una-interp   the Una Abstract Machine runs main or the examples
  → una-cli      diagnostics, documents and reports
```

- **Tier 0** covers everything but proofs. **Tier 1**, the default, adds every proof obligation and runs once tier 0 is clean.
- **Core IR** (`docs/spec/09-core-ir.md`) is the small language every surface construct desugars into. The checker elaborates to it, the verifier generates obligations from it, and the interpreter runs it, all against the same spans.
- **Verification** encodes each obligation as its own query in QF_UFLIA, runs the Z3 version the lock pins as a subprocess with the lock's step budget and seed, and reads `proven`, `refuted` with a model, or `unknown` with its reason. Queries are byte-identical across runs, results are cached, and anything the solver cannot decide is an error.
- **The Una Abstract Machine** (`docs/spec/11-dynamic-semantics.md`) evaluates Core IR with unbounded integers and records every effect in a trace; host capabilities are confined to their grants, and traces never show labeled data.

## Repository layout

```text
una/
  crates/
    una-diag/       diagnostics, the code registry, human and JSON rendering, explain texts
    una-syntax/     source files, spans, lexer, parser, syntax tree with trivia
    una-fmt/        the canonical formatter
    una-project/    una.toml, verification.lock, module discovery
    una-core/       Core IR, its printer, refinement-language terms
    una-check/      names, types, effects, capabilities, labels, inference, elaboration to Core IR
    una-verify/     obligations, SMT-LIB encoding, the solver driver, the proof cache, counterexamples
    una-interp/     the Una Abstract Machine, host capabilities, traces, the example harness
    una-report/     reserved for build report assembly (in v0.1, una-cli assembles the report)
    una-cli/        the una binary, the only crate that writes to the terminal
  docs/             charter, specification, ADRs, compiler notes, plans, guide, reviews
  examples/         the seven example packages
  tests/
    conformance/    language conformance cases, one or more per rule
    demo/           the programs of the usage checkpoint demos
  scripts/          Z3 installation and check, release packing, checkpoint demos
  images/           the logo
```

## Development

The rules for working on the compiler are in [`CLAUDE.md`](CLAUDE.md) and [`docs/compiler/conventions.md`](docs/compiler/conventions.md): no `unsafe`; no `unwrap`, `expect` or `panic!` in library crates outside documented invariants; library crates never print; output is deterministic (no hash-map iteration, timestamps or randomness in anything user-visible); every public item is documented; every diagnostic code is registered with an explain text.

```bash
cargo build --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
PATH="$HOME/.local/z3/bin:$PATH" cargo test --workspace
cargo test -p una-cli --test conformance -- --nocapture   # prints the conformance summary
```

Tests that reach tier 1 need the pinned Z3 on `PATH`; without it they fail with U5003 rather than skip.

| Layer | Where | What it checks |
| --- | --- | --- |
| Unit | `crates/*/src/` | Lexer states, unification, formula printing, the SMT encoding |
| Conformance | `tests/conformance/` | Each rule of the specification, positive and negative, with expectations in directive comments (`docs/compiler/testing.md`) |
| Snapshots | `crates/*/tests/snapshots/` | Syntax trees of the examples, Core IR, SMT queries, JSON documents; review each change, never accept one blindly |
| Properties | `crates/una-fmt/tests/` | Formatting is idempotent and preserves every `.una` file of the repository |
| End to end | `crates/una-cli/tests/` | The CLI, the ABI documents, the runtime, verification, the examples, the guide and this readme's code, the explain examples, determinism, the checkpoint demos |

CI (`.github/workflows/ci.yml`) runs formatting, clippy and the tests on Linux for every push, and also on macOS for pull requests and pushes to `main`; each push also packs the Linux release archive and runs it on the examples. Pushing a tag `v*` starts `.github/workflows/release.yml`, which builds the Linux and macOS archives with `scripts/release.sh`, checks them with `scripts/check-release.sh`, and attaches them to a draft release.

Each usage checkpoint has a demo script that prints every command with its expected and actual outcome:

```bash
cargo build -p una-cli
scripts/demo/checkpoint-1.sh    # read: syntax diagnostics, formatting
scripts/demo/checkpoint-2.sh    # understand: name, type, authority and information-flow errors
scripts/demo/checkpoint-3.sh    # run: programs, examples, traces, Core IR
scripts/demo/checkpoint-4.sh    # trust: proofs, counterexamples, obligations, the report
```

## Documentation

| Document | Purpose |
| --- | --- |
| [`docs/00-charter.md`](docs/00-charter.md) | What Una is, what it proves and assumes, the v0.1 scope. Read first |
| [`docs/spec/`](docs/spec/) | The normative specification, 15 chapters, with RFC 2119 keywords |
| [`docs/guide/una-for-agents.md`](docs/guide/una-for-agents.md) | A compact, tested reference for agents writing Una |
| [`docs/adr/`](docs/adr/) | Architecture decision records: why each key decision was made |
| [`docs/compiler/`](docs/compiler/) | Architecture, conventions and testing of the reference compiler |
| [`docs/plan/`](docs/plan/) | Roadmap, v0.1 milestones, deferred backlog, milestone plans |
| [`docs/reviews/`](docs/reviews/) | Design review triage, the decision log, the status report |
| [`docs/README.md`](docs/README.md) | The documentation map and reading order |

| Specification chapter | Defines |
| --- | --- |
| [`01-lexical-structure`](docs/spec/01-lexical-structure.md) | Characters, tokens, indentation, keywords, literals, operators |
| [`02-grammar`](docs/spec/02-grammar.md) | Surface syntax in EBNF |
| [`03-types`](docs/spec/03-types.md) | Kinds, generics, refinements, predicates, labels, subtyping |
| [`04-functions-and-contracts`](docs/spec/04-functions-and-contracts.md) | Declarations, clauses, calls, visibility, `main` |
| [`05-capabilities-and-effects`](docs/spec/05-capabilities-and-effects.md) | Capabilities, `uses`, `with`, handlers, host capabilities |
| [`06-errors`](docs/spec/06-errors.md) | Error kinds, `fail`, handling clauses, `attempt`, `checked` |
| [`07-information-flow`](docs/spec/07-information-flow.md) | Labels, the program counter, sinks, policies, trusted modules |
| [`08-requirements-and-examples`](docs/spec/08-requirements-and-examples.md) | Requirements, provenance, examples |
| [`09-core-ir`](docs/spec/09-core-ir.md) | Core IR and the desugaring of surface syntax |
| [`10-static-semantics`](docs/spec/10-static-semantics.md) | The formal typing rules |
| [`11-dynamic-semantics`](docs/spec/11-dynamic-semantics.md) | The Una Abstract Machine, evaluation rules, traces |
| [`12-verification`](docs/spec/12-verification.md) | Obligations, the decidable fragment, the SMT encoding, statuses, the lock |
| [`13-diagnostics-and-abi`](docs/spec/13-diagnostics-and-abi.md) | Diagnostic codes, JSON schemas, the command line, the build report |
| [`14-standard-library`](docs/spec/14-standard-library.md) | The prelude and the host capabilities of v0.1 |
| [`15-canonical-form`](docs/spec/15-canonical-form.md) | The canonical form: lines, spacing, blank lines, comments, U7001 |

When documents disagree, the specification wins, then accepted ADRs, then the compiler documents, then the plans.

## Roadmap

v0.1 exists to test a thesis, not to be fast. **Gate 1** compares agents working on the same tasks in TypeScript, Rust and Una, with the same model and tool budget: Una passes when it shows fewer security and specification violations, no more repair iterations, and faster detection of dangerous changes in review, each by a margin fixed before the runs (`docs/plan/roadmap.md §3`). The example packages and the guide are the starting material for its tasks.

| Version | Theme | Main content |
| --- | --- | --- |
| **v0.1** | Una Core | This repository: the language, the verifier and the Agent ABI |
| v0.2 | Systems | A native backend refining the abstract machine, fixed-width integers with overflow obligations, `edit` and `take` parameter conventions, regions, profiles |
| v0.3 | Verification and review | Laws and property tests, semantic diff, full escape governance, a requirement approval workflow |
| v0.4 | Platform | External state semantics, services, typed data access, API and infrastructure generation |

Each version starts only after the previous gate passes; deferred items and their target versions are in [`docs/plan/backlog.md`](docs/plan/backlog.md). A formal track mechanizes Una Core in Lean 4, and an evaluation track builds the Gate 1 benchmark suite in a separate repository.

The four usage checkpoints of v0.1 are all reached:

| Checkpoint | After | What a user can do |
| --- | --- | --- |
| 1. Read | M1, M2 | Write `.una` files, get syntax diagnostics with exact positions, format code |
| 2. Understand | M3 to M6 | Get name, type, authority and information-flow errors, and read inferred contracts |
| 3. Run | M7 | Write `main`, check it and run it; run the examples with effect traces |
| 4. Trust | M8 to M10 | Proven contracts with counterexamples, the obligation listing and the build report |

## Contributing

The specification comes first. Work follows the milestones in order, with conformance cases written before the implementation, and changes to the language go through the ADR process: write `docs/adr/NNNN-short-title.md` from the template with status `Proposed`, and implement it only once it is `Accepted`, updating the specification in the same change. Keywords reserved for later versions are rejected with U0103 rather than half-implemented, and nothing in the backlog is built before its version. Documentation and diagnostics follow the conventions in [`CLAUDE.md`](CLAUDE.md): American English, normative text with MUST and SHOULD, and one-sentence diagnostic messages that name the construct.

## License

Una is licensed under the Apache License, Version 2.0; see [`LICENSE`](LICENSE).
