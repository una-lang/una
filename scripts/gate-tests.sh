#!/bin/bash
# The `tests` gate of the engineering harness, in the tier scripts/gate-tier.sh chooses
# (docs/compiler/testing.md, "Gate tiers").
#
# Both tiers first run scripts/gate-tier-selftest.sh, which holds the classification to its cases.
#
# Trust: the harness does not run this file from the pull request. Its `tests` check runs a launcher
# outside the repository that copies this script, gate-tier.sh and gate-tier-selftest.sh as origin/main
# holds them into a temporary directory and runs them there, with UNA_GATE_ROOT naming the worktree
# and UNA_GATE_SCRIPTS_FROM saying which copy runs (the worktree's own copy only while origin/main has
# none). This script calls its siblings from its own directory. A change to any of the three scripts
# is full (origin/main's rules say so), and the full tier then also runs the worktree's own self-test.
#
# full: cargo build --locked -p una-rt
#       cargo nextest run --workspace --locked --no-fail-fast
#       cargo test --workspace --doc --locked
# fast: the same three steps, with nextest limited to the fixed subset below plus the tests the
#       change needs (the `extra:` lines of gate-tier.sh). The fixed subset is every test except
#       the long suites of gate-tier.sh's `skipped:` line and the exclusive timing tests (the
#       overrides of .config/nextest.toml that reserve every slot); doc tests always run.
#
# The first lines of the output state the tier and its reasons; for fast, the number of tests the
# subset leaves out (the difference of two `cargo nextest list` counts) and that the full suite runs
# nightly and before every merge of a security, soundness or runtime change. The steps stop at the
# first failure, as the harness does with `steps`.
#
# Time budget: the harness gives the check one timeout for the whole script and, when it expires,
# kills only this shell. A watchdog therefore stops every step of this script (the whole process
# tree below it) after UNA_GATE_TESTS_BUDGET seconds (default 5100, under the check's timeout of
# 5400 s), so that no cargo or nextest process outlives the gate; the gate then fails.
#
# Usage: scripts/gate-tests.sh [--plan]   (--plan prints the tier and the nextest filterset only)
set -u

here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=${UNA_GATE_ROOT:-$(CDPATH='' cd -- "$here/.." && pwd)}
export UNA_GATE_ROOT=$root
cd "$root" || exit 2

NIGHTLY_SENTENCE='the full suite runs nightly and before every merge of a security, soundness or runtime change'

plan=0
[ "${1:-}" = --plan ] && plan=1

kill_tree() {
    local c
    for c in $(pgrep -P "$1" 2>/dev/null); do kill_tree "$c"; done
    kill -TERM "$1" 2>/dev/null
}
watchdog=""
cleanup() {
    [ -n "$watchdog" ] && kill_tree "$watchdog"
    [ -n "${log:-}" ] && rm -f "$log"
}
trap cleanup EXIT
if [ "$plan" = 0 ]; then
    budget=${UNA_GATE_TESTS_BUDGET:-5100}
    bash -c '
        kill_tree() { local c; for c in $(pgrep -P "$1" 2>/dev/null); do [ "$c" = "$$" ] || kill_tree "$c"; done; kill -TERM "$1" 2>/dev/null; }
        sleep "$1"
        echo "gate-tests: over the time budget of $1 s; stopping every step" >&2
        kill_tree "$2"' gate-tests-watchdog "$budget" "$$" &
    watchdog=$!
fi

tier_out=$("$here/gate-tier.sh") || tier_out=$'full\nreason: scripts/gate-tier.sh failed'
tier=$(printf '%s\n' "$tier_out" | sed -n 1p)
reasons=$(printf '%s\n' "$tier_out" | sed -n 's/^reason: //p')
extras=$(printf '%s\n' "$tier_out" | sed -n 's/^extra: //p')
skipped=$(printf '%s\n' "$tier_out" | sed -n 's/^skipped: //p')

# The exclusive timing tests: the union of the filters of the overrides that reserve every slot.
exclusive=""
if [ "$tier" = fast ]; then
    exclusive=$(python3 -I -c '
import sys, tomllib
with open(".config/nextest.toml", "rb") as f:
    cfg = tomllib.load(f)
fs = [o["filter"] for o in cfg.get("profile", {}).get("default", {}).get("overrides", [])
      if o.get("threads-required") == "num-cpus" and o.get("filter")]
print(" | ".join("(" + x + ")" for x in fs))
' 2>/dev/null) || exclusive=""
    if [ -z "$exclusive" ] || [ -z "$skipped" ]; then
        tier=full
        reasons="the exclusive timing tests or the skipped suites could not be read"
    fi
fi

# Runs one step, echoing it first and its wall time after it.
step() {
    local t0=$SECONDS rc
    echo "+ $*"
    "$@"
    rc=$?
    echo "step time: $((SECONDS - t0)) s (exit $rc): $*"
    return $rc
}

header() {
    echo "gate tier: $tier"
    echo "gate scripts: ${UNA_GATE_SCRIPTS_FROM:-the copy in the worktree (run without the launcher)}"
    printf '%s\n' "$reasons" | sed '/^$/d; s/^/reason: /'
}

if [ "$tier" != fast ]; then
    header
    if [ "$plan" = 1 ]; then exit 0; fi
    step "$here/gate-tier-selftest.sh" || exit $?
    # The worktree changes the gate scripts: its own self-test must pass with its own classifier too.
    for f in gate-tier.sh gate-tests.sh gate-tier-selftest.sh; do
        if ! cmp -s "$here/$f" "$root/scripts/$f"; then
            step env UNA_GATE_ROOT="$root" bash "$root/scripts/gate-tier-selftest.sh" || exit $?
            break
        fi
    done
    step cargo build --locked -p una-rt || exit $?
    step cargo nextest run --workspace --locked --no-fail-fast || exit $?
    step cargo test --workspace --doc --locked
    exit $?
fi

expr="(all() - ($skipped) - ($exclusive))"
while IFS= read -r e; do
    [ -n "$e" ] && expr="$expr | ($e)"
done <<EOF
$extras
EOF
expr=$(printf '%s' "$expr" | tr '\n' ' ' | tr -s ' ')

if [ "$plan" = 1 ]; then
    header
    printf '%s\n' "$extras" | sed '/^$/d; s/^/extra: /'
    echo "filterset: $expr"
    exit 0
fi

log=$(mktemp "${TMPDIR:-/tmp}/gate-tests.XXXXXX") || exit 2
t0=$SECONDS
{ step "$here/gate-tier-selftest.sh" && step cargo build --locked -p una-rt; } >"$log" 2>&1 || { header; cat "$log"; exit 1; }
# Listing builds every test binary; the second listing reuses them.
all_count=$(cargo nextest list --workspace --locked --message-format oneline 2>>"$log" | grep -c .)
fast_count=$(cargo nextest list --workspace --locked --message-format oneline -E "$expr" 2>>"$log" | grep -c .)
echo "step time: $((SECONDS - t0)) s: the runtime library, the test binaries and the two listings" >>"$log"
header
if [ "$all_count" -gt 0 ] && [ "$fast_count" -gt 0 ]; then
    echo "tests not run: $((all_count - fast_count)) of $all_count (this tier runs $fast_count)"
else
    echo "tests not run: unknown (cargo nextest list failed)"
    cat "$log"
    exit 1
fi
echo "$NIGHTLY_SENTENCE"
printf '%s\n' "$extras" | sed '/^$/d; s/^/extra: /'
echo "filterset: $expr"
cat "$log"
step cargo nextest run --workspace --locked --no-fail-fast -E "$expr" || exit $?
step cargo test --workspace --doc --locked
