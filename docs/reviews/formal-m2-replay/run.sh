#!/bin/sh
# Replays S-01 to S-12 through the formal track's checker (M2): converts each pre-fix release's
# `una check --dump-core` of the finding's repro (dumps/) into una.core/1 and runs
# `una-core-check` on it; then runs the synthetic tests of the instance checking (tests/), and
# the two mutants of labeled instantiation on the test that only instantiation can reject, which
# must catch both. It builds both executables first, so that it never runs stale ones.
#
# Usage: docs/reviews/formal-m2-replay/run.sh
# Then S-04's shapes (M6, `dumps-0.2/`), in edition 0.2.
set -eu
here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$here/../../.." && pwd)
out=$(mktemp -d)
(cd "$root/formal" && lake build una-core-check replay-mutants > /dev/null)
for dump in "$here"/dumps/*.txt "$here"/tests/*.txt; do
    name=$(basename "$dump" .txt)
    module=$(grep -m1 '^module ' "$dump" | cut -d' ' -f2)
    python3 -I "$here/dumpcore.py" "$dump" "$module" 0.1 > "$out/$name.json"
    echo "== $name"
    "$root/formal/.lake/build/bin/una-core-check" "$out/$name.json" | cut -f2- || true
done
echo "== mutants of labeled instantiation"
"$root/formal/.lake/build/bin/replay-mutants" "$out/mutants.S-08.json" | cut -f2-
# S-04 (M6): edition 0.2's shapes, dumped by v0.2.0, the release before the fix.
for dump in "$here"/dumps-0.2/*.txt; do
    name=$(basename "$dump" .txt)
    module=$(grep -m1 '^module ' "$dump" | cut -d' ' -f2)
    python3 -I "$here/dumpcore.py" "$dump" "$module" 0.2 > "$out/$name.json"
    echo "== $name"
    "$root/formal/.lake/build/bin/una-core-check" "$out/$name.json" | cut -f2- || true
done
