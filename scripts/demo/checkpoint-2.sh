#!/bin/sh
# Checkpoint 2, "Understand" (docs/plan/roadmap.md §4, reached after M3 to M6): get name, type,
# authority and information-flow errors, and read contracts. For each command, prints the
# command, its expected outcome and its actual outcome, and exits 1 when any outcome differs.
#
# Usage: scripts/demo/checkpoint-2.sh [UNA]
# UNA is the una binary; it defaults to target/debug/una, built by `cargo build -p una-cli`.
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
una=${1:-$root/target/debug/una}
cd "$root"
if [ ! -x "$una" ]; then
    echo "error: $una not found; build it with cargo build -p una-cli" >&2
    exit 2
fi
demo=tests/demo/checkpoint-2
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

echo "== Checkpoint 2: understand (M3 to M6)"
echo

echo "-- A module with a capability, a handler and an example"
run 0 "" "" check --tier 0 "$demo/clean.una"

echo "-- Names and types"
run 1 "[U1001] Error: cannot find name \`summ\`" "unresolved_name.una:3:35" \
    check --tier 0 "$demo/unresolved_name.una"
run 1 "[U2001] Error: \`label\` returns \`Text\`, found \`Int\`" "type_mismatch.una:3:33" \
    check --tier 0 "$demo/type_mismatch.una"

echo "-- Authority: a missing \`uses\`, and \`@\` in a pure function"
run 1 "[U3001] Error: \`note\` needs \`@log: Log[write]\`, which is not in scope here" \
    "missing_uses.una:14:3" check --tier 0 "$demo/missing_uses.una"
run 1 "[U3004] Error: \`@log\` is used here, but this code declares no \`uses\`" \
    "pure_function.una:6:24" check --tier 0 "$demo/pure_function.una"

echo "-- The same diagnostic for agents, as JSON, with its mechanical edit"
run 1 '"code":"U3001"' '"replacement":"\n  uses @log: Log[write]"' \
    check --tier 0 --json "$demo/missing_uses.una"

echo "-- Information flow: a secret value printed to Console"
run 1 "[U4003] Error: \`@console.print\` accepts \`public\` data, and this argument is \`secret\`" \
    "secret_to_console.una:6:18" check --tier 0 "$demo/secret_to_console.una"

echo "-- Project files"
run 1 "[U7002] Error: unknown key \`license\` in [package]" "una.toml:5:1" \
    check --tier 0 "$demo/bad_manifest"

echo "-- Contracts, declared and inferred"
run_in "$demo/ledger" 0 "uses @journal: Journal[append] -- inferred" "fails LedgerError -- inferred" \
    contract ledger.core.record
run_in "$demo/ledger" 0 '"schema":"una.contract/1"' '"fails":{"kind":"LedgerError","inferred":true}' \
    contract --json ledger.core.record

echo "-- Explanations"
run 0 "# U4003: Sink label exceeded" "" explain U4003

if [ "$failures" -ne 0 ]; then
    echo "checkpoint 2: $failures outcome(s) differ from the expected ones"
    exit 1
fi
echo "checkpoint 2: every outcome is as expected"
