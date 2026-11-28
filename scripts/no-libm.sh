#!/bin/sh
# No part of Una may reach the platform's C math library (#1092, docs/spec/29-numerics.md §6
# rule 5). Clippy's disallowed methods see method calls in the project's own source only, not an
# operator that lowers to a call (float `%` is `fmod`) nor the code of a dependency, so this
# check reads the release artifacts: it lists their undefined symbols with `nm -u` and fails on
# every name of scripts/no-libm.txt, after it drops the platform's prefix (the leading `_` of
# Mach-O) and a symbol version (`@GLIBC_2.2.5`). The runtime and the executables, what a Una
# program contains, are strict, with no exception:
#
# - LIBUNA_RT, the release `libuna_rt.a`, listed with `nm -A -u`, so each hit names its object;
# - EXECUTABLES, a directory of executables that release `una` linked
#   (scripts/no-libm-build.sh): every file there.
#
# - UNA_WITHOUT_SOLVER, the same release `una` built without the Z3 solver
#   (scripts/no-libm-build.sh): Una's own Rust code, the interpreter's included.
#
# UNA, the release `una` binary, holds the Z3 solver (ADR 0124), C++ that calls the math library
# in its heuristics (#1277). It is checked against the same list, except for the names that
# scripts/no-libm-una.txt lists for this platform, each of which must also be on
# scripts/no-libm.txt and, when `una` imports it, be imported by the Z3 library that UNA_Z3_LIB
# names, the one `una` was linked with (`nm -A -u`), and only from objects of that library that
# the list names for it (#1953): a name `una` imports that the solver does not, that an object
# the list does not name imports, or with no UNA_Z3_LIB to tie it to, fails. On Linux both the
# static and the dynamic symbol tables of `una` and of the executables are read (`nm -u` and
# `nm -D -u`).
#
# With --link-trace TRACE (Linux only), UNA is the shipped `una`, which scripts/release.sh links
# with libstdc++.a and libgcc.a inside it, and TRACE is the record of that very link that
# scripts/link-trace.sh wrote (GNU ld's --trace-symbol for every name of scripts/no-libm.txt):
# its first line names the linked file, which must be byte for byte UNA, and its second the
# traced names, which must be all of scripts/no-libm.txt. Each object of the link that refers to
# one of those names must then be an archive member that is, byte for byte (`ar p`), the member
# of the same name of the Z3 library UNA_Z3_LIB names (the Z3 objects reach the link inside
# una-z3's rlib), and one that scripts/no-libm-una.txt names for that name on Linux; any other
# object (a member of libstdc++.a, of a Rust library, the binary's own code) is a finding. A
# definition of one of those names outside a shared library (a math library linked in
# statically) is a finding too, except one by the pinned toolchain's compiler_builtins of a name
# of the list's `linux-builtins` entries, which only Z3's listed objects may refer to, or of its
# `linux-rust-builtins` entries, which only the Rust objects they name exactly may refer to
# (#2354). The runtime's `linux-rust-builtins` references, and its `linux-rt-builtins` ones by
# the compiler_builtins member that defines the function the entry names, are allowed only where
# the same archive defines the name in a compiler_builtins member (#2354). Each
# solver name `una` imports must have such a reference. The trace names each object, so this mode needs no
# `una` built without the solver; it has no UNA_WITHOUT_SOLVER argument.
#
# First it asserts the release dependency graph (`cargo tree --locked -e normal -p una-cli -p
# una-rt`): it holds none of num-bigint, num-integer and num-traits, whose `std` code called the
# C math library (`log2` while parsing, `ln`, `exp` and `cbrt` in their roots); Una's own big
# integer in una-core replaced them (#1093). The graph holds normal dependencies only: a test
# build unifies dev-dependencies in and is not what ships. How a run of
# `cargo tree -i` is judged is scripts/no-libm-graph.sh (#1949): a tree it prints is a finding,
# and only cargo's own report of a crate it has nothing to print for, or of a package ID
# specification that matches no package, is not.
#
# The tool is, on macOS, the `llvm-nm` of the pinned Rust toolchain (the `llvm-tools` component),
# whose LLVM reads the bitcode that Rust's objects carry, else `llvm-nm`, else `nm`; on Linux,
# `nm` (binutils), else `llvm-nm`, else the toolchain's. It fails closed: a `cargo tree` that
# fails, or prints nothing without saying so, a missing artifact or list, an empty executable
# directory, no tool, or a tool that exits non-zero or reports anything but an object without
# symbols is a failure, never a skip.
#
# Usage: scripts/no-libm.sh UNA LIBUNA_RT EXECUTABLES UNA_WITHOUT_SOLVER
#        scripts/no-libm.sh --link-trace TRACE UNA LIBUNA_RT EXECUTABLES
set -u

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
deny=$root/scripts/no-libm.txt
solver=$root/scripts/no-libm-una.txt
graph=$root/scripts/no-libm-graph.sh

trace=""
rust_una=""
if [ "$#" -eq 5 ] && [ "$1" = --link-trace ]; then
    trace=$2
    shift 2
elif [ "$#" -ne 4 ]; then
    echo "usage: scripts/no-libm.sh UNA LIBUNA_RT EXECUTABLES UNA_WITHOUT_SOLVER" >&2
    echo "       scripts/no-libm.sh --link-trace TRACE UNA LIBUNA_RT EXECUTABLES" >&2
    exit 2
fi
una=$1
archive=$2
executables=$3
[ -n "$trace" ] || rust_una=$4

fail() {
    echo "no-libm: FAIL ($1)" >&2
    exit 1
}

[ -f "$deny" ] && [ -r "$deny" ] || fail "no denylist at $deny"
[ -f "$solver" ] && [ -r "$solver" ] || fail "no list of the solver's imports at $solver"
[ -f "$graph" ] && [ -r "$graph" ] || fail "no graph check at $graph"
# shellcheck source=scripts/no-libm-graph.sh
. "$graph"
[ -f "$una" ] && [ -r "$una" ] || fail "no una binary at $una"
if [ -n "$trace" ]; then
    [ -f "$trace" ] && [ -r "$trace" ] || fail "no link trace at $trace"
else
    [ -f "$rust_una" ] && [ -r "$rust_una" ] || fail "no una without the solver at $rust_una"
fi
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
[ -z "$trace" ] || [ "$os" = linux ] || fail "a link trace is GNU ld's, read on Linux only"

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
# names its object (`nm -A`); with $2 = una, a name that scripts/no-libm-una.txt lists for this
# platform ($tmp/names) is not a finding but is written to $tmp/solver, to be tied to the Z3
# library. Prints each finding; exits
# non-zero on one.
check() {
    : > "$tmp/solver"
    awk -v artifact="$1" -v mode="${2:-binary}" -v os="$os" -v denyfile="$deny" \
        -v solverfile="$tmp/names" -v solverout="$tmp/solver" -v rtfile="${3:-/dev/null}" '
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
            if (mode == "una") {
                while ((getline line < solverfile) > 0) {
                    line = trim(line)
                    if (line != "") solver[line] = 1
                }
                close(solverfile)
            }
            if (mode == "archive") {
                while ((getline line < rtfile) > 0) if (line != "") rtallowed[line] = 1
                close(rtfile)
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
                where = artifact "(" substr(rest, 1, colon - 1) ")"
                rest = substr(rest, colon + 1)
            }
            k = split(rest, w, /[ \t]+/)
            s = name(w[k])
            if (!(s in denied)) next
            if (mode == "una" && (s in solver)) { print s > solverout; next }
            if (mode == "archive" && ((s " " substr(where, length(artifact) + 2, length(where) - length(artifact) - 2)) in rtallowed)) next
            print "no-libm: " where ": " s
            bad = 1
        }
        END { exit bad ? 1 : 0 }
    ' "$tmp/out"
}

findings=0

# The solver's list, one entry per line: a platform (`macos`, `linux` or `linux-builtins`), a
# name, which must be one of the denylist's, so that a typo or a name that is not the math
# library's cannot hide anything, and at least one object of the Z3 library. $tmp/allowed gets
# `name object` for each object of this platform's entries, $tmp/names this platform's names,
# and on Linux $tmp/builtins `name object` for each object of the `linux-builtins` entries: the
# references of Z3 objects that the link resolves to the pinned toolchain's compiler_builtins,
# which only the trace mode reads; `una` may not import those names.
awk -v os="$os" -v arch="$(uname -m)" -v denyfile="$deny" -v listfile="$solver" -v allowed="$tmp/allowed" -v names="$tmp/names" \
    -v builtins="$tmp/builtins" -v rustbuiltins="$tmp/rustbuiltins" -v rtbuiltins="$tmp/rtbuiltins" '
    function trim(s) { sub(/#.*/, "", s); gsub(/^[ \t]+|[ \t]+$/, "", s); return s }
    BEGIN {
        while ((getline line < denyfile) > 0) {
            line = trim(line)
            if (line != "") denied[line] = 1
        }
        close(denyfile)
        printf "" > allowed
        printf "" > names
        printf "" > builtins
        printf "" > rustbuiltins
        printf "" > rtbuiltins
    }
    {
        line = trim($0)
        if (line == "") next
        n = split(line, w, /[ \t]+/)
        # A platform may be scoped to one processor, `PLATFORM@MACHINE` (`uname -m`), #2365.
        entry = w[1]
        machine = ""
        if (index(w[1], "@") > 0) { machine = substr(w[1], index(w[1], "@") + 1); w[1] = substr(w[1], 1, index(w[1], "@") - 1) }
        if (n < 3 || (w[1] != "macos" && w[1] != "linux" && w[1] != "linux-builtins" && w[1] != "linux-rust-builtins" && w[1] != "linux-rt-builtins") \
            || (index(entry, "@") > 0 && machine !~ /^(x86_64|aarch64|arm64)$/)) {
            print "no-libm: " listfile ": an entry that is not a platform, a name and its objects: " $0
            bad = 1
            next
        }
        if (!(w[2] in denied)) { print "no-libm: " listfile ": " w[2] " is not a name of " denyfile; bad = 1; next }
        if ((entry " " w[2]) in seen) { print "no-libm: " listfile ": " w[2] " is listed twice for " entry; bad = 1; next }
        seen[entry " " w[2]] = 1
        if (machine != "" && machine != arch) next
        if (w[1] == "linux-builtins") {
            if (os == "linux") for (i = 3; i <= n; i++) print w[2] " " w[i] > builtins
            next
        }
        if (w[1] == "linux-rust-builtins") {
            if (os == "linux") for (i = 3; i <= n; i++) print w[2] " " w[i] > rustbuiltins
            next
        }
        if (w[1] == "linux-rt-builtins") {
            if (n != 3) { print "no-libm: " listfile ": a linux-rt-builtins entry names one function that its member defines: " $0; bad = 1; next }
            if (os == "linux") print w[2] " " w[3] > rtbuiltins
            next
        }
        if (w[1] != os) next
        print w[2] > names
        for (i = 3; i <= n; i++) print w[2] " " w[i] > allowed
    }
    END { exit bad ? 1 : 0 }
' "$solver" || findings=1
# The release dependency graph holds none of the crates whose `std` code called the C math
# library (#1093); scripts/no-libm-graph.sh judges each run of `cargo tree`.
for crate in num-bigint num-integer num-traits; do
    no_libm_graph_check "$root" "$tmp" "$crate" || findings=1
done

imported=""
: > "$tmp/out"
if list -u "$una"; then
    if [ "$os" = linux ]; then list -D -u "$una" || findings=1; fi
    check "$una" una || findings=1
    imported=$(LC_ALL=C sort -u "$tmp/solver" | tr '\n' ' ' | sed 's/ $//')
    if [ -n "$imported" ]; then
        # Each of them must be an import of the Z3 library `una` was linked with, and only of
        # objects of that library that the list names for it on this platform (#1953).
        if [ -z "${UNA_Z3_LIB:-}" ] || [ ! -f "$UNA_Z3_LIB" ]; then
            echo "no-libm: $una imports $imported, which only the solver may, and UNA_Z3_LIB names no Z3 library to tie them to"
            findings=1
        else
            : > "$tmp/out"
            if list -A -u "$UNA_Z3_LIB"; then
                # `name object` for each import of a name of the list by an object of the library.
                awk -v os="$os" -v lib="$UNA_Z3_LIB" -v namesfile="$tmp/names" -v pairs="$tmp/z3" '
                    BEGIN {
                        while ((getline line < namesfile) > 0) if (line != "") listed[line] = 1
                        close(namesfile)
                        prefix = lib ":"
                        printf "" > pairs
                    }
                    /^[ \t]*$/ { next }
                    {
                        if (index($0, prefix) != 1) { print "no-libm: " lib ": a line nm wrote that names no object: " $0; bad = 1; next }
                        rest = substr($0, length(prefix) + 1)
                        colon = index(rest, ":")
                        if (colon == 0) { print "no-libm: " lib ": a line nm wrote that names no object: " $0; bad = 1; next }
                        object = substr(rest, 1, colon - 1)
                        s = $NF
                        sub(/@.*/, "", s)
                        if (os == "macos") sub(/^_/, "", s)
                        if (s in listed) print s " " object > pairs
                    }
                    END { exit bad ? 1 : 0 }
                ' "$tmp/out" || findings=1
                for name in $imported; do
                    if ! awk -v n="$name" '$1 == n { found = 1 } END { exit !found }' "$tmp/z3"; then
                        echo "no-libm: $una: $name, which the Z3 library $UNA_Z3_LIB does not import"
                        findings=1
                    fi
                    for object in $(awk -v n="$name" '$1 == n { print $2 }' "$tmp/z3" | LC_ALL=C sort -u); do
                        if ! grep -qxF "$name $object" "$tmp/allowed"; then
                            echo "no-libm: $una: $name, which the object $object of the Z3 library $UNA_Z3_LIB imports and $solver does not name for it on $os"
                            findings=1
                        fi
                    done
                done
                echo "no-libm: $una: the solver's imports, allowed by $solver from the objects it names: $imported"
            else
                findings=1
            fi
        fi
    fi
else
    findings=1
fi

if [ -n "$trace" ]; then
    # The link of the shipped `una`, by its trace (#1953).
    linked=$(sed -n '1s/^output: //p' "$trace")
    if [ -z "$linked" ] || [ "$(sed -n '1p' "$trace")" != "output: $linked" ]; then
        echo "no-libm: $trace: its first line names no linked file"
        findings=1
    elif [ ! -f "$linked" ] || ! cmp -s "$linked" "$una"; then
        echo "no-libm: $trace is the trace of $linked, which is not $una byte for byte"
        findings=1
    fi
    # Every name of the denylist must have been traced.
    sed -n '2s/^traced: //p' "$trace" | tr ' ' '\n' | awk 'NF' | LC_ALL=C sort -u > "$tmp/traced"
    sed 's/#.*//' "$deny" | awk 'NF { print $1 }' | LC_ALL=C sort -u > "$tmp/deny"
    missing=$(LC_ALL=C comm -23 "$tmp/deny" "$tmp/traced" | tr '\n' ' ' | sed 's/ $//')
    if [ -n "$missing" ]; then
        echo "no-libm: $trace does not trace $missing"
        findings=1
    fi
    if [ -z "${UNA_Z3_LIB:-}" ] || [ ! -f "$UNA_Z3_LIB" ]; then
        echo "no-libm: $trace: UNA_Z3_LIB names no Z3 library to tie the trace's references to"
        findings=1
        z3_digest=none
    else
        z3_digest=present
    fi
    # The archive members that the trace's references name and that are, byte for byte, the
    # member of that name of the Z3 library (`ar p`): the Z3 library's objects reach the link
    # inside una-z3's rlib, so an object is matched by its bytes, not by the archive's path.
    : > "$tmp/z3members"
    sed -n '3,$s/^reference [^ ]* \(.*\)$/\1/p' "$trace" | grep '([^()]*)$' | LC_ALL=C sort -u > "$tmp/members"
    while IFS= read -r object_file; do
        [ "$z3_digest" != none ] || break
        member=$(printf '%s\n' "$object_file" | sed 's/^.*(\([^()]*\))$/\1/')
        archive_file=$(printf '%s\n' "$object_file" | sed 's/([^()]*)$//')
        [ -f "$archive_file" ] || continue
        if ar p "$archive_file" "$member" > "$tmp/member" 2> /dev/null && [ -s "$tmp/member" ] \
            && ar p "$UNA_Z3_LIB" "$member" > "$tmp/z3member" 2> /dev/null && [ -s "$tmp/z3member" ] \
            && cmp -s "$tmp/member" "$tmp/z3member"; then
            printf '%s\n' "$object_file" >> "$tmp/z3members"
        fi
    done < "$tmp/members"
    # The pinned toolchain's compiler_builtins, which may define a name of the `linux-builtins`
    # entries.
    sysroot=$(cd "$root" && rustc --print sysroot 2> /dev/null) || sysroot=""
    host=$(cd "$root" && rustc -vV 2> /dev/null | sed -n 's/^host: //p') || host=""
    if [ -z "$sysroot" ] || [ -z "$host" ]; then
        echo "no-libm: rustc names no sysroot, so no definition by the toolchain's compiler_builtins can be accepted"
        findings=1
        builtins_prefix=none
    else
        builtins_prefix=$sysroot/lib/rustlib/$host/lib/libcompiler_builtins-
    fi
    : > "$tmp/referenced"
    awk -v trace="$trace" -v denyfile="$tmp/deny" -v allowedfile="$tmp/allowed" \
        -v builtinsfile="$tmp/builtins" -v rustfile="$tmp/rustbuiltins" -v membersfile="$tmp/z3members" -v cb="$builtins_prefix" \
        -v referenced="$tmp/referenced" '
        BEGIN {
            while ((getline line < denyfile) > 0) denied[line] = 1
            close(denyfile)
            while ((getline line < allowedfile) > 0) allowed[line] = 1
            close(allowedfile)
            while ((getline line < builtinsfile) > 0) {
                builtin[line] = 1
                split(line, bw, " ")
                builtin_name[bw[1]] = 1
            }
            close(builtinsfile)
            # The `linux-rust-builtins` entries (#2354): Rust objects, by their exact member
            # names, whose references to a name resolve to compiler_builtins.
            while ((getline line < rustfile) > 0) {
                rust[line] = 1
                split(line, rw, " ")
                builtin_name[rw[1]] = 1
            }
            close(rustfile)
            while ((getline line < membersfile) > 0) z3member[line] = 1
            close(membersfile)
        }
        NR <= 2 { next }
        /^[ \t]*$/ { next }
        {
            kind = $1
            sym = $2
            file = substr($0, length(kind) + length(sym) + 3)
            if ((kind != "reference" && kind != "definition") || sym == "" || file == "") {
                print "no-libm: " trace ": a line that is not a reference or a definition: " $0
                bad = 1
                next
            }
            if (!(sym in denied)) next
            if (kind == "definition") {
                if (file ~ /\.so(\.[0-9]+)*$/) next
                if ((sym in builtin_name) && cb != "none" && index(file, cb) == 1 && file ~ /\.rlib\([^()]*\)$/) next
                print "no-libm: " trace ": " file " defines " sym ", outside a shared library, and not by the compiler_builtins of the pinned toolchain for a name of the linux-builtins entries"
                bad = 1
                next
            }
            ok = 0
            if ((file in z3member) && match(file, /\([^()]*\)$/)) {
                member = substr(file, RSTART + 1, RLENGTH - 2)
                ok = ((sym " " member) in allowed) || ((sym " " member) in builtin)
            }
            if (!ok && match(file, /\([^()]*\)$/)) {
                # A `linux-rust-builtins` member, inside the rlib of its own crate (#2364): the
                # member `<crate>-<hash>.<...>.rcgu.o` of `.../lib<crate>-<hash>.rlib`.
                member = substr(file, RSTART + 1, RLENGTH - 2)
                library = substr(file, 1, RSTART - 1)
                sub(/.*\//, "", library)
                crate = member
                sub(/\..*/, "", crate)
                ok = ((sym " " member) in rust) && library == ("lib" crate ".rlib")
            }
            if (ok) { print sym > referenced; next }
            print "no-libm: " trace ": " file " references " sym ", and is not, byte for byte, an object of the Z3 library that scripts/no-libm-una.txt names for it"
            bad = 1
        }
        END { exit bad ? 1 : 0 }
    ' "$trace" || findings=1
    # Each solver name `una` imports comes from such a reference.
    for name in $imported; do
        if ! grep -qxF "$name" "$tmp/referenced"; then
            echo "no-libm: $trace shows no reference of an object of the Z3 library to $name, which $una imports"
            findings=1
        fi
    done
    echo "no-libm: $trace: every reference to a name of $deny is from an object of the Z3 library that $solver names for it"
fi

: > "$tmp/out"
if [ -z "$trace" ]; then
    if list -u "$rust_una"; then
        if [ "$os" = linux ]; then list -D -u "$rust_una" || findings=1; fi
        check "$rust_una" || findings=1
    else
        findings=1
    fi
fi

: > "$tmp/rtallowed"
if [ "$os" = linux ] && { [ -s "$tmp/rustbuiltins" ] || [ -s "$tmp/rtbuiltins" ]; }; then
    # The runtime's references that the Owner's decisions of 2026-11-27 allow (#2354), each only
    # where the same archive defines the name in a member of the toolchain's compiler_builtins,
    # so that a link of the archive binds it there and never to the platform's libm:
    # `linux-rust-builtins` names exact members (a Rust crate's object), `linux-rt-builtins` a
    # compiler_builtins member by the function it defines (its role), never by its file name.
    : > "$tmp/out"
    if list -A -C --defined-only "$archive"; then
        awk -v lib="$archive" -v rustfile="$tmp/rustbuiltins" -v rtfile="$tmp/rtbuiltins" -v out="$tmp/rtallowed" '
            BEGIN {
                while ((getline line < rustfile) > 0) if (line != "") rust[line] = 1
                close(rustfile)
                while ((getline line < rtfile) > 0) if (line != "") { split(line, w, " "); role[w[2]] = role[w[2]] " " w[1] }
                close(rtfile)
                prefix = lib ":"
            }
            index($0, prefix) == 1 {
                rest = substr($0, length(prefix) + 1)
                colon = index(rest, ":")
                if (colon == 0) next
                member = substr(rest, 1, colon - 1)
                rest = substr(rest, colon + 1)
                sub(/^[0-9a-fA-F]* +/, "", rest)
                kind = substr(rest, 1, 1)
                sym = substr(rest, 3)
                gsub(/\[[0-9a-f]+\]/, "", sym)
                if (kind != "T" && kind != "W") next
                if (index(member, "compiler_builtins-") == 1) {
                    cbdef[sym] = 1
                    if (sym in role) roles[member] = roles[member] role[sym]
                }
            }
            END {
                for (pair in rust) { split(pair, p, " "); if (p[1] in cbdef) print pair > out }
                for (member in roles) {
                    k = split(roles[member], names, " ")
                    for (i = 1; i <= k; i++) if (names[i] in cbdef) print names[i] " " member > out
                }
            }
        ' "$tmp/out" || findings=1
    else
        findings=1
    fi
fi
: > "$tmp/out"
if list -A -u "$archive"; then
    check "$archive" archive "$tmp/rtallowed" || findings=1
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

if [ -n "$trace" ]; then what="the link trace $trace"; else what=$rust_una; fi
if [ "$findings" -ne 0 ]; then
    fail "a std feature, a C math library symbol, or a cargo tree or nm error above, in $una, $what, $archive or $count executables of $executables"
fi
echo "no-libm: PASS ($una, $what, $archive and $count executables of $executables)"
