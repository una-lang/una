#!/bin/sh
# Checkpoint 3, "Run" (docs/plan/roadmap.md §4, reached after M7): write `main`, check it and run
# it; run the examples with their effect traces; read the Core IR. For each command, prints the
# command, its expected outcome and its actual outcome, and exits 1 when any outcome differs.
#
# Usage: scripts/demo/checkpoint-3.sh [UNA]
# UNA is the una binary; it defaults to target/debug/una, built by `cargo build -p una-cli`.
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
una=${1:-$root/target/debug/una}
cd "$root"
if [ ! -x "$una" ]; then
    echo "error: $una not found; build it with cargo build -p una-cli" >&2
    exit 2
fi
demo=tests/demo/checkpoint-3
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

echo "== Checkpoint 3: run (M7)"
echo

echo "-- The first program"
run 0 "Hello, Una" "" run examples/hello --grant console

echo "-- A program with a kind, recursion, a lambda and interpolation"
run 0 "countdown: 3 2 1 liftoff" "launch: hold, fuel at 97%" \
    run "$demo/countdown.una" --grant console

echo "-- Capabilities are granted before the program starts"
run 1 "[U3012] Error: \`main\` uses \`now\` of \`Clock\`, which neither \`[run] grants\` nor a \`--grant\` flag grants" \
    "ungranted.una:5:3" run "$demo/ungranted.una" --grant console
run 0 "clock started: yes" "" run "$demo/ungranted.una" --grant console --grant clock

echo "-- A failure of main shows its public fields only"
run 1 "[U6001] Error: \`main\` failed with \`Blocked(reason: \"maintenance\")\`" "gate.una:5:5" \
    run "$demo/gate.una" --grant console

echo "-- A recursion that never ends stops at the maximum call depth"
run 1 "[U6003] Error: this call exceeds the maximum call depth of 10000" "deep.una:3:50" \
    run "$demo/deep.una"

echo "-- The examples of the reference example"
run 0 "7 examples: 7 passed, 0 failed" "" test examples/events

echo "-- Their effect traces for agents, with labeled arguments redacted"
run 0 '"instance":"given:keys","capability":"Keys","op":"hmac"' '"data":"<redacted:secret>"' \
    test --json examples/events

echo "-- An expectation that does not hold"
run 1 "shop.una:11: expect result == 27" "it is false, with \`result\` = \`30\`" \
    test "$demo/shop.una"

echo "-- The Core IR that the interpreter runs"
run 0 "fun availability(e: Event) : Availability ensures e.remaining < 1 implies result is SoldOut =" \
    "let %t0: Int = e.remaining in" \
    check --dump-core events.seating examples/events

echo "-- Explanations"
run 0 "# U6001: \`main\` failed" "" explain U6001

if [ "$failures" -ne 0 ]; then
    echo "checkpoint 3: $failures outcome(s) differ from the expected ones"
    exit 1
fi
echo "checkpoint 3: every outcome is as expected"
