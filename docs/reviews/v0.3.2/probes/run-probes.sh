#!/bin/sh
# Prints the codes of each probe with $OLD and $NEW (tier 0); a probe with no code is accepted.
set -u
here=$(cd "$(dirname "$0")" && pwd)
codes() { (cd "$2" && "$1" check --tier 0 . 2>&1 | grep -oE '^\[U[0-9]+\]' | tr -d '[]' | sort -u | tr '\n' ' '); }
for dir in "$here"/cases/*/; do
  printf '%-36s old=[%s] new=[%s]\n' "$(basename "$dir")" "$(codes "$OLD" "$dir")" "$(codes "$NEW" "$dir")"
done
