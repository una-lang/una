#!/bin/sh
# usage: cmp349.sh <root>... ; prints codes on main (OLD) and PR (NEW) for each package dir
set -u
codes() { (cd "$2" && "$1" check --tier 0 . 2>&1 | grep -oE '^\[U[0-9]+\]|^error\[U[0-9]+\]' | grep -oE 'U[0-9]+' | sort | uniq -c | awk '{printf "%s%s ", $2, ($1>1?"x"$1:"")}'); }
for root in "$@"; do
  find "$root" -name una.toml | sort | while read -r t; do
    d=$(dirname "$t"); ed=$(grep -oE '"0\.[12]"' "$t" | tr -d '"')
    o=$(codes "$OLD" "$d"); n=$(codes "$NEW" "$d")
    flag=""; [ "$o" != "$n" ] && flag="  <-- changed"
    printf '%-55s %s  old=[%s] new=[%s]%s\n' "${d#$root/}" "$ed" "$o" "$n" "$flag"
  done
done
