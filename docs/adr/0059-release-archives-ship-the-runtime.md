# 0059. Release archives ship the native runtime

- Status: Accepted
- Date: 2026-10-02
- Deciders: coordinator under the owner's standing authorization for alpha releases (2026-10-02), on a proposal by Owner (Una-2)
- Resolves: the release of `v0.2.0-alpha.1` (#26)
- Amends: ADR 0035 rule 3

## Context

ADR 0035 rule 3 makes a release archive hold `una`, `LICENSE` and `README.md`. ADR 0055 (NB-1) ships `una` with native support for both targets, and `una build` links every executable with the runtime library `libuna_rt.a` (ADR 0055 rule 5). An archive without that library cannot build anything natively: `una build` stops with "set UNA_RT_LIB".

## Decision

1. **Contents.** `scripts/release.sh <target>` builds `una` and the static library of `una-rt` with `--release --locked`. It writes `una-<version>-<target>.tar.gz`, holding `una`, `lib/libuna_rt.a`, `LICENSE` and `README.md` in a directory of the same name.
2. **Where `una build` looks.** `una build` looks for the runtime in `lib/` beside the `una` binary first. That is where the archive puts it. Then it looks next to the binary, where a Cargo build leaves it, then in `../lib`. `UNA_RT_LIB` overrides all three.
3. **The check.** `scripts/check-release.sh <target>` expects those four entries. It also builds `examples/hello` natively with the unpacked `una`, and runs the executable.
4. The rest of ADR 0035 rule 3 is unchanged: the digest, the targets, the release workflow, and the CI check of the Linux archive.

## Consequences

- An archive builds native executables with no other download beyond the system `cc`. Checking still needs the pinned Z3, as before.
- The archive grows by the size of the runtime library, which holds the Rust standard library.

## Alternatives considered

- **Put `libuna_rt.a` next to `una`.** Not chosen: a `lib/` directory keeps the archive's top level to the binary and its documents, and leaves room for more libraries.
- **Embed the library in the `una` binary, and write it out at build time.** Not chosen: it doubles the binary's size for every command, and it needs the library to exist before `una` builds, which Cargo cannot order without a build script.

## Amendment (2026-10-02)

While GitHub Actions is unavailable, `scripts/local-release.sh` builds the archives instead of the release workflow, with the same contents, `lib/libuna_rt.a` included. ADR 0035's amendment of the same date describes it (#156).

## Amendment (2026-10-31): the runtime is built into `una` (ADR 0124)

Appended when ADR 0124 step 2 (#1207) was implemented, as ADR 0124's Appendix A states. Release archives no longer ship the runtime: it is built into `una` (ADR 0124 rules 12 to 19). Rule 1's archive holds `una`, `LICENSE`, `README.md` and the guides, and no `lib/`; rule 3's check expects those entries and refuses a `lib/` entry. Rule 2's search order holds for development builds only: a release `una` reads no file and no `UNA_RT_LIB` for its runtime. The Alternative "embed the library in the `una` binary, and write it out at build time" is the one taken: `scripts/release.sh` builds the runtime first and `una`'s build script embeds it by digest, so Cargo needs no ordering of its own, and until ADR 0124 step 3 `una build` writes it out into the link's private directory.
