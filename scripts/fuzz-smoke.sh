#!/bin/sh
# A smoke run of the fuzz targets of fuzz/ (#1430), for the `fuzzing` check of the engineering
# harness: the targets, flags, seed and steps of the `sanitize` release gate's libFuzzer step
# (scripts/fuzz-targets.sh, which scripts/sanitize.sh sources too), at about a twentieth of its
# counts. It is not the release gate: scripts/sanitize.sh keeps the full counts and floors.
#
# It builds the targets with the checking toolchain of scripts/check-toolchain.toml, libFuzzer's
# coverage flags, AddressSanitizer and `--cfg una_fuzz`; checks that the targets are exactly
# those listed, each instrumented with AddressSanitizer active; runs the canary, whose planted
# crash behind `una!` must be found, kept and minimized; then runs every target from its
# committed seeds only, with the fixed seed, for its count divided by FUZZ_SMOKE_DIVISOR but at
# least FUZZ_SMOKE_MIN inputs. It fails on a crash, an AddressSanitizer report, a panic, an input
# over 10 s or 4 GiB, a target that ran fewer inputs than asked, or coverage below the smoke
# floors: each target's counters floor unchanged (the counters of the code libFuzzer loads do
# not depend on the count, so a runtime without libFuzzer's counters still fails), and its `cov`
# and `ft` floors a quarter of the gate's (never below 1), since a shorter run reaches less of them
# (in a first run, `words` reached ft 8,782 of the gate's floor 16,955), while a target that returns
# early on every input still falls below them.
#
# The build and the runs go under $CARGO_TARGET_DIR/fuzz-smoke, or target/fuzz-smoke. A missing
# or different checking toolchain is a FAIL, never a skip. macOS only, like scripts/sanitize.sh.
#
# Usage: scripts/fuzz-smoke.sh
set -u

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

fail() {
    echo "fuzz-smoke: FAIL ($1)" >&2
    exit 1
}

step() {
    name=$1
    shift
    echo "== $name"
    "$@" || fail "$name"
}

FUZZ_SMOKE_DIVISOR=20
FUZZ_SMOKE_MIN=2000
FUZZ_SMOKE_FLOOR_DIVISOR=4

channel=$(sed -n 's/^channel *= *"\(.*\)"$/\1/p' scripts/check-toolchain.toml)
commit=$(sed -n 's/^commit *= *"\(.*\)"$/\1/p' scripts/check-toolchain.toml)
[ -n "$channel" ] && [ -n "$commit" ] || fail "scripts/check-toolchain.toml names no channel and commit"
have=$(rustc +"$channel" -vV 2>/dev/null | sed -n 's/^commit-hash: //p')
[ "$have" = "$commit" ] \
    || fail "the checking toolchain $channel ($commit) is missing or differs; run scripts/install-check-toolchain.sh"
host=$(rustc +"$channel" -vV | sed -n 's/^host: //p')
[ "$(uname -s)" = Darwin ] || fail "macOS only for now"

. "$root/scripts/fuzz-targets.sh"
fuzz_out=${CARGO_TARGET_DIR:-$root/target}/fuzz-smoke
mkdir -p "$fuzz_out"
fuzz_out=$(CDPATH='' cd -- "$fuzz_out" && pwd)
step "The fuzz targets, with libFuzzer and AddressSanitizer" fuzz_build
fuzz_bin=$fuzz_out/build/$host/release
# A ceiling only, as in the gate: each smoke run takes seconds.
FUZZ_CEILING=300

fuzz_check_listed
fuzz_check_instrumented
fuzz_canary
od -A d -c "$fuzz_out/canary/minimized" | head -1
for entry in $FUZZ_TARGETS; do
    runs=$(( ${entry#*:} / FUZZ_SMOKE_DIVISOR ))
    [ "$runs" -ge "$FUZZ_SMOKE_MIN" ] || runs=$FUZZ_SMOKE_MIN
    fuzz_target "${entry%%:*}" "$runs" "$FUZZ_SMOKE_FLOOR_DIVISOR"
done

echo "fuzz-smoke: PASS ($channel, $host, counts divided by $FUZZ_SMOKE_DIVISOR, at least $FUZZ_SMOKE_MIN)"
