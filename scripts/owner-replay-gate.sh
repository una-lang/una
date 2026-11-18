#!/bin/bash
# The owner-replay release gate (ADR 0122 Amendment 14, spec/30 §14 rule 5, card #1997): the
# replay of an owner's chain checked against the oracle of its contract and against the literal
# reference of the amendment, at every question: on every chain of the bounded model to depth 4,
# on every chain to depth 5 from the narrow alphabet's registration (three keys at threshold 2,
# every other move kept), on seeded walks of depths 6 to 9, and on the
# named rows (the known shapes, the rules of acceptance one by one and the response at the
# floor). Run from the repository root by scripts/release-gates.sh.
#
# Each test is a unit test of una-project, ignored in the ordinary run (which runs the same
# comparison to depth 3, and the named rows), since the model of signatures exists only in that
# crate's test build.
# The test binary is built once in release form and run directly, in its own process group,
# under a supervisor that reports a death by signal as 128 + n and its watchdog as 142, so no
# test can pass by being killed, and no process of the group outlives the gate.
#
# Guards, all required, for each test: exit status 0; `^running 1 test$`; a line beginning
# `test result: ok. 1 passed; 0 failed;`; the summary line recorded below, exactly: for the
# bounded runs `owner-replay: <run> chains <N> responses <M> walks <W> unsupported <U>
# failures 0`, for the named rows `owner-replay: named rows shapes <S> failures 0` (the runs
# are deterministic: a fixed space and fixed seeds); and the test's own coverage assertions,
# which fail it when a counter of the alphabet is zero. A change to the model changes the
# recorded values in the same change, with its reason.
#
# Exhaustive depth 5 on the whole alphabet does not fit the gate's budget; it runs as a soak
# outside the release gates (card #2018), with more seeded walks (the tests ..._to_depth_five
# and ..._on_more_seeded_walks).
#
# Usage: scripts/owner-replay-gate.sh [WATCHDOG_SECONDS]   (default 3600)
# Exit code: 0 pass, 1 fail.

set -u
watchdog=${1:-3600}

# name|summary line, after `owner-replay: `
EXPECTED=(
    "tlog::owner::cases::the_named_rows_hold|named rows shapes 28 failures 0"
    "tlog::owner::cases::the_replay_agrees_with_the_reference_to_depth_four|depth 4 chains 570487 responses 6154835 walks 0 unsupported 2852435 failures 0"
    "tlog::owner::cases::the_replay_agrees_with_the_reference_on_seeded_walks|walks chains 480000 responses 1445987 walks 480000 unsupported 2400000 failures 0"
    "tlog::owner::cases::the_replay_agrees_with_the_reference_to_depth_five_on_the_narrow_alphabet|depth 5 narrow chains 280088 responses 1266843 walks 0 unsupported 1400440 failures 0"
)

fail() {
    echo "owner-replay: FAIL: $*"
    exit 1
}

exe=$(cargo test --release --locked -p una-project --lib --no-run --message-format=json 2>/dev/null |
    python3 -B -c '
import json, sys
found = []
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["kind"] == ["lib"] and m["target"]["name"] == "una_project" and m["profile"]["test"]:
        found.append(m["executable"])
print(found[-1] if found else "")
')
[ -n "$exe" ] && [ -x "$exe" ] || fail "the release test binary of una-project was not built"
echo "owner-replay: the test binary is $exe"

for row in "${EXPECTED[@]}"; do
    IFS='|' read -r name summary <<<"$row"
    log=$(mktemp)
    start=$(date +%s)
    # The supervisor: the test in its own process group; the watchdog kills the whole group
    # (142); a death by signal n is 128 + n; a process of the group left after the test ended
    # is killed and reported (125).
    perl -e '
        $p = fork; die "fork" unless defined $p;
        if (!$p) { setpgrp(0, 0); exec @ARGV; exit 127 }
        $SIG{ALRM} = sub { kill "KILL", -$p; exit 142 };
        alarm '"$watchdog"';
        waitpid($p, 0); $s = $?;
        alarm 0;
        if (kill(0, -$p)) { kill "KILL", -$p; print STDERR "owner-replay: a process of the group outlived the test\n"; exit 125 }
        if ($s & 127) { print STDERR "owner-replay: the model died by signal ", ($s & 127), "\n"; exit 128 + ($s & 127) }
        exit($s >> 8)
    ' "$exe" --include-ignored --exact "$name" --nocapture --test-threads 1 >"$log" 2>&1
    status=$?
    cat "$log"
    echo "owner-replay: $name: exit $status, $(($(date +%s) - start)) s"
    if [ "$status" -eq 142 ]; then
        fail "$name: the ${watchdog}-second watchdog killed the model (process group)"
    elif [ "$status" -eq 125 ]; then
        fail "$name: a process of its group outlived the test"
    elif [ "$status" -gt 128 ]; then
        fail "$name: the model died by signal $((status - 128))"
    elif [ "$status" -ne 0 ]; then
        fail "$name: the test failed (exit $status)"
    fi
    grep -qE '^running 1 test$' "$log" || fail "$name: not exactly one test ran"
    grep -qE '^test result: ok\. 1 passed; 0 failed;' "$log" || fail "$name: no passing test result"
    want="owner-replay: $summary"
    grep -qxF "$want" "$log" || fail "$name: the summary is not \`$want\`: $(grep '^owner-replay: ' "$log" | grep -v ': the' | head -n 1)"
    rm -f "$log"
done
echo "owner-replay: PASS"
