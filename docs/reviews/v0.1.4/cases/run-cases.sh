#!/bin/sh
# Checks each case of this review with UNA (tier 0, as the PR's sweep does; U7004 is not
# expected, every case has its verification.lock) and prints its diagnostic codes, or `ok`.
set -u
here=$(cd "$(dirname "$0")" && pwd)
for dir in "$here"/*/*/; do
  name="$(basename "$(dirname "$dir")")/$(basename "$dir")"
  codes=$(cd "$dir" && "$UNA" check --tier 0 . 2>&1 | grep -oE '^\[U[0-9]+\]' | tr -d '[]' | sort | uniq -c | awk '{printf "%s%s ", $2, ($1>1?"x"$1:"")}')
  printf '%-40s %s\n' "$name" "${codes:-ok}"
done
