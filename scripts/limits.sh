#!/bin/sh
# The limits gate on this machine (ADR 0124 rule 31, docs/release-gates.md §2): `limits` and the
# confinement cases, natively on each shipped target, never under emulation. A solver child past
# its memory limit ends as `memout`, past its processor-time limit as `cpu` (it ends by SIGXCPU,
# taken on its own thread, and Z3's handler of that signal never runs, #2008, #2268), past the
# wall-clock guard as `wallclock`, with
# the parent intact (crates/una-cli/tests/solver_limits.rs, ignored in the ordinary run); then the solver child's confinement cases (crates/una-sys/tests/
# solver_confinement.rs). It runs on macOS for the `limits` gate, and in the Linux container of
# scripts/local-ci.sh for `limits-linux-arm64` and `limits-linux` (`scripts/local-ci.sh --linux-arm64
# --limits`), which is the release image (#2105): the shipped Z3 library at the glibc floor.
# It refuses to run unless scripts/emulation.sh says this process runs natively (it
# detects the marker of scripts/local-ci.sh, Rosetta on macOS, and on Linux an emulator such as
# Rosetta for Linux or QEMU's user mode), since under emulation no seccomp filter installs (ADR
# 0124 probe P1, #1442) and its evidence would not be native.
#
# Each of the two runs must pass exactly the number of cases below (#1939), read from libtest's
# one `test result:` line, so that a case renamed out of `--ignored`, put behind a cfg or deleted
# fails the gate; crates/una-cli/tests/release_gates.rs holds these counts to the test files.
#
# Usage: scripts/limits.sh
set -u

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root" || exit 1

fail() {
    echo "limits: FAIL ($1)" >&2
    exit 1
}

# The cases each run passes: the three ignored tests of solver_limits.rs, and the tests of
# solver_confinement.rs that this system compiles (the common ones, and those of its cfg).
LIMITS_CASES=3
case $(uname -s) in
    Darwin) CONFINEMENT_CASES=14 ;;
    Linux) CONFINEMENT_CASES=10 ;;
    *) fail "no count of the confinement cases is known for $(uname -s)" ;;
esac

scripts/emulation.sh \
    || fail "this machine runs $(uname -sm) under emulation, or cannot tell; the limits gate runs natively on each target (ADR 0124 rule 31)"
UNA_Z3_LIB=$(scripts/z3-library.sh) || fail "the Z3 library of supply-chain/z3.lock"
export UNA_Z3_LIB
# In the release image (scripts/local-ci.sh --linux-arm64 --limits, #2105), the library the
# cases link must be the one that ships: its digest is the one supply-chain/z3.lock's [library]
# table records for this machine's target, the library of the archive.
if [ -n "${UNA_LIMITS_SHIPPED_LIBRARY:-}" ]; then
    triple=$(uname -m)-unknown-linux-gnu
    shipped=$(sed -n '/^\[library\]$/,$s/^"'"$triple"'" *= *"\([0-9a-f]*\)"$/\1/p' supply-chain/z3.lock | head -n 1)
    [ -n "$shipped" ] || fail "supply-chain/z3.lock records no library for $triple"
    linked=$(sha256sum "$UNA_Z3_LIB" | cut -d ' ' -f 1)
    [ "$linked" = "$shipped" ] \
        || fail "the Z3 library $UNA_Z3_LIB is $linked, not the shipped $shipped of supply-chain/z3.lock"
    echo "== the shipped Z3 library: $UNA_Z3_LIB, sha256 $linked, glibc $(getconf GNU_LIBC_VERSION)"
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/una-limits.XXXXXX") || fail "a temporary directory"
trap 'rm -rf "${work:?}"' EXIT

# cases NAME COUNT COMMAND...: runs COMMAND, its output shown as it comes and kept, and passes
# when it exits 0 and its one `test result:` line says `ok. COUNT passed; 0 failed`.
cases() {
    name=$1 count=$2
    shift 2
    rm -f "$work/status" "$work/output"
    { "$@" 2>&1; echo "$?" > "$work/status"; } | tee "$work/output"
    [ "$(cat "$work/status" 2>/dev/null)" = 0 ] || fail "$name"
    results=$(grep -c '^test result: ' "$work/output")
    [ "$results" = 1 ] || fail "$name: $results test result lines, not 1"
    passed=$(sed -n 's/^test result: ok\. \([0-9][0-9]*\) passed; 0 failed;.*/\1/p' "$work/output")
    [ "$passed" = "$count" ] || fail "$name: ${passed:-no} cases passed, not $count"
    echo "== $name: $count cases passed"
}

echo "== limits on $(uname -sm)"
cases "the solver limits" "$LIMITS_CASES" \
    cargo test --release --locked -p una-cli --test solver_limits -- --ignored --nocapture
echo "== the solver child's confinement on $(uname -sm)"
cases "the solver child's confinement" "$CONFINEMENT_CASES" \
    cargo test --locked -p una-sys --features solver --test solver_confinement -- --nocapture
echo "limits: PASS ($(uname -sm))"
