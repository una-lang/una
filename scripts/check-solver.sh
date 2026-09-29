#!/bin/sh
# Prints the version of the z3 found on PATH and checks it against verification.lock
# (spec/12 §5, §9). Exits 1 when z3 is missing or its version differs from the pinned one.
#
# Usage: scripts/check-solver.sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
lock="$root/verification.lock"
solver=$(sed -n 's/^solver *= *"\([^"]*\)"$/\1/p' "$lock")
expected=$(sed -n 's/^solver_version *= *"\([^"]*\)"$/\1/p' "$lock")
if [ "$solver" != z3 ] || [ -z "$expected" ]; then
    echo "error: verification.lock must pin solver \"z3\" and a solver_version" >&2
    exit 2
fi
echo "verification.lock pins z3 $expected"

if ! command -v z3 >/dev/null 2>&1; then
    echo "error: z3 not found on PATH; install z3 $expected with scripts/install-z3.sh DIR and add DIR/bin to PATH" >&2
    exit 1
fi
banner=$(z3 --version)
detected=$(printf '%s\n' "$banner" | sed -n 's/^Z3 version \([0-9][0-9.]*\).*$/\1/p')
echo "detected $banner at $(command -v z3)"
if [ "$detected" != "$expected" ]; then
    echo "error: z3 version mismatch: verification.lock pins $expected, found ${detected:-an unrecognized version}; install it with scripts/install-z3.sh DIR" >&2
    exit 1
fi
echo "ok: z3 $detected matches verification.lock"
