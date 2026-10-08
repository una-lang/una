<p align="center">
  <img src="images/una-logo-wide.png" alt="Una: the verification-first programming language for AI agents. Safe, specified, verifiable, agent-ready." width="100%">
</p>

<p align="center">
  <strong>Agents write the implementation. The compiler proves it conforms to the specification. People review the specification.</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/version-v0.3.2%20Beta-7c5cff" alt="Version v0.3.2, Beta">
  <img src="https://img.shields.io/badge/solver-Z3%205.1.0-4f46e5" alt="Solver: Z3 5.1.0">
  <img src="https://img.shields.io/badge/rust-1.98.1-3b3b58" alt="Rust 1.98.1">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-2e2e48" alt="License: Apache-2.0"></a>
</p>

Una is a verification-first programming language for software that AI agents write. A function's header is its contract: what it requires, what it ensures, which authority it uses (files, the console, the clock) and how sensitive its data is. The compiler proves the body against that header with an SMT solver, and it rejects code whose authority or data flow goes beyond what the header declares. When a proof fails, it gives the counterexample and ranked repairs as JSON that an agent can act on. People review the short, explicit specification; the compiler checks that the implementation meets it.

> Rust makes unsafe code hard to write. Una makes it hard for an agent to write code whose intent, authority and invariants are not explicitly specified.

## Contents

- [Status](#status)
- [Install](#install)
- [A taste](#a-taste)
- [What Una proves, trusts and assumes](#what-una-proves-trusts-and-assumes)
- [Built for agents](#built-for-agents)
- [Learn more](#learn-more)
- [Example packages](#example-packages)
- [Build from source](#build-from-source)
- [Contributing](#contributing)
- [License](#license)

## Status

**v0.3.0 is the Beta**, and v0.3.2, a security patch of it, is the current release. It has:
- the verifier and the reference interpreter;
- the native backend: `una build` compiles a program into an executable that behaves exactly like the interpreter;
- in edition 0.2, a standard library for real programs: maps, text operations, JSON and an HTTP client, every result labeled;
- fixed-width integers;
- the memory model, with `edit`, `take` and `for` loops, and lists and texts updated in place;
- structured concurrency;
- constraints on generics;
- printing of structured values;
- importable standard modules;
- editions 0.1 and 0.2;
- archives for macOS on arm64 and Linux on x86_64 and arm64.

Before its tag, every area of v0.3.0 was reviewed by someone other than its author. Feedback on the Beta will shape the language: open an [issue](https://github.com/una-lang/una/issues/new/choose). Una is a research language: it is not ready for production use. The changes in each release, and the known limits, are in [`CHANGELOG.md`](CHANGELOG.md).

## Install

The installer puts a release in `~/.una`:
- the `una` binary;
- the native runtime that `una build` links;
- Z3 at the version the proofs need.

```bash
curl -fsSL https://raw.githubusercontent.com/una-lang/una/main/install.sh | sh
export PATH="$HOME/.una/bin:$PATH"
una explain U5001   # check that it works
```

What the installer checks:
- It installs only an archive whose SHA-256 digest is listed in that release's validation record, and only when the record's verdict is PASS. The records are listed in [`docs/releases/validation.md`](docs/releases/validation.md).
- Z3 must match the digest its project publishes.
- On any mismatch it installs nothing.

`UNA_VERSION` picks another release, and `UNA_HOME` picks another directory. The installer needs `curl` or `wget`, `tar`, `unzip`, and `shasum` or `sha256sum`. `una build` also needs a C compiler available as `cc`. To uninstall, run `sh install.sh --uninstall`, which removes exactly what the installer recorded.

The [releases page](https://github.com/una-lang/una/releases) also has an archive per platform with its digest beside it: macOS arm64, Linux x86_64 and Linux arm64. Each archive, and an installation, also holds the guides under `docs/guide/`: [Getting started](docs/guide/getting-started.md), the [language guide](docs/guide/syntax-reference.md) and [Una for agents](docs/guide/una-for-agents.md), so an agent can learn Una from the archive alone, with `una explain <code>` for each diagnostic. Keep `lib/` beside `una`, and put Z3 5.1.0 on `PATH` (`scripts/install-z3.sh DIR` installs it). Without the pinned Z3, `una check` fails with U5003: verification never passes silently.

## A taste

From [`examples/wordfreq`](examples/wordfreq), the program of the [getting-started tutorial](docs/guide/getting-started.md). It uses maps and text operations, from v0.3.0:

```una
--- The first letter of a word.
pub initial(word: Text) -> Text
  requires word.len >= 1
  ensures result.len == 1
=
  word.slice(start: 0, end: 1)

--- How often each word occurs, in ascending order of the words.
pub counts(words: List[Text]) -> Map[Text, Int]
=
  let edit counts: Map[Text, Int] = empty_map()
  for word in words
    let seen = counts.get(key: word)
      if none use 0
    edit counts.set(key: word, value: seen + 1)
  counts
```

`una check` proves that `initial` always returns one letter, and that every call to it passes a nonempty word. `una test` runs the examples written next to the code. `una build` compiles the program into a native executable:

```text
$ una build . && ./target/una/wordfreq
built target/una/wordfreq
{"and":2,"bat":1,"cat":1,"hat":1,"the":3}
```

When a contract is false, the compiler says where, why and with which values. This function forgets the sold-out band:

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
$ una check bands.una
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

## What Una proves, trusts and assumes

Every guarantee has the form: *the implementation satisfies its specification, assuming the trusted base behaves as specified and the stated assumptions hold.* `una report` lists four zones for every build:

| Zone | Contents | Who is responsible |
| --- | --- | --- |
| **Proven** | Type safety, confinement of effects and capabilities, information-flow noninterference (termination-insensitive), refinements and contracts | The compiler and the verifier |
| **Trusted** | The compiler, the interpreter and the native runtime, the SMT solver, the standard library, `trusted` modules, host capabilities, the key that authenticates the proof cache | Una maintainers and module owners |
| **Assumed** | The behavior of external systems as declared in `capability` signatures | Capability authors |
| **Human** | That requirements and contracts capture the intent | Requirement owners and reviewers |

Outside the model, as every report says: timing, termination and resource side channels, and the behavior of external systems beyond their declared signatures. Una proves **conformance to an explicit specification**, not that the specification says the right thing.

A few principles of the design:

- **A function is an equation**: the header is the contract, and the body after `=` is the implementation.
- **`@` marks every use of authority**: a line without `@` touches nothing outside the program.
- **Words over symbols**: `implies`, `if error fail`, `expect performed`.
- **One canonical form**: significant indentation and one formatter. Non-canonical source is an error.
- **Visible escape hatches**: `checked`, `trusted` modules and declassification are counted and reported.
- **Unknown is not proven**: an obligation the solver cannot decide is an error, never a pass.

## Built for agents

Every command takes `--json` and prints exactly one JSON document, with a `schema` field naming its shape. Output is the same from run to run. Each diagnostic has a registered code, precise spans and, where the compiler has them, ranked repairs. A repair with an `edit` is mechanical, so an agent can apply it without interpretation:

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

| Command | What it does |
| --- | --- |
| `una check` | Every check and every proof |
| `una test` | Runs the examples, with their effect traces |
| `una run` | Runs `main` with the capabilities it is granted |
| `una build` | Compiles `main` into a native executable |
| `una fmt` | Rewrites source in canonical form |
| `una obligations` | Lists every proof obligation and its status |
| `una contract <module.function>` | Prints a function's header, declared or inferred |
| `una trace <REQUIREMENT_ID>` | Follows a requirement to its functions and proofs |
| `una report` | The build report: what is proven, trusted, assumed and declassified |
| `una explain <code>` | Explains a diagnostic code, with examples |
| `una --version` | The version, the editions it serves and the pinned Z3 |

## Learn more

| Document | What it is |
| --- | --- |
| [Getting started](docs/guide/getting-started.md) | A tutorial, from install to a verified, natively built program |
| [Language guide](docs/guide/syntax-reference.md) | Every construct, with its diagnostics |
| [Una for agents](docs/guide/una-for-agents.md) | A compact reference for agents writing Una; every code block is checked by the tests |
| [Specification](docs/spec/) | The normative definition of the language |
| [Charter](docs/00-charter.md) | What Una is for, what it proves and what it assumes |
| [Design decisions](docs/adr/) | Why each key decision was made |
| [Validation records](docs/releases/validation.md) | How each release was checked, with the digests of its archives |
| [Changelog](CHANGELOG.md) | The changes of each release |
| [Documentation map](docs/README.md) | Every document, in reading order |

## Example packages

Every package under [`examples/`](examples) checks with every obligation proven and passes its examples.

| Package | What it shows |
| --- | --- |
| [`hello`](examples/hello) | The smallest program: `main` with the console |
| [`wordfreq`](examples/wordfreq) | Text, a map and JSON: the program of the tutorial |
| [`events`](examples/events) | Event check-in: range refinements, a postcondition traced to a requirement, a phone number that leaves only as a keyed hash |
| [`toolshelf`](examples/toolshelf) | Records and sums, predicates in contracts, recovering from a failure, the clock |
| [`pantry`](examples/pantry) | Generic kinds and functions, a refinement that proves a division, lambdas |
| [`journal`](examples/journal) | Information flow: labels, a declassification policy, files and the environment |
| [`greenhouse`](examples/greenhouse) | Operation contracts, and handlers verified against them |
| [`shortlinks`](examples/shortlinks) | An escape hatch within its budget, and a generated requirement awaiting review |

## Build from source

You need the Rust toolchain pinned in `rust-toolchain.toml` (installed with [rustup](https://rustup.rs)) and Z3 5.1.0, which `scripts/install-z3.sh` downloads and checks.

```bash
git clone https://github.com/una-lang/una.git && cd una
scripts/install-z3.sh "$HOME/.local/z3" && export PATH="$HOME/.local/z3/bin:$PATH"
cargo build --release -p una-cli -p una-rt && export PATH="$PWD/target/release:$PATH"

una check examples/events    # every check and every proof
una test examples/events     # run the examples, with effect traces
una report examples/journal  # the build report
```

The compiler is a Rust workspace: `una-syntax` parses, `una-check` checks names, types, authority and information flow, `una-verify` turns obligations into SMT queries for Z3, `una-interp` is the reference interpreter, `una-codegen` and `una-rt` make native executables, and `una-cli` is the `una` binary. [`docs/compiler/`](docs/compiler/) describes the architecture.

## Contributing

Bug reports, questions and soundness reports are welcome: open an [issue](https://github.com/una-lang/una/issues/new/choose). A program that `una check` accepts but that breaks a guarantee in the table above is a soundness bug, and it is the most valuable report you can send. [`CONTRIBUTING.md`](CONTRIBUTING.md) explains how changes are made and validated, [`SECURITY.md`](SECURITY.md) how to report a vulnerability privately, and [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) the standards of the community.

## License

Una is licensed under the Apache License, Version 2.0; see [`LICENSE`](LICENSE).
