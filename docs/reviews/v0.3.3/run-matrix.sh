#!/bin/sh
# Checks every cell of the #177 matrix with $UNA (tier 0) and, for an accepted cell that runs,
# compares the interpreter (`una run`) with the native executable (`una build`).
#   reject: FAIL when the cell has no security diagnostic (U4xxx), LEAK when it checks clean.
#   accept: FAIL when it does not check clean.
#   rule:   expected to be rejected by a rule that does not depend on the secret.
#   A cell whose diagnostics are only of other kinds (syntax, types) is BROKEN: the case is wrong.
#   NATIVE-DIFF when an accepted cell's interpreter and native outputs, exit codes or traces differ.
set -u
here=$(cd "$(dirname "$0")" && pwd)
fails=0; leaks=0; broken=0; native=0
tmp=$(mktemp -d)
for dir in "$here"/cases/*/; do
  name=$(basename "$dir")
  first=$(head -1 "$dir/src/sw.una")
  expect=$(printf '%s' "$first" | sed -E 's/^-- expect: ([a-z]+):.*/\1/')
  codes=$(cd "$dir" && "$UNA" check --tier 0 . 2>&1 | grep -oE '^\[U[0-9]+\]' | tr -d '[]' | sort | uniq -c | awk '{printf "%s%s ", $2, ($1>1?"x"$1:"")}')
  verdict=ok
  case "$expect" in
    reject)
      if [ -z "$codes" ]; then verdict=LEAK; leaks=$((leaks + 1))
      elif ! printf '%s' "$codes" | grep -q 'U4'; then verdict=BROKEN; broken=$((broken + 1)); fi ;;
    rule) [ -z "$codes" ] && { verdict=FAIL; fails=$((fails + 1)); } ;;
    accept)
      if [ -n "$codes" ]; then verdict=FAIL; fails=$((fails + 1))
      elif ! printf '%s' "$first" | grep -q 'check only'; then
        (cd "$dir" && "$UNA" run . > "$tmp/interp" 2>&1); ri=$?
        if (cd "$dir" && "$UNA" build . --out "$tmp/exe" > "$tmp/build" 2>&1); then
          "$tmp/exe" --trace "$tmp/ntrace" > "$tmp/native" 2>&1; rn=$?
          (cd "$dir" && "$UNA" --json run . > "$tmp/ijson" 2>/dev/null)
          traces=same
          python3 -I -c 'import json,sys; a=json.load(open(sys.argv[1]))["trace"]; b=json.load(open(sys.argv[2])); sys.exit(0 if a==b else 1)' "$tmp/ijson" "$tmp/ntrace" 2>/dev/null || traces=differ
          if [ "$ri" != "$rn" ] || ! cmp -s "$tmp/interp" "$tmp/native" || [ "$traces" = differ ]; then
            echo "    traces: $traces"
            verdict=NATIVE-DIFF; native=$((native + 1))
            echo "    interp ($ri): $(tr '\n' ' ' < "$tmp/interp" | cut -c1-120)"
            echo "    native ($rn): $(tr '\n' ' ' < "$tmp/native" | cut -c1-120)"
          fi
        else
          verdict=BUILD-FAIL; native=$((native + 1))
          echo "    build: $(tr '\n' ' ' < "$tmp/build" | cut -c1-160)"
        fi
      fi ;;
  esac
  printf '%-66s %-7s %-11s %s\n' "$name" "$expect" "$verdict" "${codes:-clean}"
done
rm -rf "${tmp:?}"
echo "LEAK: $leaks  FAIL: $fails  BROKEN: $broken  NATIVE: $native"
