#!/bin/sh
# The leak's scope (N-45): each case of this area, checked by each candidate, and, when it is
# accepted, run with S=secretvalue: LEAK when the secret's length, 11, reaches the output.
# Candidates: V020 (v0.2.0, 2a69456), MAIN (3aed995), PR327 (2e0ff81) and FIX (1b3a5e5), paths to
# `una`.
set -u
here=$(cd "$(dirname "$0")" && pwd)
printf '%-34s %-22s %-22s %-22s %-22s\n' case v0.2.0 main pr327-2e0ff81 pr327-1b3a5e5
for dir in "$here"/*/; do
  name=$(basename "$dir")
  row=$(printf '%-34s' "$name")
  for una in "$V020" "$MAIN" "$PR327" "$FIX"; do
    # A candidate given as `-` is not run: its column comes from an earlier run.
    if [ "$una" = "-" ]; then
      row="$row $(printf '%-22s' '(earlier run)')"
      continue
    fi
    codes=$(cd "$dir" && "$una" check . 2>&1 | grep -oE '^\[U[0-9]+\]' | sort -u | tr -d '[]' | tr '\n' ' ')
    if [ -z "$codes" ]; then
      out=$(cd "$dir" && S=secretvalue "$una" run . 2>/dev/null | tr '\n' ' ')
      case "$out" in
        *11*) cell="LEAK" ;;
        *) cell="accepted" ;;
      esac
    else
      cell="rejected $codes"
    fi
    row="$row $(printf '%-22s' "$cell")"
  done
  echo "$row"
done
