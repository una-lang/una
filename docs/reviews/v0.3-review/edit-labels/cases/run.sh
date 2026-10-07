#!/bin/sh
# Runs every case of the review with the candidate's `una` (UNA, by default the worktree's
# debug build): `check` for each, and for a case with `[run]`, the interpreter and the native
# executable, whose outputs and exit statuses are compared. Results go to `results/`. Run the
# whole set with UNA set to a build of the candidate (summary.md).
set -u
here=$(cd "$(dirname "$0")" && pwd)
una=${UNA:-$here/../../../../../target/debug/una}
out=$here/results
mkdir -p "$out"
for dir in "$here"/*/*/; do
  case=$(basename "$dir"); area=$(basename "$(dirname "$dir")")
  name="$area-$case"
  (cd "$dir" && "$una" check . > "$out/$name.check" 2>&1; echo "exit $?" >> "$out/$name.check")
  if ! tail -1 "$out/$name.check" | grep -q 'exit 0'; then
    echo "$name: rejected, $(grep -c '^\[U' "$out/$name.check") diagnostics"
  elif grep -q '^\[run\]' "$dir/una.toml"; then
    (cd "$dir" && "$una" run . > "$out/$name.interp" 2>&1; echo "exit $?" >> "$out/$name.interp")
    (cd "$dir" && "$una" build . > /dev/null 2>&1 && ./target/una/* > "$out/$name.native" 2>&1; echo "exit $?" >> "$out/$name.native")
    # The debug runtime's note of its copies is not output (spec/16 §5 compares stdout).
    grep -v '^note: una-rt (debug build)' "$out/$name.native" > "$out/$name.native.cmp"
    if cmp -s "$out/$name.interp" "$out/$name.native.cmp"; then
      echo "$name: engines agree"
    else
      echo "$name: ENGINES DIFFER"
    fi
    rm -rf "${dir:?}target"
  else
    echo "$name: accepted"
  fi
done
