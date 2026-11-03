#!/bin/sh
# Hands the named volumes of the Linux containers (scripts/local-ci.sh, scripts/hunt-linux.sh,
# scripts/local-release.sh) to the image's unprivileged user, uid and gid 1000 (#1394), before the
# unprivileged run that mounts them. The containers run as that user because the solver child
# refuses to run as root (ADR 0124 rule 7), and a volume made by a root container before (or by
# an older checkout's root image since) holds entries that user cannot write. This one step runs
# as root, with no network, and changes only the owner of the entries that another owner holds;
# on a volume that is already the user's it changes nothing. It never changes permissions, and
# the unprivileged run that follows is unchanged.
#
# Usage: scripts/container-volumes.sh PLATFORM IMAGE VOLUME:MOUNTPOINT...
set -eu

[ "$#" -ge 3 ] || { echo "usage: scripts/container-volumes.sh PLATFORM IMAGE VOLUME:MOUNTPOINT..." >&2; exit 2; }
platform=$1
image=$2
shift 2
# The mount points, for the step's own arguments, and one --volume option per pair, appended
# after the pairs, which are then dropped.
count=$#
mounts=""
for pair in "$@"; do
    case $pair in
    ?*:/?*) ;;
    *) echo "error: $pair is not VOLUME:MOUNTPOINT" >&2; exit 2 ;;
    esac
    set -- "$@" --volume "$pair"
    mounts="$mounts ${pair#*:}"
done
shift "$count"
# shellcheck disable=SC2086 # the mount points are absolute paths without spaces
docker run --rm --platform "$platform" --network none --user 0:0 "$@" \
    --entrypoint /bin/sh "$image" -c '
        set -eu
        for dir; do
            changed=$(find "$dir" \( ! -user 1000 -o ! -group 1000 \) -print | wc -l)
            if [ "$changed" -ne 0 ]; then
                find "$dir" \( ! -user 1000 -o ! -group 1000 \) -exec chown -h 1000:1000 {} +
                echo "== $dir: $changed entries handed to uid 1000 (#1394)"
            fi
        done' sh $mounts
