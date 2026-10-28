#!/bin/sh
# No part of Una may reach the platform's C math library (#1092, docs/spec/29-numerics.md §6
# rule 5). Clippy's disallowed methods see method calls in the project's own source only, not an
# operator that lowers to a call (float `%` is `fmod`) nor the code of a dependency, so this
# check reads the release artifacts: it lists their undefined symbols with `nm -u` and fails on
# every name of scripts/no-libm.txt, after it drops the platform's prefix (the leading `_` of
# Mach-O) and a symbol version (`@GLIBC_2.2.5`). Every artifact is strict, with no exception:
#
# - UNA, the release `una` binary;
# - LIBUNA_RT, the release `libuna_rt.a`, listed with `nm -A -u`, so each hit names its object;
# - EXECUTABLES, a directory of executables that release `una` linked
#   (scripts/no-libm-build.sh): every file there. On Linux both the static and the dynamic
#   symbol tables of `una` and of the executables are read (`nm -u` and `nm -D -u`).
#
# First it asserts the release dependency graph (`cargo tree --locked -e normal,features -p
# una-cli -p una-rt`): none of num-bigint, num-integer and num-traits has its `std` or `default`
# feature, since num-bigint's `std` code calls the C math library (`log2` while parsing, `ln`,
# `exp` and `cbrt` in its roots), and its other code is integer-only. The graph holds normal
# dependencies only: a test build unifies dev-dependencies' features in and is not what ships.
#
# The tool is, on macOS, the `llvm-nm` of the pinned Rust toolchain (the `llvm-tools` component),
# whose LLVM reads the bitcode that Rust's objects carry, else `llvm-nm`, else `nm`; on Linux,
# `nm` (binutils), else `llvm-nm`, else the toolchain's. It fails closed: a `cargo tree` that
# fails or prints nothing, a missing artifact or list, an empty executable directory, no tool,
# or a tool that exits non-zero or reports anything but an object without symbols is a failure,
# never a skip.
#
# Usage: scripts/no-libm.sh UNA LIBUNA_RT EXECUTABLES
set -u

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
deny=$root/scripts/no-libm.txt

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

# Checks the listing in $tmp/out of the artifact $1, strictly; with $2 = archive, each line
# names its object (`nm -A`). Prints each finding; exits non-zero on one.
check() {
    awk -v artifact="$1" -v mode="${2:-binary}" -v os="$os" -v denyfile="$deny" '
        function trim(s) { sub(/#.*/, "", s); gsub(/^[ \t]+|[ \t]+$/, "", s); return s }
        function name(s) {
            sub(/@.*/, "", s)
            if (os == "macos") sub(/^_/, "", s)
            return s
        }
        BEGIN {
            while ((getline line < denyfile) > 0) {
                line = trim(line)
                if (line != "") denied[line] = 1
            }
            close(denyfile)
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
                where = artifact "(" substr(rest, 1, colon - 1) ")"
                rest = substr(rest, colon + 1)
            }
            k = split(rest, w, /[ \t]+/)
            s = name(w[k])
            if (!(s in denied)) next
            print "no-libm: " where ": " s
            bad = 1
        }
        END { exit bad ? 1 : 0 }
    ' "$tmp/out"
}

findings=0

# The release dependency graph: no `std` (nor `default`, which means `std`) feature of the
# crates whose `std` code calls the C math library. `cargo tree -i` of a crate the graph does
# not hold fails too, so a renamed or replaced dependency is a failure to look at, not a pass.
for crate in num-bigint num-integer num-traits; do
    if ! (cd "$root" && cargo tree --locked -e normal,features -p una-cli -p una-rt -i "$crate") \
        > "$tmp/tree" 2> "$tmp/err"; then
        cat "$tmp/err" >&2
        echo "no-libm: cargo tree of $crate failed"
        findings=1
    elif ! grep -q "^$crate v" "$tmp/tree"; then
        cat "$tmp/tree" "$tmp/err" >&2
        echo "no-libm: cargo tree printed no $crate"
        findings=1
    elif grep -E "$crate feature \"(std|default)\"" "$tmp/tree"; then
        echo "no-libm: the release graph enables the std feature of $crate"
        findings=1
    else
        features=$(grep -E -o "$crate feature \"[^\"]*\"" "$tmp/tree" | sed 's/.* feature //' \
            | LC_ALL=C sort -u | tr '\n' ' ' | sed 's/ $//')
        echo "no-libm: $crate in the release graph, features: ${features:-none}"
    fi
done

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
    fail "a std feature, a C math library symbol, or a cargo tree or nm error above, in $una, $archive or $count executables of $executables"
fi
echo "no-libm: PASS ($una, $archive and $count executables of $executables)"
