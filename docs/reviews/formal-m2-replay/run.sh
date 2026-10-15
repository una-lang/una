#!/bin/sh
# Replays S-01 to S-12 through the formal track's checker (M2): converts each pre-fix release's
# `una check --dump-core` of the finding's repro (dumps/) into una.core/1 and runs
# `una-core-check` on it; then runs the synthetic tests of the instance checking (tests/), and
# the two mutants of labeled instantiation on the test that only instantiation can reject, which
# must catch both. It builds both executables first, so that it never runs stale ones.
#
# Usage: docs/reviews/formal-m2-replay/run.sh
# Then S-04's shapes (M6, `dumps-0.2/`), in edition 0.2, and S-05's (`dumps-s05/`), in edition 0.1.
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
# S-05: edition 0.1's shapes, dumped by v0.1.3, the release before the fix, read with the
# file-state label that v0.3.6's `una core --json` infers for the same repro (dumps-s05/
# file_state.tsv), the one input not taken from the release; then with each fixed label, which
# the README shows is not faithful.
for fs in inferred public internal; do
    echo "== S-05, file_state $fs"
    tail -n +2 "$here/dumps-s05/file_state.tsv" | while IFS="$(printf '\t')" read -r file level _; do
        [ "$fs" = inferred ] || level=$fs
        name=$(basename "$file" .txt)
        python3 -I "$here/dumpcore.py" "$here/dumps-s05/$file" main 0.1 "$level" > "$out/$name.$fs.json"
        echo "== $name"
        "$root/formal/.lake/build/bin/una-core-check" "$out/$name.$fs.json" | cut -f2- || true
    done
done
# S-27: the Core exports (`una --json core`) of the conformance case
# `security/interpolated_record_fields_core_label`, by v0.3.14 (the release before the fix) and by
# the fix (`core-s27/`): the re-check rejects the first, whose `to_text` temporary is labeled
# `public` while it holds a `secret[PII]` field, and accepts the second.
for core in "$here"/core-s27/*.json; do
    echo "== $(basename "$core" .json)"
    "$root/formal/.lake/build/bin/una-core-check" "$core" | cut -f2- || true
done
