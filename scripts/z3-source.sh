#!/bin/sh
# Prints the absolute path of the local copy of the pinned Z3 tarball that UNA_Z3_SOURCE names
# (#1340, #1902), for scripts/local-ci.sh and scripts/hunt-linux.sh, which mount it read-only
# into their containers so that a cold cache volume builds the library without the lock's
# mirror. Prints nothing when UNA_Z3_SOURCE is unset or empty. Fails, with a message on stderr,
# when it names anything but a readable regular file, or a path with a colon, which a Docker
# --volume option cannot hold: a wrong setting is an error, never a run without the copy. The
# file's contents are not checked here: scripts/build-z3.sh checks them against
# supply-chain/z3.lock before any build.
#
# Usage: scripts/z3-source.sh
set -eu

[ "$#" -eq 0 ] || { echo "usage: scripts/z3-source.sh" >&2; exit 2; }
source=${UNA_Z3_SOURCE:-}
[ -n "$source" ] || exit 0
if [ ! -f "$source" ] || [ ! -r "$source" ]; then
    echo "error: UNA_Z3_SOURCE names no readable file: $source" >&2
    exit 1
fi
dir=$(CDPATH='' cd -- "$(dirname -- "$source")" && pwd) \
    || { echo "error: cannot resolve the directory of UNA_Z3_SOURCE: $source" >&2; exit 1; }
path="$dir/$(basename -- "$source")"
case $path in
    *:*)
        echo "error: UNA_Z3_SOURCE resolves to a path with a colon, which a Docker --volume cannot mount: $path" >&2
        exit 1
        ;;
esac
printf '%s\n' "$path"
