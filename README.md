<p align="center">
  <img src="images/una-logo-wide.png" alt="Una: the verification-first programming language for AI agents. Safe, specified, verifiable, agent-ready." width="100%">
</p>

<p align="center">
  <strong>Agents write the implementation. The compiler proves it conforms to the specification. People review the specification.</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/version-v0.5.2%20Beta-7c5cff" alt="Version v0.5.2, Beta">
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

**v0.3.0 is the Beta**, and v0.5.2, the third increment of release R0.5 (in the edition 0.5 preview, `const` and `var` and module constants; an `Int` of any size written as text; Linux on glibc 2.28 and later), is the current release. It has:
- the verifier and the reference interpreter;
- the native backend: `una build` compiles a program into an executable that behaves exactly like the interpreter;
- in edition 0.2, a standard library for real programs: maps, text operations, JSON and an HTTP client, every result labeled;
- fixed-width integers;
- the memory model, with `edit`, `take` and `for` loops, and lists and texts updated in place;
- structured concurrency;
- constraints on generics;
- printing of structured values;
- importable standard modules;
- editions 0.1 and 0.2, and edition 0.3 as a preview: it has every rule of 0.2, conditions on the operations a function may perform, locked contracts of public functions, the first part of module boundaries, and errors for failure handling on code that cannot fail; later releases add the rest of its designs, so they may reject a package that this one accepts;
- edition 0.4 as a preview, the edition of release R0.4 (below), and edition 0.5, the edition of this release R0.5, as an early preview, with `const` and `var` (ADR 0137) and module constants (ADR 0126);
- archives for macOS on arm64 and Linux on arm64 with glibc 2.28 or later (#2105: tested on Ubuntu 22.04, Debian 12 and AlmaLinux 8; Ubuntu 20.04, Debian 10 and 11 and RHEL 8 and later are inferred from the glibc symbol versions `una` needs; the releases 0.4.0 to 0.5.1 need glibc 2.39 or later), each listed with its digest in the validation record `docs/validation/0.5.2.md`; Linux on x86-64 and Windows are not targets of v0.5.2.

Releases, compiler versions and editions follow one number (ADR 0139): increment R0.N.k of release R0.N ships `una` v0.N.k, the first being R0.N.0 (amendment of 2026-11-20), and edition 0.N is named by that release. v0.4.0 ships edition 0.4, with long-running programs, tasks, HTTP servers and the core of Itá, as a preview: it may still change under the terms of ADR 0121, and a later release makes it stable (ADR 0139, amendment of 2026-11-10, accepted). Edition 0.5 follows with collections, numerics, module constants and `const` and `var`. The plan by release is kept by the maintainers.

Before its tag, every area of v0.3.0 was reviewed by someone other than its author. Feedback on the Beta will shape the language: open an [issue](https://github.com/una-lang/una/issues/new/choose). Una is a research language: it is not ready for production use. Edition 0.4, the edition of v0.4.0, is a preview (ADR 0121): a later release may ask for edits to a package of edition 0.4. The changes in each release, and the known limits, are in [`CHANGELOG.md`](CHANGELOG.md); what this release proves, tests and assumes is in [Known limits](docs/guide/known-limits.md#assurance-in-this-release).

## Install

The installer puts a release in `~/.una`: the `una` binary, with the Z3 solver and the native runtime that `una build` links built into it (ADR 0124). It installs no separate Z3, and `una` never looks for a `z3` on `PATH`.

```bash
curl -fsSL https://raw.githubusercontent.com/una-lang/una/main/install.sh | sh
export PATH="$HOME/.una/bin:$PATH"
una explain U5001   # check that it works
```

The installer is the supported way to install. An archive downloaded by hand in a browser does not run on macOS as it is: see "A macOS archive downloaded in a browser" below.

What the installer checks:
- It installs only an archive whose SHA-256 digest is listed in that release's validation summary, or that [`docs/validation/repacked.md`](docs/validation/repacked.md) pairs with that listed digest (an archive packed again after its release), and only when the summary's verdict is PASS. The summaries are in [`docs/validation/`](docs/validation/), and [`docs/release-gates.md`](docs/release-gates.md) says how they are made.
- On any mismatch it installs nothing.

`UNA_VERSION` picks another release, 0.4.0 or later (an earlier one needs a separate Z3, which the installer no longer installs, so it refuses it), and `UNA_HOME` picks another directory. The installer needs `curl` or `wget`, `tar`, `shasum` or `sha256sum`, and `id`, and on Linux `getconf`. On macOS arm64, `una build` links with the linker built into `una` and runs no other program, so it needs no C compiler. On Linux arm64, `una` needs glibc 2.39 or later: it is a dynamic executable against the system's glibc, with the C++ runtime of the solver (`libstdc++` and `libgcc`) linked into it, and it does not run on musl systems such as Alpine. There `una check`, `una test` and `una run` need no other program, but `una build` links through a C compiler available as `cc` (gcc or clang, with the C library's development files), and the executable it writes is dynamic against the glibc of the machine that built it. The installer refuses, before any download, Linux on x86-64, a musl system, a glibc older than 2.39 and a 32-bit userland on a 64-bit kernel. It reads the C library from the running system (`getconf`, `ldd` and the aarch64 loader `una` needs), so a glibc system that also has musl's loader installed is not refused. As root, the installer installs only into a `UNA_HOME` that only root can change: `UNA_HOME`, the directories it writes in and the files there are not symbolic links, and they are owned by root and writable by no one else, and so is every directory above them, except that a directory above an existing, root-owned one may be writable by others when it is sticky, as `/tmp` is. So `UNA_HOME=/tmp/una` is refused unless `/tmp/una` already exists and is root's. The same holds for `TMPDIR` (default `/tmp`), where the archive is checked and unpacked, which may itself be sticky; uninstall makes no work directory and does not check it. Each file is written as a new one, after its name is removed, so the installer never writes into a file that is a hard link to another one. For every user, a directory, a FIFO or a symbolic link to a directory where the installer writes a file, or a file where it writes in a directory, is refused before anything is written, and so is, for an ordinary user, a directory the installer writes in that the user cannot write in. A regular file or a link at a name the installer writes is replaced; a directory or a special file is never removed to make room. Otherwise it refuses before any download, and as root it never runs the installed `una`. Only owners and mode bits are read: write access granted by an ACL (on macOS, a `+` after the mode in `ls -l`) is not seen. Run again over an earlier installation, the installer removes the files of the earlier record that the new release does not hold, such as the `z3` and `lib/libuna_rt.a` of a release before 0.4.0, and the directories left empty, and names them. To uninstall, run `curl -fsSL https://raw.githubusercontent.com/una-lang/una/main/install.sh | sh -s -- --uninstall`, or `sh install.sh --uninstall` with a local copy of the script. It removes exactly what the installer recorded, and the key of the proof cache that `una` makes at run time, `~/.una/proof-cache.key`, then `~/.una` when nothing else is in it. It removes no other file, it removes a key that is a symbolic link without touching the file the link names, it keeps a key file that `UNA_PROOF_KEY` names, and its message says what it removed and names what is left.

The [releases page](https://github.com/una-lang/una/releases) also has an archive per platform with its digest beside it: from v0.4.0, macOS arm64 and Linux arm64 (releases before v0.4.0 also have Linux x86_64; the installer installs none of those releases, as above). Each archive, and an installation, also holds the guides under `docs/guide/`: [Getting started](docs/guide/getting-started.md), the [language guide](docs/guide/syntax-reference.md), [Una for agents](docs/guide/una-for-agents.md) and its [compact form](docs/guide/una-compact.md), under 2,000 tokens, so an agent can learn edition `0.2` from the archive alone, with `una explain <code>` for each diagnostic; the edition `0.4` pages are in the repository and on the site. An archive whose `una` has the runtime built in holds no `lib/`; with an earlier archive, keep `lib/` beside `una`. Z3 5.1.0 is built into `una`, and `una --version` prints its version. When the solver cannot run, `una check` fails with U5003, for example when it runs as root, which the solver refuses (in a container, run as an ordinary user: `docker run --user 1000:1000 ...`): verification never passes silently.

**A macOS archive downloaded in a browser.** The `una` in the macOS archive is ad hoc signed and is not notarized. A browser marks what it downloads with the `com.apple.quarantine` attribute, and the files unpacked from the archive carry it. macOS does not run that `una`: started from a terminal, the process is killed (exit code 137) and `una` prints nothing. Either use the installer, whose download carries no such attribute, or remove the attribute from the unpacked file with `xattr -d com.apple.quarantine <file>`, where `<file>` is the unpacked `una`.

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

Proven means discharged by the checker and the Z3 solver, relative to the verifier's encoding, and an obligation the solver cannot decide is an error (U5002), never a pass. Noninterference and type safety are enforced by the checker's rules; the formal model in Lean proves them for a fragment of the language only, and `una check` does not run that re-check on your package ([Known limits](docs/guide/known-limits.md), [`docs/security/tcb.md`](docs/security/tcb.md)).

Every report lists what is outside the model, among them: timing, termination and resource side channels, and the behavior of external systems beyond their declared signatures. Una proves **conformance to an explicit specification**, not that the specification says the right thing.

A few principles of the design:

- **A function is an equation**: the header is the contract, and the body after `=` is the implementation.
- **`@` marks every direct use of authority**: an operation on the outside world is written `@name.op(...)`, and a public function's `uses` clause lists everything it and the functions it calls may do.
- **Words over symbols**: `implies`, `if error fail`, `expect performed`.
- **One canonical form**: significant indentation and one formatter. Non-canonical source is an error.
- **Visible escape hatches**: `checked`, `trusted` modules and declassification are counted and reported.
- **Unknown is not proven**: an obligation the solver cannot decide is an error, never a pass.

## Built for agents

Every command but `una lsp` takes `--json` and prints exactly one JSON document, with a `schema` field naming its shape. Output is the same from run to run. Each diagnostic has a registered code, precise spans and, where the compiler has them, ranked repairs. A repair with an `edit` is mechanical, so an agent can apply it without interpretation:

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
| `una test` | Runs the tests (`example` in editions 0.1 and 0.2), with their effect traces |
| `una run` | Runs `main` with the capabilities it is granted |
| `una build` | Compiles `main` into a native executable |
| `una fmt` | Rewrites source in canonical form |
| `una fix` | Applies the mechanical repairs of `una check`, then formats, and reports what remains |
| `una core` | Prints the package's Core IR; with `--json`, as `una.core/1` |
| `una migrate` | Moves a package from `una.toml` to `ita.una` and `ita.lock`, edition 0.4 |
| `una ita <command>` | The package manager of edition 0.4 (local and mirror sources only in this release) |
| `una lsp` | The language server, for editors |
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
| [Known limits](docs/guide/known-limits.md) | What this release does not do, or does only with limits, in edition 0.4, each item with its rule |
| [Serving HTTP](docs/guide/serving-http.md) | Grants, the `serve` section and the language form, stopping, tests and the report of a server, in edition 0.4 |
| [Loops that run until stopped](docs/guide/language/control-flow/for-ever.md) | `for ever`, `for until`, `stop` and `skip`, `diverges`, `on stop` and the host stops (signals, `--deadline`, `--stop-grace`, `--stop-after-iterations`, `--virtual-time`), in edition 0.4 |
| [Driven programs](docs/guide/driven-programs.md) | A loop the host runs: the `drive` section, `init`, `step` and `on_stop`, stopping and testing, in edition 0.4 |
| [Tasks and time](docs/guide/tasks-and-time.md) | `together`, `tasks` and `start`, `within` and `first of`, cancellation, the two engines' schedules and the timing labels, in editions 0.2 and 0.4 |
| [Specification](docs/spec/) | The normative definition of the language |
| [Charter](docs/00-charter.md) | What Una is for, what it proves and what it assumes |
| [Design decisions](docs/adr/) | Why each key decision was made |
| [Release gates](docs/release-gates.md) and [validation summaries](docs/validation/) | How each release was checked, with the digests of its archives |
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
| [`health`](examples/health) | Edition 0.4 preview: a health endpoint, the `serve` section of `ita.una`, tests that call `handle` with `test_request` |
| [`metronome`](examples/metronome) | Edition 0.4 preview: a driven program, the `drive` section of `ita.una`, a state that `step` edits on each tick |

## Build from source

You need the Rust toolchain pinned in `rust-toolchain.toml` (installed with [rustup](https://rustup.rs)), and, to build the Z3 library that `una` links, `cmake`, `make`, a C++20 compiler and `git`; on Linux, the compiler's static `libstdc++.a` too (on Fedora and RHEL, the package `libstdc++-static`), since `una` links the solver's C++ runtime statically there ([`CONTRIBUTING.md`](CONTRIBUTING.md#building-on-linux)). `scripts/z3-library.sh --export` builds that library once per pin and toolchain from the source `supply-chain/z3.lock` pins (10 to 20 minutes), keeps it in `~/.cache/una/z3`, and prints the line that sets `UNA_Z3_LIB` to it; `UNA_Z3_SOURCE` names a local copy of the pinned source tarball when it cannot be downloaded. Run it in every shell that builds `una`: without `UNA_Z3_LIB` the build warns and makes a `una` with no solver, whose `una check` fails with U5003 ([`docs/compiler/testing.md`](docs/compiler/testing.md) §3).

```bash
git clone https://github.com/una-lang/una.git && cd una
eval "$(scripts/z3-library.sh --export)"
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
