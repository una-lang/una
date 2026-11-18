#!/bin/sh
# Runs `una --json check` on every adversarial program of the given directories, each in a fresh
# temporary copy (so the proof cache is cold and nothing is written into the repository), and
# prints one line per program: "<dir>/<program>\t<sorted unique codes or ->\t<exit>".
#
# Usage: rerun-adversarial.sh UNA_BINARY DIR...
# Z3 at the version of verification.lock must be on PATH.
set -eu
una=$1
shift
command -v jq >/dev/null || { echo "error: jq is required" >&2; exit 2; }
for dir in "$@"; do
    for prog in "$dir"/*; do
        name=$(basename "$prog")
        case "$name" in README.md) continue ;; esac
        tmp=$(mktemp -d "${TMPDIR:-/tmp}/una-adv.XXXXXX")
        cp -R "$prog" "$tmp/"
        set +e
        out=$(cd "$tmp" && "$una" --json check "$name" 2>/dev/null)
        code=$?
        set -e
        codes=$(printf '%s' "$out" | jq -r '[.diagnostics[]?.code] | unique | join(",")' 2>/dev/null || echo '?')
        [ -n "$codes" ] || codes=-
        printf '%s/%s\t%s\t%s\n' "$(basename "$(dirname "$dir")")" "$name" "$codes" "$code"
        rm -rf "${tmp:?}"
    done
done
