# Contributing to Una

Thank you for your interest in Una. Reports from people who use the language are the most useful contribution right now: a program the compiler gets wrong, a diagnostic that misleads, a guide that is unclear.

## Reporting

Open an [issue](https://github.com/una-lang/una/issues/new/choose) and pick the template that fits:

- **Bug report**: the compiler crashes, rejects a valid program, or prints the wrong output.
- **Soundness report**: `una check` accepts a program that breaks a guarantee Una claims to prove (types, authority, information flow, contracts). These are the most valuable reports.
- **Question**: anything about the language, the tools or the documentation.

Include the smallest program that shows the problem, the command you ran, what you expected, and the output of `una --version`. If a report could hurt users before it is fixed, such as a way for a program to escape its grants at run time, follow [`SECURITY.md`](SECURITY.md) instead of opening a public issue.

## What is in this repository

The public tree holds the compiler's sources (`crates/`), the specification (`docs/spec/`), the guides (`docs/guide/`), the standard library listing (`docs/std/`), the compiler's design notes (`docs/compiler/`), the test suites (`tests/` and the integration tests of each crate), the scripts that build, validate and release `una` (`scripts/`), the examples, the validation records of each release (`docs/validation/`), the security documents, the formal development (`formal/`), the fuzz targets and the pinned supply-chain inputs. The architecture decision records (`docs/adr/`), the plan (`docs/plan/`) and the review records (`docs/reviews/`) are here too. Where a document names one of those, it says it is kept by the maintainers.

## How work flows

1. **An issue first.** Every change starts as an issue, so its scope is agreed before code is written. One issue per branch and per pull request.
2. **The specification first.** [`docs/spec/`](docs/spec/) defines the language, and when the code and the specification disagree, the specification wins. A change to the language starts as an architecture decision record: copy [`docs/adr/0000-template.md`](docs/adr/0000-template.md) to `docs/adr/NNNN-short-title.md` with status `Proposed`. It is implemented only once it is `Accepted`, and the specification is updated in the same change as the implementation.
3. **Implement**, following [`docs/compiler/conventions.md`](docs/compiler/conventions.md). Write the tests that encode a rule before implementing it: a language rule gets conformance cases in [`tests/conformance/`](tests/conformance/), positive and negative, in the format of [`docs/compiler/testing.md`](docs/compiler/testing.md), and a fix gets a regression test that fails before it.
4. **Check locally**, then open the pull request.

Branches are named `dev/<issue>-<topic>`, for example `dev/6-integration-rule`. Every commit references its issue (`Refs #6`), and the pull request closes it (`Closes #6`, one keyword per issue). Each pull request adds one changelog fragment, `changelog.d/<issue>-<slug>.md`: a single line that is one bullet, `- **What changed** (#<issue>, …). Details.`, and never edits [`CHANGELOG.md`](CHANGELOG.md) itself. Two pull requests then never change the same lines. The maintainers assemble the fragments at each release.

## Building and checking

You need the Rust toolchain pinned in `rust-toolchain.toml` and the Z3 library that `una` links; the README's [Build from source](README.md#build-from-source) explains both (`scripts/z3-library.sh` builds the library from the source `supply-chain/z3.lock` pins). Then:

```bash
eval "$(scripts/z3-library.sh --export)"
cargo build --release -p una-cli -p una-rt
cargo fmt --all --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo nextest run --workspace --locked   # the test suites; `cargo test --workspace --locked` runs them too
```

These commands are known to pass on macOS arm64, the platform of the maintainers' validation; on other platforms `cargo clippy` may report warnings that are not yet fixed. The public tree has no continuous-integration workflow.

### Building on Linux

On Linux, every build of `una`, a development build included, links the C++ runtime of the solver statically, so the toolchain needs GCC's static archives: `libstdc++.a` (on Debian and Ubuntu, the package `libstdc++-<version>-dev` that `g++` installs; on Fedora and RHEL, `libstdc++-static`) and `libgcc.a`. `crates/una-z3/stage.rs` asks for it with the directive `cargo:rustc-link-lib=static:-bundle=stdc++`, and the release build of `scripts/release.sh` adds `-C link-arg=-static-libstdc++ -C link-arg=-static-libgcc`, so the Linux `una` needs no `libstdc++.so.6` (#1956, ADR 0124 rule 1). It still loads glibc and GCC's unwinder `libgcc_s.so.1`, which the Rust standard library links (ADR 0124, amendment of 2026-11-13). `cc -print-file-name=libstdc++.a` prints the archive's path when the toolchain has it, and only its name when it does not.

The release `una` is linked in the image of `scripts/local-ci/Dockerfile`, which installs its packages with apt and no version. Their versions and the files the linker puts into `una` from them are pinned in `supply-chain/z3.lock`:

- `[toolchain.aarch64-unknown-linux-gnu]`: the Debian packages and their versions, as `dpkg-query -W` printed them in the image (binutils, cmake, make, GCC 13, `libstdc++-13-dev`, `libgcc-13-dev`, `libgcc-s1`, `libc6`, `libc6-dev`).
- `[toolchain-files.aarch64-unknown-linux-gnu]`: the SHA-256 of `libstdc++.a`, `libgcc.a`, `libgcc_eh.a`, the start-up objects (`Scrt1.o`, `crti.o`, `crtn.o`, `crtbeginS.o`, `crtendS.o`) and `libc_nonshared.a`.

Two scripts enforce them:

- `scripts/check-toolchain.sh TARGET` compares each package with `dpkg-query -W` and each file, found where `cc -print-file-name=<file>` names it, with its digest, and exits 2 on a missing table, package or file or on another version or digest. `scripts/release.sh` runs it for a Linux target before it builds `una`, so a rebuilt image whose packages changed cannot ship until the lock is updated deliberately. It reads the image's files at the paths `cc` names and does not trace the link. Run it in that image: `scripts/check-toolchain.sh aarch64-unknown-linux-gnu` prints one line `toolchain: <target>: ...` per package and file.
- `scripts/load-commands.sh target/release/una` reads the `NEEDED` entries of `una` with `readelf -d` and fails on any library that `scripts/load-commands.txt` and `scripts/load-commands-una.txt` do not list, so a `una` that needs `libstdc++.so.6` fails. `scripts/load-commands-test.sh` holds its canaries: a program linked with `-l stdc++` must fail as `una`, one linked with `-l static:-bundle=stdc++` must pass. [`docs/release-gates.md`](docs/release-gates.md) describes both in the `build` and `release` gates.

`una` uses no `z3` on `PATH`. Without `UNA_Z3_LIB` the build makes a `una` with no solver, and a check at tier 1 fails with U5003. A pull request is merged after the maintainers' own validation on its final commit; a release is tagged when every gate passes on one commit, or when the maintainer accepts it in the record, whose verdict line says so. Each release's record is in [`docs/validation/`](docs/validation/) and states which gates ran and on which commit, as [`docs/release-gates.md`](docs/release-gates.md) describes.

## Rules for the code

The full list is in [`docs/compiler/conventions.md`](docs/compiler/conventions.md). The most important:

- No `unsafe`, except in the documented boundary module of the native runtime.
- No `unwrap`, `expect` or `panic!` in library crates, except for internal invariants marked `// INVARIANT: <reason>`. Problems a user can cause are diagnostics with a registered code.
- Library crates never print; only `una-cli` writes to the terminal.
- Output is deterministic: no hash-map iteration, timestamps or randomness in anything a user sees.
- Every public item has a doc comment, and every diagnostic code has an entry in the specification and an `una explain` text.
- Choose the algorithm with the best average-case complexity, and state it.

## Writing

Documentation and messages are in English, with American spelling. Normative text uses MUST, MUST NOT, SHOULD and MAY. A diagnostic message is one sentence with a lowercase start and no trailing period, and it names the construct: `capability @audit is not in scope`.

## Conduct

Everyone who takes part in the project follows the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

By contributing, you agree that your contributions are licensed under the Apache License, Version 2.0, the license of the project.
