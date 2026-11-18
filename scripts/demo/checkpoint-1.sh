#!/bin/sh
# Checkpoint 1, "Read" (docs/plan/roadmap.md §4, reached after M1 and M2): write .una files,
# get syntax diagnostics with exact positions, and format code. For each command, prints the
# command, its expected outcome and its actual outcome, and exits 1 when any outcome differs.
#
# Usage: scripts/demo/checkpoint-1.sh [UNA]
# UNA is the una binary; it defaults to target/debug/una, built by `cargo build -p una-cli`.
# Temporary files go to target/demo/checkpoint-1.
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
una=${1:-$root/target/debug/una}
cd "$root"
if [ ! -x "$una" ]; then
    echo "error: $una not found; build it with cargo build -p una-cli" >&2
    exit 2
fi
demo=tests/demo/checkpoint-1
work=target/demo/checkpoint-1
rm -rf "$work"
mkdir -p "$work"
failures=0

# run STATUS TEXT POSITION ARGS...: runs `una ARGS` and compares its exit status with STATUS and
# its output with TEXT and POSITION, which it must contain; empty TEXT means no output at all.
run() {
    expected_status=$1
    expected_text=$2
    expected_position=$3
    shift 3
    echo "\$ una $*"
    if [ -z "$expected_text" ]; then
        echo "expected: exit $expected_status, no output"
    elif [ -z "$expected_position" ]; then
        echo "expected: exit $expected_status, output with: $expected_text"
    else
        echo "expected: exit $expected_status, output with: $expected_text, at $expected_position"
    fi
    set +e
    output=$("$una" "$@" 2>&1)
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

echo "== Checkpoint 1: read (M1, M2)"
echo

echo "-- A well-formed module"
run 0 "" "" check --tier 0 "$demo/hello.una"

echo "-- Syntax diagnostics at exact positions"
run 1 "[U0101] Error: unexpected \`*\`, expected an expression" "unexpected_token.una:4:33" \
    check --tier 0 "$demo/unexpected_token.una"
run 1 "[U0103] Error: \`match\` is not a keyword in Una; use \`when\`" "reserved_keyword.una:4:27" \
    check --tier 0 "$demo/reserved_keyword.una"
run 1 "[U0007] Error: malformed integer literal \`007\`" "malformed_literal.una:4:22" \
    check --tier 0 "$demo/malformed_literal.una"

echo "-- The same diagnostic for agents, as JSON"
run 1 '"code":"U0101"' '"start":{"line":4,"col":33}' \
    check --tier 0 --json "$demo/unexpected_token.una"

echo "-- Formatting"
run 1 "[U7001] Error: line 4 is not in canonical form" "not_canonical.una:4:1" \
    check --tier 0 "$demo/not_canonical.una"
run 1 "[U7001] Error: line 4 is not in canonical form" "not_canonical.una:4:1" \
    fmt --check "$demo/not_canonical.una"
cp "$demo/not_canonical.una" "$work/not_canonical.una"
run 0 "" "" fmt "$work/not_canonical.una"
run 0 "" "" check --tier 0 "$work/not_canonical.una"

echo "-- Explanations"
run 0 "# U0101: Unexpected token" "" explain U0101

if [ "$failures" -ne 0 ]; then
    echo "checkpoint 1: $failures outcome(s) differ from the expected ones"
    exit 1
fi
echo "checkpoint 1: every outcome is as expected"
