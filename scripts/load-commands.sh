#!/bin/sh
# What `una` and the executables it links ask the dynamic loader for (#1103): nothing but the
# entries of scripts/load-commands.txt. On macOS it reads every load command with `otool -l`:
# each kind must be listed, each dylib by its exact install name, and the dynamic linker. On
# Linux it reads the dynamic section with `readelf -d` and the program headers with
# `readelf -l`: each NEEDED entry and the program interpreter must be listed, and an RPATH,
# RUNPATH, AUXILIARY, FILTER, AUDIT or DEPAUDIT entry fails. A linker or a crate's `#[link]`
# that adds a library, as the libc crate's `iconv` did, fails here.
#
# UNA alone may also load the entries of scripts/load-commands-una.txt (#1277): the C++ runtime
# of the Z3 solver built into it (ADR 0124). The executables are held to scripts/load-commands.txt
# alone, so nothing of that list reaches a program that `una build` links.
#
# It reads UNA, then every file of the directory EXECUTABLES, executables that UNA linked. When
# EXECUTABLES is not given, it links examples/hello with UNA into a fresh temporary directory
# (a copy of the example, HOME there, without UNA_RT_LIB, so UNA links the runtime built into
# it, a release build's (ADR 0124 step 2), or the `libuna_rt.a` beside it, a development
# build's) and reads that executable.
#
# It fails closed: a missing UNA, list or tool, an empty executable directory, a build that
# fails, a tool that exits non-zero, or a listing with no load command, no dylib, no NEEDED
# entry or no interpreter is a failure, never a skip.
#
# Usage: scripts/load-commands.sh UNA [EXECUTABLES]
set -u

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
allow=$root/scripts/load-commands.txt
allow_una=$root/scripts/load-commands-una.txt

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
    echo "usage: scripts/load-commands.sh UNA [EXECUTABLES]" >&2
    exit 2
fi
una=$1
case $una in /*) ;; *) una=$(pwd)/$una ;; esac

fail() {
    echo "load-commands: FAIL ($1)" >&2
    exit 1
}

[ -f "$allow" ] && [ -r "$allow" ] || fail "no allowlist at $allow"
[ -f "$allow_una" ] && [ -r "$allow_una" ] || fail "no allowlist of una at $allow_una"
[ -f "$una" ] && [ -x "$una" ] || fail "no una binary at $una"

case $(uname -s) in
Darwin)
    os=macos
    command -v otool > /dev/null 2>&1 || fail "no otool: install the Xcode command line tools"
    ;;
Linux)
    os=linux
    command -v readelf > /dev/null 2>&1 || fail "no readelf: install binutils"
    ;;
*) fail "no rule for the platform $(uname -s)" ;;
esac

tmp=$(mktemp -d "${TMPDIR:-/tmp}/load-commands.XXXXXX") || fail "cannot make a temporary directory"
trap 'rm -rf "${tmp:?}"' EXIT

if [ "$#" -eq 2 ]; then
    executables=$2
    [ -d "$executables" ] || fail "no executable directory at $executables"
else
    executables=$tmp/executables
    mkdir -p "$executables" "$tmp/home" "$tmp/work" || fail "cannot make $tmp"
    cp -R "$root/examples/hello" "$tmp/work/" || fail "cannot copy examples/hello"
    if ! (cd "$tmp/work/hello" && env -u UNA_RT_LIB HOME="$tmp/home" NO_COLOR=1 \
        "$una" build . --out "$executables/examples__hello") > "$tmp/build.log" 2>&1; then
        cat "$tmp/build.log" >&2
        fail "$una did not build examples/hello"
    fi
fi
set --
for exe in "$executables"/* "$executables"/.[!.]*; do
    [ -f "$exe" ] && set -- "$@" "$exe"
done
[ "$#" -gt 0 ] || fail "no executable in $executables"

# Prints the entries of the artifact $1 that the allowlist must hold, one `KIND VALUE` per
# line; a line `forbidden KIND VALUE` never matches.
entries() {
    if [ "$os" = macos ]; then
        otool -l "$1" > "$tmp/listing" 2> "$tmp/err" || {
            cat "$tmp/err" >&2
            echo "load-commands: otool -l $1 failed" >&2
            return 1
        }
        awk '
            $1 == "cmd" { cmd = $2; commands++; print "command " cmd; next }
            $1 == "name" && (cmd == "LC_LOAD_DYLINKER" || cmd ~ /_DYLIB$/) {
                name = $0
                sub(/^[ \t]*name /, "", name)
                sub(/ \(offset [0-9]+\)$/, "", name)
                if (cmd == "LC_LOAD_DYLINKER") { print "dylinker " name; linkers++ }
                else if (cmd == "LC_LOAD_DYLIB") { print "dylib " name; named++ }
                else print "forbidden " cmd " " name
            }
            END {
                if (commands == 0) { print "missing a load command"; exit 1 }
                if (named == 0) { print "missing an LC_LOAD_DYLIB" ; exit 1 }
                if (linkers == 0) { print "missing an LC_LOAD_DYLINKER"; exit 1 }
            }' "$tmp/listing" > "$tmp/entries"
        status=$?
        # Every LC_LOAD_DYLIB names a dylib.
        dylibs=$(grep -c '^command LC_LOAD_DYLIB$' "$tmp/entries")
        names=$(grep -c '^dylib ' "$tmp/entries")
        [ "$dylibs" = "$names" ] || echo "missing the name of an LC_LOAD_DYLIB" >> "$tmp/entries"
    else
        readelf -dW "$1" > "$tmp/dynamic" 2> "$tmp/err" && readelf -lW "$1" > "$tmp/program" 2>> "$tmp/err" || {
            cat "$tmp/err" >&2
            echo "load-commands: readelf -d or -l $1 failed" >&2
            return 1
        }
        if grep -v '^readelf: Warning: ' "$tmp/err" | grep -q .; then
            cat "$tmp/err" >&2
            echo "load-commands: readelf reported an error on $1" >&2
            return 1
        fi
        {
            awk '
                /\(NEEDED\)/ {
                    value = $0
                    sub(/^.*\[/, "", value)
                    sub(/\][ \t]*$/, "", value)
                    print "needed " value
                    needed++
                }
                /\((RPATH|RUNPATH|AUXILIARY|FILTER|AUDIT|DEPAUDIT)\)/ { print "forbidden " $0 }
                END { if (needed == 0) print "missing a NEEDED entry" }' "$tmp/dynamic"
            awk '
                /Requesting program interpreter:/ {
                    value = $0
                    sub(/^.*interpreter: /, "", value)
                    sub(/\][ \t]*$/, "", value)
                    print "interpreter " value
                    found++
                }
                END { if (found == 0) print "missing a program interpreter" }' "$tmp/program"
        } > "$tmp/entries"
        status=0
    fi
    cat "$tmp/entries"
    return $status
}

# Checks the artifact $1: prints each entry the allowlists do not hold, and fails on one. The
# allowlist is scripts/load-commands.txt, with $2 (scripts/load-commands-una.txt, for `una`
# alone) added when it is given.
check() {
    entries "$1" > "$tmp/found" || {
        cat "$tmp/found" >&2
        return 1
    }
    awk -v os="$os" -v artifact="$1" -v allowfile="$allow" -v extrafile="${2:-}" '
        function read(file) {
            while ((getline line < file) > 0) {
                sub(/#.*/, "", line)
                n = split(line, field, /[ \t]+/)
                start = (field[1] == "") ? 2 : 1
                if (n < start + 2) continue
                if (field[start] == os) allowed[field[start + 1] " " field[start + 2]] = 1
            }
            close(file)
        }
        BEGIN {
            read(allowfile)
            if (extrafile != "") read(extrafile)
        }
        {
            if (!($0 in allowed)) {
                print "load-commands: " artifact ": not allowed: " $0
                bad++
            }
        }
        END { exit bad ? 1 : 0 }' "$tmp/found"
}

failures=0
count=0
for artifact in "$una" "$@"; do
    count=$((count + 1))
    # The first artifact is UNA, the only one checked against scripts/load-commands-una.txt too.
    extra=""
    [ "$count" -eq 1 ] && extra=$allow_una
    if check "$artifact" "$extra"; then
        echo "load-commands: ok $artifact: $(grep -E '^(dylib|needed|interpreter|dylinker) ' "$tmp/found" | cut -d' ' -f2- | tr '\n' ' ')"
    else
        failures=$((failures + 1))
    fi
done
[ "$failures" -eq 0 ] || fail "$failures of $count artifacts load what scripts/load-commands.txt (and for una scripts/load-commands-una.txt) does not allow"
echo "load-commands: PASS ($count artifacts, $os)"
