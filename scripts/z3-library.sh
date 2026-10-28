#!/bin/sh
# Prints the path of the Z3 library built into una (ADR 0124 rules 1 and 19): the entry of the
# cache (UNA_Z3_CACHE, else ~/.cache/una/z3) for the pin of supply-chain/z3.lock and this
# machine's toolchain, checked against the digest its build recorded (library.sha256); an entry
# whose bytes changed is refused and built again, and a missing one is built, by
# scripts/build-z3.sh (a cold build of 10 to 20 minutes; UNA_Z3_SOURCE names a local copy of the
# pinned tarball until the mirror release exists). scripts/local-ci.sh and the release gates set
# UNA_Z3_LIB from it. Progress goes to stderr; stdout is the path alone.
#
# Usage: scripts/z3-library.sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cache=${UNA_Z3_CACHE:-$HOME/.cache/una/z3}
key=$("$root/scripts/build-z3.sh" --key) || { echo "error: the cache key of the Z3 library" >&2; exit 1; }
lib="$cache/$key/lib/libz3shell.a"
recorded="$cache/$key/library.sha256"
digest() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d ' ' -f 1
    else shasum -a 256 "$1" | cut -d ' ' -f 1; fi
}
if [ -f "$lib" ] && [ -f "$recorded" ]; then
    actual=$(digest "$lib")
    if [ "$actual" = "$(cat "$recorded")" ]; then
        echo "the Z3 library of supply-chain/z3.lock, from the cache $cache/$key ($actual)" >&2
        echo "$lib"
        exit 0
    fi
    echo "the cached Z3 library's digest differs from its record ($actual); refused, building again" >&2
fi
echo "building the Z3 library of supply-chain/z3.lock into $cache/$key" >&2
if [ -n "${UNA_Z3_SOURCE:-}" ]; then
    "$root/scripts/build-z3.sh" --source "$UNA_Z3_SOURCE" --out "$cache" >&2
else
    "$root/scripts/build-z3.sh" --out "$cache" >&2
fi
[ -f "$lib" ] || { echo "error: the Z3 build wrote no library at $lib" >&2; exit 1; }
echo "$lib"
