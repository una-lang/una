#!/bin/bash
# The `tests` gate of the engineering harness, in the tier scripts/gate-tier.sh chooses
# (docs/compiler/testing.md, "Gate tiers").
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
# Usage: scripts/gate-tests.sh [--plan]   (--plan prints the tier and the nextest filterset only)
set -u

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root" || exit 2

NIGHTLY_SENTENCE='the full suite runs nightly and before every merge of a security, soundness or runtime change'

plan=0
[ "${1:-}" = --plan ] && plan=1

tier_out=$(scripts/gate-tier.sh) || tier_out=$'full\nreason: scripts/gate-tier.sh failed'
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

header() {
    echo "gate tier: $tier"
    printf '%s\n' "$reasons" | sed '/^$/d; s/^/reason: /'
}

if [ "$tier" != fast ]; then
    header
    if [ "$plan" = 1 ]; then exit 0; fi
    set -x
    cargo build --locked -p una-rt || exit $?
    cargo nextest run --workspace --locked --no-fail-fast || exit $?
    cargo test --workspace --doc --locked
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
trap 'rm -f "$log"' EXIT
{
    echo "+ cargo build --locked -p una-rt"
    cargo build --locked -p una-rt
} >"$log" 2>&1 || { header; cat "$log"; exit 1; }
all_count=$(cargo nextest list --workspace --locked --message-format oneline 2>>"$log" | grep -c .)
fast_count=$(cargo nextest list --workspace --locked --message-format oneline -E "$expr" 2>>"$log" | grep -c .)
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
set -x
cargo nextest run --workspace --locked --no-fail-fast -E "$expr" || exit $?
cargo test --workspace --doc --locked
