# The Z3 built into `una`

ADR 0124 rule 1. This directory is the recipe, reviewed like code; `supply-chain/z3.lock` is the pin;
`scripts/build-z3.sh` runs the recipe.

## What is built

From the pinned source tarball, checked against the lock's digest before anything is extracted, two
artifacts from the same objects, defines and flags:

- `lib/libz3shell.a`: Z3's SMT-LIB 2 front end (`src/shell/`) as a static library, whose entry is
  `una_z3_shell_main(argc, argv)` instead of `main`, so that it does not clash with `una`'s own.
  Upstream CMake builds the front end only as the `z3` executable; `shell-library.patch` adds the
  library target and the renamed entry under the define `UNA_Z3_SHELL_LIBRARY`, and changes nothing
  else. `una-z3` calls that entry (rule 2), and `una __solver` runs it exactly as `z3 -in -smt2`.
- `bin/z3`: the standalone front end from the same build, for the `solver-identity` gate (rule 31),
  which compares the solver child's output with it byte for byte.

## Pristine source, patches with digests

The tarball is upstream's tree, unmodified. Every change of ours is a patch file in this directory,
listed in the lock's `[patches]` table with its SHA-256 and applied in that order by the script, after
the archive's digest check and only if the patch's own digest matches. A reviewer audits upstream plus
a readable diff, never a modified archive.

## The settings

- `cmake-options.txt`: the CMake cache settings, in full: Z3's release configuration (`-O3`,
  `-DNDEBUG`, `_EXTERNAL_RELEASE`, `_MP_INTERNAL`, C++20, hidden visibility, position-independent
  code), a static library, no GMP (so nothing under the LGPL is linked), no git information in the
  version text, no bindings, examples, tests or documentation.
- `-ffile-prefix-map`: the script maps the source and build directories to `/z3-src` and `/z3-build`,
  so that the bytes do not depend on where the build ran.
- `-ffp-contract=off` is not used (rule 1): it would be adopted only as a reproducibility choice,
  after the `solver-identity` gate passes with it against the upstream comparison.
- The configure log's `Z3_COMPONENT_CXX_FLAGS` and `Z3_COMPONENT_CXX_DEFINES` lines, which are the
  flags CMake actually adds, go into the build record.

## The toolchain

On Linux, the C++ compiler, CMake and linker of the release image of `scripts/local-ci/` (`Dockerfile.release`: GCC 13 and
binutils 2.40 of gcc-toolset-13, cmake and make from the pinned AlmaLinux 8, glibc 2.28, #2105;
Ubuntu 24.04 before, through 0.5.1); on macOS, Apple clang of the Xcode version that the release
record names. Each version is written into the build record and the validation summary. The macOS
version is recorded, not pinned, so a third party with another Xcode may not reproduce the bytes.
`libstdc++` and `libgcc` are linked statically into `una` on Linux at `una`'s own link, not here.

## Reproducibility and storage

The library is built once per pin or recipe change: `scripts/build-z3.sh --reproducible` runs two
cold builds and fails unless both libraries have the same digest. That digest goes into the lock's
`[library]` table for the target, and the library into the project's release storage under it.
Release builds take it from there by digest, so the `release` and `reproducible` gates check a
digest instead of rebuilding Z3. A cold build takes on the order of 10 to 20 minutes per target
on the gate machine, more under emulation; the build record keeps the measured time.

## Changing the pin

A new Z3 version is a change of the lock reviewed by the source diff between the two tags, a new
mirrored tarball, two cold builds per target, and the `solver-identity` gate on every target. Z3 has
no advisory feed of its own. Z3 is MIT-licensed (`LICENSE.txt` of the tarball), which `una --license`
prints (ADR 0124 step 5).
