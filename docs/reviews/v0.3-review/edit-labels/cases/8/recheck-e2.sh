#!/bin/sh
# E-2 recheck (N-44): with UNA the candidate's `una`, the unsolved shapes give U2001 as on main,
# the function case gives U4001 with an example of the binding's own type, and the annotated
# control checks.
set -u
here=$(cd "$(dirname "$0")" && pwd)
for name in single-flow-closed deep-nested list-of-none map-of-empty-lists functions-unannotated functions-annotated map-of-functions; do
  dir="$here/$name"
  [ -d "$dir" ] || continue
  messages=$(cd "$dir" && "$UNA" check . 2>&1 | grep -E '^\[U' | tr '\n' ' ')
  echo "$name: ${messages:-accepted}"
done
