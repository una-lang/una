#!/bin/sh
# Checks the glibc floor of Linux executables (#2105): the highest `GLIBC_` symbol version each
# FILE asks the dynamic loader for (`objdump -T`) must be at or below FLOOR, compared as
# numbers (2.4 is before 2.35). The loader of an older glibc refuses to start an executable that
# names a later version, so this is what decides the oldest system a shipped `una` runs on;
# install.sh refuses a system below the same floor (its `glibc_floor`), and
# scripts/check-release.sh runs this on the packed `una` and on an executable it builds.
#
# Every versioned reference counts, weak ones included: a weak reference to a later version
# loads on an older glibc, but nothing here relies on that. A reference to `GLIBC_PRIVATE`, or
# to a version that is not numbers, fails too. A file in which objdump finds no `GLIBC_`
# version fails: it is not a dynamic glibc executable, or the listing was not read, and the
# check would prove nothing. What is linked statically into FILE (the Z3 library, the C++
# runtime, the runtime of `una build`) has no symbol versions of its own: its needs are FILE's.
#
# On success it prints, for each FILE, a line
# `glibc: <file>: highest symbol version GLIBC_<version>, floor <floor>`. A file above the floor
# exits 1, with each symbol above it on stderr; a usage error, a floor that is not MAJOR.MINOR,
# or an objdump that is missing or fails exits 2.
#
# Usage: scripts/glibc-floor.sh FLOOR FILE...
set -eu

if [ "$#" -lt 2 ]; then
    echo "usage: scripts/glibc-floor.sh FLOOR FILE..." >&2
    exit 2
fi
floor=$1
shift
case $floor in
    *[!0-9.]* | *.*.* | .* | *. | "") bad=1 ;;
    *.*) bad= ;;
    *) bad=1 ;;
esac
if [ -n "$bad" ]; then
    echo "error: the floor '$floor' is not MAJOR.MINOR, such as 2.35" >&2
    exit 2
fi
command -v objdump >/dev/null 2>&1 \
    || { echo "error: objdump is not installed; the glibc floor cannot be read" >&2; exit 2; }

status=0
for file in "$@"; do
    [ -f "$file" ] || { echo "error: $file is not a file" >&2; exit 2; }
    listing=$(LC_ALL=C objdump -T "$file" 2>/dev/null) \
        || { echo "error: objdump -T cannot read $file" >&2; exit 2; }
    code=0
    printf '%s\n' "$listing" | LC_ALL=C awk -v floor="$floor" -v file="$file" '
        BEGIN { split(floor, f, "."); highest = "" }
        {
            version = ""
            for (i = 1; i <= NF; i++) {
                field = $i
                gsub(/[()]/, "", field)
                if (field ~ /^GLIBC_/) version = substr(field, 7)
            }
            if (version == "") next
            seen++
            if (version !~ /^[0-9]+\.[0-9]+(\.[0-9]+)?$/) {
                above[++n] = $NF " (GLIBC_" version ")"
                next
            }
            split(version, v, ".")
            if (v[1] + 0 > f[1] + 0 || (v[1] + 0 == f[1] + 0 && (v[2] + 0 > f[2] + 0 || (v[2] + 0 == f[2] + 0 && v[3] + 0 > 0))))
                above[++n] = $NF " (GLIBC_" version ")"
            if (highest == "") { highest = version; split(version, h, "."); next }
            if (v[1] + 0 > h[1] + 0 || (v[1] + 0 == h[1] + 0 && (v[2] + 0 > h[2] + 0 || (v[2] + 0 == h[2] + 0 && v[3] + 0 > h[3] + 0)))) {
                highest = version
                split(version, h, ".")
            }
        }
        END {
            if (!seen) {
                print "error: " file " names no GLIBC_ symbol version: it is not a dynamic glibc executable, so its glibc floor cannot be read" > "/dev/stderr"
                exit 1
            }
            if (n) {
                print "error: " file " needs a glibc above the floor " floor ":" > "/dev/stderr"
                for (i = 1; i <= n; i++) print "  " above[i] > "/dev/stderr"
                exit 1
            }
            print "glibc: " file ": highest symbol version GLIBC_" highest ", floor " floor
        }' || code=$?
    [ "$code" = 0 ] || status=1
done
exit "$status"
