#!/bin/sh
# Checks every cell of the matrix with $UNA (tier 0) and prints its codes, its expectation, and
# FAIL where a cell expected `reject` checks clean, or a control expected `accept` does not.
set -u
here=$(cd "$(dirname "$0")" && pwd)
fails=0
for dir in "$here"/cases/*/; do
  name=$(basename "$dir")
  expect=$(head -1 "$dir/src/sw.una" | sed -E 's/^-- expect: ([a-z]+):.*/\1/')
  codes=$(cd "$dir" && "$UNA" check --tier 0 . 2>&1 | grep -oE '^\[U[0-9]+\]' | tr -d '[]' | sort | uniq -c | awk '{printf "%s%s ", $2, ($1>1?"x"$1:"")}')
  verdict=ok
  case "$expect" in
    reject|rule) [ -z "$codes" ] && verdict=FAIL ;;
    accept) [ -n "$codes" ] && verdict=FAIL ;;
  esac
  [ "$verdict" = FAIL ] && fails=$((fails + 1))
  printf '%-58s %-7s %-4s %s\n' "$name" "$expect" "$verdict" "${codes:-clean}"
done
echo "FAIL: $fails"
