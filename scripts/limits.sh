#!/bin/sh
# The limits gate on this machine (ADR 0124 rule 31, docs/release-gates.md §2): `limits` and the
# confinement cases, natively on each shipped target, never under emulation. A solver child past
# its memory limit ends as `memout`, past its processor-time limit by SIGXCPU, past the wall-clock
# guard as `wallclock`, with the parent intact (crates/una-cli/tests/solver_limits.rs, ignored in
# the ordinary run); then the solver child's confinement cases (crates/una-sys/tests/
# solver_confinement.rs). It runs on macOS for the `limits` gate, and in the Linux container of
# scripts/local-ci.sh for `limits-linux-arm64` and `limits-linux` (`scripts/local-ci.sh --linux-arm64
# --limits`). It refuses to run under emulation (the linux/amd64 container on an arm64 Mac, which
# scripts/local-ci.sh marks with UNA_LOCAL_CI_SERIAL, or a process translated by Rosetta), where no
# seccomp filter installs (ADR 0124 probe P1, #1442), so its evidence would not be native.
#
# Usage: scripts/limits.sh
set -u

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root" || exit 1

fail() {
    echo "limits: FAIL ($1)" >&2
    exit 1
}

translated=$( (sysctl -n sysctl.proc_translated 2>/dev/null) || echo 0)
if [ -n "${UNA_LOCAL_CI_SERIAL:-}" ] || [ "$translated" = 1 ]; then
    fail "this machine runs $(uname -sm) under emulation; the limits gate runs natively on each target (ADR 0124 rule 31)"
fi
UNA_Z3_LIB=$(scripts/z3-library.sh) || fail "the Z3 library of supply-chain/z3.lock"
export UNA_Z3_LIB
echo "== limits on $(uname -sm)"
cargo test --release --locked -p una-cli --test solver_limits -- --ignored --nocapture \
    || fail "the solver limits"
echo "== the solver child's confinement on $(uname -sm)"
cargo test --locked -p una-sys --features solver --test solver_confinement -- --nocapture \
    || fail "the solver child's confinement"
echo "limits: PASS ($(uname -sm))"
