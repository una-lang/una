#!/bin/sh
# Checkpoint 4, "Trust" (docs/plan/roadmap.md §4, reached after M8 to M10): contracts proven by
# the solver, a false one refuted with its counterexample, the obligation listing, the build
# report and the trace of a requirement. For each command, prints the command, its expected
# outcome and its actual outcome, and exits 1 when any outcome differs. Tier 1 needs Z3 at the
# version of verification.lock on PATH; release archives are packed by scripts/release.sh.
#
# Usage: scripts/demo/checkpoint-4.sh [UNA]
# UNA is the una binary; it defaults to target/debug/una, built by `cargo build -p una-cli`.
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
una=${1:-$root/target/debug/una}
cd "$root"
if [ ! -x "$una" ]; then
    echo "error: $una not found; build it with cargo build -p una-cli" >&2
    exit 2
fi
# `run_in` changes directory, so the binary's path must not be relative.
una=$(CDPATH='' cd -- "$(dirname -- "$una")" && pwd)/$(basename -- "$una")
demo=tests/demo/checkpoint-4
failures=0

# run STATUS TEXT POSITION ARGS...: runs `una ARGS` and compares its exit status with STATUS and
# its output with TEXT and POSITION, which it must contain; empty TEXT means no output at all.
# run_in DIR ... runs the command in DIR instead of the repository root.
dir=.
run_in() {
    dir=$1
    shift
    run "$@"
    dir=.
}

run() {
    expected_status=$1
    expected_text=$2
    expected_position=$3
    shift 3
    if [ "$dir" = . ]; then
        echo "\$ una $*"
    else
        echo "\$ (cd $dir && una $*)"
    fi
    if [ -z "$expected_text" ]; then
        echo "expected: exit $expected_status, no output"
    elif [ -z "$expected_position" ]; then
        echo "expected: exit $expected_status, output with: $expected_text"
    else
        echo "expected: exit $expected_status, output with: $expected_text, at $expected_position"
    fi
    set +e
    output=$(cd "$dir" && "$una" "$@" 2>&1)
    status=$?
    set -e
    echo "actual:   exit $status"
    if [ -n "$output" ]; then
        printf '%s\n' "$output" | sed 's/^/  | /'
    fi
    ok=yes
    if [ "$status" != "$expected_status" ]; then
        ok=no
    fi
    if [ -z "$expected_text" ]; then
        if [ -n "$output" ]; then
            ok=no
        fi
    else
        case $output in
            *"$expected_text"*) ;;
            *) ok=no ;;
        esac
        case $output in
            *"$expected_position"*) ;;
            *) ok=no ;;
        esac
    fi
    if [ "$ok" = yes ]; then
        echo "result:   as expected"
    else
        echo "result:   DIFFERENT"
        failures=$((failures + 1))
    fi
    echo
}

echo "== Checkpoint 4: trust (M8 to M10)"
echo

echo "-- A false ensures is refuted, with the values that break it"
run 1 "[U5001] Error: postcondition of \`availability\` is refuted" "counterexample: remaining = 0, result = Few" \
    check "$demo/bands.una"

echo "-- The fixed version is proven"
run 0 "" "" check "$demo/bands_fixed.una"

echo "-- A proof the solver cannot finish within the lock's step budget is unknown, never a pass"
run 1 "[U5002] Error: postcondition of \`total\` could not be proven" "src/budget.una:6:11" \
    check "$demo/budget"

echo "-- A solver other than the pinned one is refused"
run 1 "[U5003] Error: the solver is z3" "verification.lock pins z3 4.8.0" check "$demo/other-solver"

echo "-- Every obligation of the reference example, with its status"
run 0 "postcondition src/events/seating.una:23:11 e.remaining < 1 implies result is SoldOut" \
    "exhaustive    src/events/seating.una:26:3" obligations examples/events

echo "-- The same listing for agents"
run 0 '"schema":"una.obligations/1"' '"status":"proven"' --json obligations examples/events

echo "-- The build report: proofs, authority, information flow, trust and coverage"
run 0 "events.keys.attendee_key at src/events/checkin.una:37:21" \
    "coverage: 5 pub functions, 1 with ensures, 1 with derived from, 0 trivial ensures" \
    report examples/events

echo "-- A contract that says nothing is counted as a missing one"
run 0 "coverage: 1 pub functions, 0 with ensures, 0 with derived from, 1 trivial ensures" "" \
    report "$demo/trivial.una"

echo "-- From a requirement to its functions and their proofs"
run_in examples/events 0 "requirement EVENTS_3 (active)" \
    "ensures e.remaining < 1 implies result is SoldOut" trace EVENTS_3

echo "-- Explanations"
run 0 "# U5001: Obligation refuted" "" explain U5001

if [ "$failures" -ne 0 ]; then
    echo "checkpoint 4: $failures outcome(s) differ from the expected ones"
    exit 1
fi
echo "checkpoint 4: every outcome is as expected"
