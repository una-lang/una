#!/bin/sh
# No part of Una may reach the platform's C math library (#1092, docs/spec/29-numerics.md §6
# rule 5). Clippy's disallowed methods see method calls in the project's own source only, not an
# operator that lowers to a call (float `%` is `fmod`) nor the code of a dependency, so this
# check reads the built artifacts: it lists their undefined symbols with `nm -u` and fails on
# every name of scripts/no-libm.txt, after it drops the platform's prefix (the leading `_` of
# Mach-O) and a symbol version (`@GLIBC_2.2.5`).
#
# - UNA, the `una` binary: strict.
# - LIBUNA_RT, `libuna_rt.a`: listed with `nm -A -u`, so each hit names its object; a hit passes
#   only when an entry of scripts/no-libm-allow.txt names its object and symbol, and an entry
#   that no hit uses fails, so the file stays exact.
# - EXECUTABLES, a directory of linked executables, such as the native differential keeps under
#   UNA_NATIVE_KEEP (crates/una-cli/tests/native.rs): every file there, strict. On Linux both
#   the static and the dynamic symbol tables are read (`nm -u` and `nm -D -u`).
#
# The tool is, on macOS, the `llvm-nm` of the pinned Rust toolchain (the `llvm-tools` component),
# whose LLVM reads the bitcode that Rust's objects carry, else `llvm-nm`, else `nm`; on Linux,
# `nm` (binutils), else `llvm-nm`, else the toolchain's. It fails closed: a missing artifact or
# list, an empty executable directory, no tool, or a tool that exits non-zero or reports
# anything but an object without symbols is a failure, never a skip.
#
# Usage: scripts/no-libm.sh UNA LIBUNA_RT EXECUTABLES
set -u

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
deny=$root/scripts/no-libm.txt
allow=$root/scripts/no-libm-allow.txt

if [ "$#" -ne 3 ]; then
    echo "usage: scripts/no-libm.sh UNA LIBUNA_RT EXECUTABLES" >&2
    exit 2
fi
una=$1
archive=$2
executables=$3

fail() {
    echo "no-libm: FAIL ($1)" >&2
    exit 1
}

[ -f "$deny" ] && [ -r "$deny" ] || fail "no denylist at $deny"
[ -f "$allow" ] && [ -r "$allow" ] || fail "no allow file at $allow"
[ -f "$una" ] && [ -r "$una" ] || fail "no una binary at $una"
[ -f "$archive" ] && [ -r "$archive" ] || fail "no runtime library at $archive"
[ -d "$executables" ] || fail "no executable directory at $executables"
present=0
for exe in "$executables"/* "$executables"/.[!.]*; do
    [ -e "$exe" ] && present=1
done
[ "$present" -eq 1 ] || fail "no executable in $executables"

case $(uname -s) in
Darwin) os=macos ;;
Linux) os=linux ;;
*) fail "no rule for the platform $(uname -s)" ;;
esac

# The pinned toolchain's own llvm-nm, when its llvm-tools component is installed.
toolchain_nm() {
    sysroot=$(cd "$root" && rustc --print sysroot 2>/dev/null) || return 1
    host=$(cd "$root" && rustc -vV 2>/dev/null | sed -n 's/^host: //p')
    [ -n "$sysroot" ] && [ -n "$host" ] || return 1
    [ -x "$sysroot/lib/rustlib/$host/bin/llvm-nm" ] || return 1
    printf '%s\n' "$sysroot/lib/rustlib/$host/bin/llvm-nm"
}
nm_tool=""
if [ "$os" = macos ]; then
    nm_tool=$(toolchain_nm) || nm_tool=""
    [ -n "$nm_tool" ] || nm_tool=$(command -v llvm-nm) || nm_tool=""
    [ -n "$nm_tool" ] || nm_tool=$(command -v nm) || nm_tool=""
else
    nm_tool=$(command -v nm) || nm_tool=""
    [ -n "$nm_tool" ] || nm_tool=$(command -v llvm-nm) || nm_tool=""
    [ -n "$nm_tool" ] || nm_tool=$(toolchain_nm) || nm_tool=""
fi
[ -n "$nm_tool" ] || fail "no nm: install binutils or llvm-nm, or the toolchain's component with \`rustup component add llvm-tools\`"
echo "no-libm: listing undefined symbols with $nm_tool"

tmp=$(mktemp -d "${TMPDIR:-/tmp}/no-libm.XXXXXX") || fail "cannot make a temporary directory"
trap 'rm -rf "${tmp:?}"' EXIT

# Runs nm with the arguments that follow, appending its output to $tmp/out. Any exit status but
# 0, and any diagnostic but an object without symbols, is an nm error.
list() {
    "$nm_tool" "$@" >> "$tmp/out" 2> "$tmp/err"
    status=$?
    if [ "$status" -ne 0 ] || grep -v ': no symbols$' "$tmp/err" | grep -q .; then
        cat "$tmp/err" >&2
        echo "no-libm: $nm_tool $* failed (exit status $status)" >&2
        return 1
    fi
}

# Checks the listing in $tmp/out of the artifact $1: strict, or, with $2 = archive, against the
# allow file. Prints each finding and each allowed hit; exits non-zero on a finding.
check() {
    awk -v artifact="$1" -v mode="${2:-strict}" -v os="$os" -v denyfile="$deny" -v allowfile="$allow" '
        function trim(s) { sub(/#.*/, "", s); gsub(/^[ \t]+|[ \t]+$/, "", s); return s }
        function name(s) {
            sub(/@.*/, "", s)
            if (os == "macos") sub(/^_/, "", s)
            return s
        }
        function glob(p,    r, i, c) {
            r = "^"
            for (i = 1; i <= length(p); i++) {
                c = substr(p, i, 1)
                if (c == "*") r = r ".*"
                else if (c == "?") r = r "."
                else if (c ~ /[A-Za-z0-9_-]/) r = r c
                else r = r "[" c "]"
            }
            return r "$"
        }
        BEGIN {
            while ((getline line < denyfile) > 0) {
                line = trim(line)
                if (line != "") denied[line] = 1
            }
            close(denyfile)
            entries = 0
            if (mode == "archive") {
                while ((getline line < allowfile) > 0) {
                    if (trim(line) == "") continue
                    sub(/^[ \t]+/, "", line)
                    n = split(line, f, /[ \t]+/)
                    if (n < 3) { print "no-libm: " allowfile ": an entry needs an object, a symbol and a reason: " line; bad = 1; continue }
                    if (!(f[2] in denied)) { print "no-libm: " allowfile ": " f[2] " is not a name of the denylist"; bad = 1; continue }
                    entries++
                    pattern[entries] = glob(f[1]); symbol[entries] = f[2]
                    reason[entries] = line; sub(/^[^ \t]+[ \t]+[^ \t]+[ \t]+/, "", reason[entries])
                    shown[entries] = f[1] " " f[2]
                }
                close(allowfile)
            }
            prefix = artifact ":"
        }
        /^[ \t]*$/ { next }
        {
            where = artifact
            rest = $0
            if (mode == "archive") {
                if (index(rest, prefix) != 1) { print "no-libm: " artifact ": a line nm wrote that names no object: " $0; bad = 1; next }
                rest = substr(rest, length(prefix) + 1)
                colon = index(rest, ":")
                if (colon == 0) { print "no-libm: " artifact ": a line nm wrote that names no object: " $0; bad = 1; next }
                object = substr(rest, 1, colon - 1)
                rest = substr(rest, colon + 1)
                where = artifact "(" object ")"
            }
            k = split(rest, w, /[ \t]+/)
            s = name(w[k])
            if (!(s in denied)) next
            if (mode == "archive") {
                for (e = 1; e <= entries; e++) {
                    if (symbol[e] == s && object ~ pattern[e]) {
                        used[e] = 1
                        print "no-libm: allowed " where ": " s " (" reason[e] ")"
                        next
                    }
                }
            }
            print "no-libm: " where ": " s
            bad = 1
        }
        END {
            for (e = 1; e <= entries; e++)
                if (!(e in used)) { print "no-libm: " allowfile ": the entry \"" shown[e] "\" matches no symbol of " artifact; bad = 1 }
            exit bad ? 1 : 0
        }
    ' "$tmp/out"
}

findings=0

: > "$tmp/out"
if list -u "$una"; then
    if [ "$os" = linux ]; then list -D -u "$una" || findings=1; fi
    check "$una" || findings=1
else
    findings=1
fi

: > "$tmp/out"
if list -A -u "$archive"; then
    check "$archive" archive || findings=1
else
    findings=1
fi

count=0
for exe in "$executables"/* "$executables"/.[!.]*; do
    [ -e "$exe" ] || continue
    if [ ! -f "$exe" ]; then
        echo "no-libm: $exe is not a file"
        findings=1
        continue
    fi
    count=$((count + 1))
    : > "$tmp/out"
    if list -u "$exe"; then
        if [ "$os" = linux ]; then list -D -u "$exe" || findings=1; fi
        check "$exe" || findings=1
    else
        findings=1
    fi
done

if [ "$findings" -ne 0 ]; then
    fail "a C math library symbol or an nm error above, in $una, $archive or $count executables of $executables"
fi
echo "no-libm: PASS ($una, $archive and $count executables of $executables)"
