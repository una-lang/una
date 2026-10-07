#!/bin/sh
# For each runnable case of this review: the diagnostic codes of `una check` (full tiers), and
# what `una run` prints for S="" and S="x" (the secret's length 0 or 1), with $UNA.
set -u
here=$(cd "$(dirname "$0")" && pwd)
for dir in "$here"/run/*/; do
  name=$(basename "$dir")
  codes=$(cd "$dir" && "$UNA" check . 2>&1 | grep -oE '^\[U[0-9]+\]' | tr -d '[]' | sort | uniq -c | awk '{printf "%s%s ", $2, ($1>1?"x"$1:"")}')
  if [ -z "$codes" ]; then
    out0=$(cd "$dir" && S="" "$UNA" run . 2>&1 | tr '\n' ' ')
    out1=$(cd "$dir" && S="x" "$UNA" run . 2>&1 | tr '\n' ' ')
    printf '%-28s ok   S="" -> %-6s S=x -> %s\n' "$name" "$out0" "$out1"
  else
    printf '%-28s %s\n' "$name" "$codes"
  fi
done
