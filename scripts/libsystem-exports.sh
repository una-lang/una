#!/bin/sh
# The export list of libSystem that the internal linker binds against (ADR 0124 rule 23,
# docs/plan/0124-step3-design.md §3, decision D3): every symbol that `/usr/lib/libSystem.B.dylib`
# and the libraries it re-exports export for the target `arm64-macos`, read from the SDK's text
# stub `usr/lib/libSystem.B.tbd` (a tbd-version 4 document per library). It writes
# supply-chain/libsystem/exports-arm64.txt: a header of `#` lines naming the SDK and libSystem's
# current version, then one symbol per line, sorted, without duplicates. The digests of the
# stub and of the list go into supply-chain/libsystem.lock.
#
# The parser reads only what it needs: the top-level keys, the `targets:` of each entry under
# `exports:` and `reexports:`, and that entry's `symbols:`, `weak-symbols:` and
# `thread-local-symbols:` lists. It fails on a key it does not know inside those entries, on a
# list that never closes, and on an empty result, so a change of the format is a loud failure,
# never a shorter list. The list is regenerated only when the pinned SDK changes.
#
# Usage: scripts/libsystem-exports.sh SDK_PATH
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: scripts/libsystem-exports.sh SDK_PATH" >&2
    exit 2
fi
sdk=$1
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
tbd=$sdk/usr/lib/libSystem.B.tbd
settings=$sdk/SDKSettings.json
[ -f "$tbd" ] || { echo "libsystem-exports: no $tbd" >&2; exit 1; }
[ -f "$settings" ] || { echo "libsystem-exports: no $settings" >&2; exit 1; }
version=$(sed -n 's/.*"Version"[[:space:]]*:[[:space:]]*"\([0-9.]*\)".*/\1/p' "$settings" | head -n 1)
[ -n "$version" ] || { echo "libsystem-exports: no Version in $settings" >&2; exit 1; }
current=$(awk '/^install-name:/ { name = $2 } /^current-version:/ && name == "'"'"'/usr/lib/libSystem.B.dylib'"'"'" { print $2; exit }' "$tbd")
[ -n "$current" ] || { echo "libsystem-exports: no current-version of libSystem in $tbd" >&2; exit 1; }

out_dir=$root/supply-chain/libsystem
out=$out_dir/exports-arm64.txt
tmp=$(mktemp "${TMPDIR:-/tmp}/libsystem-exports.XXXXXX")
trap 'rm -f "${tmp:?}"' EXIT

awk '
function fail(message) { print "libsystem-exports: " message " at line " NR > "/dev/stderr"; failed = 1; exit 1 }
function take(text,    n, i, parts, token) {
    gsub(/[\[\],]/, " ", text)
    n = split(text, parts, " ")
    for (i = 1; i <= n; i++) {
        token = parts[i]
        if (token == "") continue
        gsub(/^'"'"'|'"'"'$/, "", token)
        if (wanted && token !~ /^\$ld\$/) print token
    }
}
/^--- / { section = ""; next }
/^\.\.\./ { section = ""; next }
/^[a-z-]+:/ {
    key = $1; sub(/:$/, "", key)
    section = (key == "exports" || key == "reexports") ? key : ""
    if (inlist) fail("a list that does not close")
    next
}
section == "" { next }
{
    line = $0
    if (inlist) {
        if (listkind == "symbols") take(line); else if (listkind == "targets") targets = targets " " line
        if (line ~ /\]/) { inlist = 0; if (listkind == "targets") wanted = (targets ~ /(^|[ \[,])arm64-macos([ ,\]]|$)/) }
        next
    }
    if (line ~ /^  - targets:/) {
        targets = line; listkind = "targets"; wanted = 0
        if (line ~ /\]/) wanted = (targets ~ /(^|[ \[,])arm64-macos([ ,\]]|$)/); else inlist = 1
        next
    }
    if (line ~ /^    (symbols|weak-symbols|thread-local-symbols):/) {
        sub(/^    [a-z-]+:/, "", line); listkind = "symbols"; take(line)
        if (line !~ /\]/) inlist = 1
        next
    }
    if (line ~ /^    (objc-classes|objc-eh-types|objc-ivars):/) {
        listkind = "skip"; if (line !~ /\]/) inlist = 1
        next
    }
    fail("an unknown key in " section ": " line)
}
END { if (inlist && !failed) fail("a list that does not close at the end") }
' "$tbd" | LC_ALL=C sort -u > "$tmp"

count=$(wc -l < "$tmp" | tr -d ' ')
[ "$count" -gt 1000 ] || { echo "libsystem-exports: only $count symbols; refusing" >&2; exit 1; }
grep -qx '_write' "$tmp" || { echo "libsystem-exports: _write is missing; refusing" >&2; exit 1; }

mkdir -p "$out_dir"
{
    echo "# The symbols that /usr/lib/libSystem.B.dylib and the libraries it re-exports export for"
    echo "# arm64-macos, from the SDK's usr/lib/libSystem.B.tbd, by scripts/libsystem-exports.sh."
    echo "# sdk $version"
    echo "# libsystem-current-version $current"
    cat "$tmp"
} > "$out"
tbd_sha=$(shasum -a 256 "$tbd" | awk '{print $1}')
list_sha=$(shasum -a 256 "$out" | awk '{print $1}')
{
    echo "# The SDK whose libSystem export list the internal linker binds against (ADR 0124 rule 23,"
    echo "# docs/plan/0124-step3-design.md decision D3), made by scripts/libsystem-exports.sh. The list"
    echo "# changes only with this pin; crates/una-link checks the list's digest at its build."
    echo "sdk = \"macosx$version\""
    echo "libsystem_current_version = \"$current\""
    echo "libsystem_tbd_sha256 = \"$tbd_sha\""
    echo "exports_sha256 = \"$list_sha\""
    echo "symbols = $count"
} > "$root/supply-chain/libsystem.lock"
echo "libsystem-exports: $count symbols from the SDK $version into $out"
