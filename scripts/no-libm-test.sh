#!/bin/sh
# Canaries of scripts/no-libm.sh (#1277): small artifacts, compiled here with the pinned rustc,
# each of which leaves one C math library name undefined, must make the check fail with that
# name, and the clean ones must pass. They cover the scopes: a name of scripts/no-libm-una.txt
# is allowed in `una` alone and only when the Z3 library imports it, never in `una` built
# without the solver, in `libuna_rt.a` or in an executable that `una build` links, and any
# other name of scripts/no-libm.txt fails everywhere. Every name of scripts/no-libm-una.txt is
# then checked on its own in each of those scopes (#1951), and the tie to the Z3 library is by
# object (#1953): the stand-in Z3 libraries hold members named as the objects the list names, and
# a name imported by another member of the Z3 library fails. One `una` imports two names of the
# list, with a Z3 library that imports only the first, and must fail on the second. On Linux the
# check of the shipped `una` by its link trace (--link-trace) is given real links, made with
# scripts/link-trace.sh as scripts/release.sh makes them, and each of its rules a link or a
# trace that breaks it.
# The canaries declare the C function themselves (`extern "C"`), in crates marked
# `#![no_builtins]`, so that LLVM keeps each call (it would turn a call of `sqrt` or `ceil` into an
# instruction) and each artifact imports exactly the names it is meant to. The real artifacts are
# never changed.
#
# The release graph check (scripts/no-libm-graph.sh, #1949) has canaries of its own: its verdict
# on each kind of `cargo tree` result, given as files, and on the real graph of this repository,
# where una-core is held (a finding), num-bigint is in Cargo.lock but not in the graph, and a
# name that no package has is not in Cargo.lock (neither is a finding).
#
# Usage: scripts/no-libm-test.sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
check="$root/scripts/no-libm.sh"
work=$(mktemp -d "${TMPDIR:-/tmp}/no-libm-test.XXXXXX")
trap 'rm -rf "${work:?}"' EXIT
status=0

case $(uname -s) in
Darwin) os=macos ;;
Linux) os=linux ;;
*) echo "error: no rule for the platform $(uname -s)" >&2; exit 1 ;;
esac
list=$root/scripts/no-libm-una.txt
# The names the solver's list holds for this platform, in its order, and the first object it
# names for a name.
# An entry `PLATFORM@MACHINE` holds on that processor only (`uname -m`, #2365).
machine=$(uname -m)
names=$(sed 's/#.*//' "$list" | awk -v os="$os" -v m="$machine" '$1 == os || $1 == os "@" m { print $2 }')
object_of() { sed 's/#.*//' "$list" | awk -v os="$os" -v m="$machine" -v n="$1" '($1 == os || $1 == os "@" m) && $2 == n { print $3; exit }'; }
# The first allowed name of the solver's list, and a denied name that it does not hold.
allowed=$(printf '%s\n' "$names" | sed -n 1p)
[ -n "$allowed" ] || { echo "error: scripts/no-libm-una.txt lists no name for $os" >&2; exit 1; }
allowed_object=$(object_of "$allowed")
denied=cbrt
if sed 's/#.*//' "$list" | awk '{ print $2 }' | grep -qxF "$denied"; then
    echo "error: the canary's denied name $denied is on scripts/no-libm-una.txt" >&2
    exit 1
fi

# rustc of the pinned toolchain, run from the repository so that rust-toolchain.toml applies.
# On x86_64-unknown-linux-gnu it links with GNU ld, as scripts/release.sh links `una`, not with
# rustc's default rust-lld, so that the canaries' links and traces are made as the shipped one's
# (#2340).
linker_flags=""
case $(cd "$root" && rustc -vV | sed -n 's/^host: //p') in
    x86_64-unknown-linux-gnu) linker_flags="-C linker-features=-lld" ;;
esac
compile() {
    # shellcheck disable=SC2086 # linker_flags is empty or two words
    (cd "$root" && rustc --edition 2021 -O $linker_flags "$@") > "$work/rustc.log" 2>&1 || {
        cat "$work/rustc.log" >&2
        echo "error: rustc $* failed" >&2
        exit 1
    }
}

# A Rust source whose `main` (or, with `lib`, an exported function) calls the C functions named.
rust_source() {
    kind=$1
    shift
    {
        echo '#![allow(unused)]'
        echo '#![no_builtins]'
        printf '#[cfg_attr(target_os = "linux", link(name = "m"))]\nextern "C" {\n'
        for name in "$@"; do printf '    fn %s(x: f64) -> f64;\n' "$name"; done
        echo '}'
        if [ "$kind" = lib ]; then
            printf '#[no_mangle]\npub extern "C" fn %s(x: f64) -> f64 {\n    let mut y = x;\n' "${exported:-una_canary}"
        else
            printf 'fn main() {\n    let mut y = std::hint::black_box(2.0f64);\n'
        fi
        for name in "$@"; do printf '    y += unsafe { %s(y) };\n' "$name"; done
        if [ "$kind" = lib ]; then echo '    y'; else echo '    println!("{y}");'; fi
        echo '}'
    }
}

# exe NAME FUNCTION...: an executable at $work/NAME.
exe() {
    out=$1
    shift
    rust_source main "$@" > "$work/$out.rs"
    compile -o "$work/$out" "$work/$out.rs"
}

# archive NAME FUNCTION...: a static archive at $work/NAME.a holding one object.
archive() {
    out=$1
    shift
    rust_source lib "$@" > "$work/$out.rs"
    compile --crate-type lib --emit obj -o "$work/$out.o" "$work/$out.rs"
    rm -f "$work/$out.a"
    ar rc "$work/$out.a" "$work/$out.o" > "$work/ar.log" 2>&1 || {
        cat "$work/ar.log" >&2
        echo "error: ar failed" >&2
        exit 1
    }
    ranlib "$work/$out.a" 2> /dev/null || true
}

# z3lib NAME MEMBER:FUNCTION...: a stand-in for the Z3 library at $work/NAME.a, one member per
# argument, named MEMBER and importing FUNCTION (none when it is empty), each exporting a
# function of its own.
z3lib() {
    out=$1
    shift
    rm -rf "${work:?}/members-$out" "$work/$out.a"
    mkdir "$work/members-$out"
    index=0
    for pair in "$@"; do
        index=$((index + 1))
        member=${pair%%:*} function=${pair#*:}
        exported=una_${out}_$index
        # shellcheck disable=SC2086 # an empty FUNCTION is no argument
        rust_source lib $function > "$work/members-$out/$index.rs"
        exported=
        compile --crate-type lib --emit obj -o "$work/members-$out/$member" "$work/members-$out/$index.rs"
        (cd "$work/members-$out" && ar rc "$work/$out.a" "$member") > "$work/ar.log" 2>&1 || {
            cat "$work/ar.log" >&2
            echo "error: ar failed" >&2
            exit 1
        }
    done
    ranlib "$work/$out.a" 2> /dev/null || true
}

# members NAME MEMBER:IMPORT:EXPORT...: a static archive at $work/NAME.a, one member per
# argument, named MEMBER, importing IMPORT (none when empty) and exporting the C symbol EXPORT
# (its exact name, mangled or not), for the runtime's entries of scripts/no-libm-una.txt (#2354).
members() {
    out=$1
    shift
    rm -rf "${work:?}/members-$out" "$work/$out.a"
    mkdir "$work/members-$out"
    index=0
    for triple in "$@"; do
        index=$((index + 1))
        member=${triple%%:*} rest=${triple#*:}
        import=${rest%%:*} export_name=${rest#*:}
        {
            echo '#![allow(unused)]'
            echo '#![no_builtins]'
            if [ -n "$import" ]; then printf 'extern "C" {\n    fn %s(x: f64) -> f64;\n}\n' "$import"; fi
            printf '#[export_name = "%s"]\npub extern "C" fn exported(x: f64) -> f64 {\n' "$export_name"
            if [ -n "$import" ]; then printf '    unsafe { %s(x) }\n' "$import"; else echo '    x'; fi
            echo '}'
        } > "$work/members-$out/$index.rs"
        compile --crate-type lib --emit obj -o "$work/members-$out/$member" "$work/members-$out/$index.rs"
        (cd "$work/members-$out" && ar rc "$work/$out.a" "$member") > "$work/ar.log" 2>&1 || {
            cat "$work/ar.log" >&2
            echo "error: ar failed" >&2
            exit 1
        }
    done
    ranlib "$work/$out.a" 2> /dev/null || true
}

# A directory holding the executable $1 alone.
dir_of() {
    rm -rf "${work:?}/exes-$1"
    mkdir "$work/exes-$1"
    cp "$work/$1" "$work/exes-$1/"
    echo "$work/exes-$1"
}

exe clean_una                   # no math name
exe solver_una "$allowed"       # the solver's name, as `una` imports it
exe denied_una "$allowed" "$denied"
exe clean_exe
exe allowed_exe "$allowed"      # the solver's name in an emitted executable
exe denied_exe "$denied"
archive clean_rt
archive allowed_rt "$allowed"   # the solver's name in the runtime
archive denied_rt "$denied"
z3lib z3 "$allowed_object:$allowed"  # a stand-in for the Z3 library, importing the solver's name
z3lib other_z3 "$allowed_object:"    # a Z3 library that does not import it
z3lib elsewhere_z3 "$allowed_object:$allowed" "elsewhere.cpp.o:$allowed"  # and from an object the list does not name

# run NAME UNA_Z3_LIB UNA LIBUNA_RT EXECUTABLES: runs the check, its output in $work/NAME.log,
# with `una` without the solver $rust (clean_una unless set).
# With --link-trace, the check's trace mode, which has no UNA_WITHOUT_SOLVER.
run() {
    name=$1 z3=$2
    shift 2
    if [ "$1" = --link-trace ]; then
        UNA_Z3_LIB=$z3 "$check" "$@" > "$work/$name.log" 2>&1
    elif [ -n "$z3" ]; then
        UNA_Z3_LIB=$z3 "$check" "$@" "${rust:-$work/clean_una}" > "$work/$name.log" 2>&1
    else
        env -u UNA_Z3_LIB "$check" "$@" "${rust:-$work/clean_una}" > "$work/$name.log" 2>&1
    fi
}

# pass NAME ARGS...: the check must pass.
pass() {
    name=$1
    shift
    if ! run "$name" "$@"; then
        echo "error: canary $name fails the check:" >&2
        cat "$work/$name.log" >&2
        status=1
    fi
}

# reject NAME MESSAGE ARGS...: the check must fail, and say MESSAGE.
reject() {
    name=$1 message=$2
    shift 2
    if run "$name" "$@"; then
        echo "error: canary $name passes the check" >&2
        status=1
    elif ! grep -qF -- "$message" "$work/$name.log"; then
        echo "error: canary $name fails for another reason than \"$message\":" >&2
        cat "$work/$name.log" >&2
        status=1
    fi
}

pass clean "" "$work/clean_una" "$work/clean_rt.a" "$(dir_of clean_exe)"
pass solver "$work/z3.a" "$work/solver_una" "$work/clean_rt.a" "$(dir_of clean_exe)"
reject una-denied "$work/denied_una: $denied" \
    "$work/z3.a" "$work/denied_una" "$work/clean_rt.a" "$(dir_of clean_exe)"
reject una-no-z3 "UNA_Z3_LIB names no Z3 library" \
    "" "$work/solver_una" "$work/clean_rt.a" "$(dir_of clean_exe)"
reject una-not-from-z3 "$allowed, which the Z3 library $work/other_z3.a does not import" \
    "$work/other_z3.a" "$work/solver_una" "$work/clean_rt.a" "$(dir_of clean_exe)"
reject una-not-from-listed-object "$allowed, which the object elsewhere.cpp.o of the Z3 library $work/elsewhere_z3.a imports" \
    "$work/elsewhere_z3.a" "$work/solver_una" "$work/clean_rt.a" "$(dir_of clean_exe)"
rust=$work/solver_una
reject rust-solver-name "$work/solver_una: $allowed" \
    "$work/z3.a" "$work/clean_una" "$work/clean_rt.a" "$(dir_of clean_exe)"
rust=
reject runtime-solver-name "$work/allowed_rt.a(allowed_rt.o): $allowed" \
    "$work/z3.a" "$work/solver_una" "$work/allowed_rt.a" "$(dir_of clean_exe)"
reject runtime-denied "$work/denied_rt.a(denied_rt.o): $denied" \
    "$work/z3.a" "$work/clean_una" "$work/denied_rt.a" "$(dir_of clean_exe)"
reject executable-solver-name "allowed_exe: $allowed" \
    "$work/z3.a" "$work/solver_una" "$work/clean_rt.a" "$(dir_of allowed_exe)"
reject executable-denied "denied_exe: $denied" \
    "$work/z3.a" "$work/clean_una" "$work/clean_rt.a" "$(dir_of denied_exe)"

# Every name of the solver's list, each on its own (#1951): allowed in `una` when the stand-in Z3
# library imports that very name, and rejected in `una` when the Z3 library imports only another
# name of the list, in `una` without the solver, in `libuna_rt.a` and in an executable. The
# canaries are linked, never run, so each C function is declared `f64 -> f64` whatever its real
# signature (`frexp` takes a pointer too).
first=$(printf '%s\n' "$names" | sed -n 1p)
second=$(printf '%s\n' "$names" | sed -n 2p)
[ -n "$second" ] || { echo "error: scripts/no-libm-una.txt lists fewer than two names for $os" >&2; exit 1; }
named=0
for listed in $names; do
    named=$((named + 1))
    if [ "$listed" = "$first" ]; then other=$second; else other=$first; fi
    object=$(object_of "$listed")
    exe "una_$listed" "$listed"
    exe "exe_$listed" "$listed"
    archive "rt_$listed" "$listed"
    z3lib "z3_$listed" "$object:$listed"
    z3lib "z3_not_$listed" "$(object_of "$other"):$other"
    z3lib "z3_elsewhere_$listed" "elsewhere.cpp.o:$listed"
    pass "solver-$listed" "$work/z3_$listed.a" "$work/una_$listed" "$work/clean_rt.a" "$(dir_of clean_exe)"
    reject "una-not-from-z3-$listed" "$listed, which the Z3 library $work/z3_not_$listed.a does not import" \
        "$work/z3_not_$listed.a" "$work/una_$listed" "$work/clean_rt.a" "$(dir_of clean_exe)"
    reject "una-not-from-listed-object-$listed" "$listed, which the object elsewhere.cpp.o of the Z3 library $work/z3_elsewhere_$listed.a imports" \
        "$work/z3_elsewhere_$listed.a" "$work/una_$listed" "$work/clean_rt.a" "$(dir_of clean_exe)"
    rust=$work/una_$listed
    reject "rust-$listed" "$work/una_$listed: $listed" \
        "$work/z3_$listed.a" "$work/clean_una" "$work/clean_rt.a" "$(dir_of clean_exe)"
    rust=
    reject "runtime-$listed" "$work/rt_$listed.a(rt_$listed.o): $listed" \
        "$work/z3_$listed.a" "$work/clean_una" "$work/rt_$listed.a" "$(dir_of clean_exe)"
    reject "executable-$listed" "exe_$listed: $listed" \
        "$work/z3_$listed.a" "$work/clean_una" "$work/clean_rt.a" "$(dir_of "exe_$listed")"
done

# Two names of the list in one `una` (#1953): allowed when the Z3 library imports both, each from
# an object the list names for it, and rejected on the second when it imports only the first.
exe two_una "$first" "$second"
z3lib two_z3 "$(object_of "$first"):$first" "$(object_of "$second"):$second"
pass two-names "$work/two_z3.a" "$work/two_una" "$work/clean_rt.a" "$(dir_of clean_exe)"
reject two-names-second-not-from-z3 "$second, which the Z3 library $work/z3_$first.a does not import" \
    "$work/z3_$first.a" "$work/two_una" "$work/clean_rt.a" "$(dir_of clean_exe)"
if grep -qF "$first, which the Z3 library" "$work/two-names-second-not-from-z3.log"; then
    echo "error: canary two-names-second-not-from-z3 rejects $first, which the Z3 library imports" >&2
    status=1
fi

# The check of the shipped `una` by its link trace (#1953), Linux only, where scripts/release.sh
# links through scripts/link-trace.sh: real links of a `una` that calls a stand-in Z3 library,
# each made by that linker, and traces that break one rule each.
traced=0
if [ "$os" = linux ]; then
    tname=$(printf '%s\n' "$names" | sed -n 1p)
    tobject=$(object_of "$tname")
    z3lib libz3t "$tobject:$tname"
    z3lib libz3e "elsewhere.cpp.o:$tname"
    z3lib libcxxt "hash_tr1.o:$tname"
    z3lib libz3other "$tobject:$tname" "extra.cpp.o:"
    mkdir -p "$work/copy"
    cp "$work/libz3other.a" "$work/copy/libz3t.a"  # another library at a path of the same name
    # traced_exe NAME LIBS DIRECT [CALLS]: `una` at $work/NAME, its link trace at $work/NAME.trace,
    # linked with the stand-in libraries LIBS; it calls the functions CALLS (by default the first
    # one of each library of LIBS), and DIRECT (a name or nothing) itself.
    traced_exe() {
        tout=$1 libs=$2 direct=$3 calls=${4:-}
        if [ -z "$calls" ]; then
            for lib in $libs; do calls="$calls una_${lib}_1"; done
        fi
        {
            echo '#![allow(unused)]'
            echo '#![no_builtins]'
            if [ -n "${NOLM:-}" ]; then printf 'extern "C" {\n'; else printf '#[cfg_attr(target_os = "linux", link(name = "m"))]\nextern "C" {\n'; fi
            for call in $calls; do printf '    fn %s(x: f64) -> f64;\n' "$call"; done
            [ -z "$direct" ] || printf '    fn %s(x: f64) -> f64;\n' "$direct"
            printf '}\nfn main() {\n    let mut y = std::hint::black_box(2.0f64);\n'
            for call in $calls; do printf '    y += unsafe { %s(y) };\n' "$call"; done
            [ -z "$direct" ] || printf '    y += unsafe { %s(y) };\n' "$direct"
            echo '    println!("{y}");'
            echo '}'
        } > "$work/$tout.rs"
        set --
        for lib in $libs; do
            case $lib in
            *.rlib) set -- "$@" -l "static:+verbatim=$lib" ;;  # a Rust library, by its file name
            *) set -- "$@" -l "static=${lib#lib}" ;;
            esac
        done
        rm -rf "${work:?}/traces-$tout"
        mkdir "$work/traces-$tout"
        UNA_LINK_TRACE_DIR=$work/traces-$tout
        export UNA_LINK_TRACE_DIR
        compile -o "$work/$tout" "$work/$tout.rs" -L "$work" "$@" -C "linker=$root/scripts/link-trace.sh"
        unset UNA_LINK_TRACE_DIR
        set -- "$work/traces-$tout"/link.*
        [ "$#" -eq 1 ] && [ -f "$1" ] || { echo "error: the link of $tout left not exactly one trace" >&2; exit 1; }
        cp "$1" "$work/$tout.trace"
    }
    # The Z3 objects reach the shipped link inside una-z3's rlib: libbundle.a holds libz3t.a's
    # member, the same bytes, in another archive. libz3b's member refers to a name of the
    # `linux-builtins` entries, which the link resolves to the toolchain's compiler_builtins.
    bname=$(sed 's/#.*//' "$list" | awk '$1 == "linux-builtins" { print $2; exit }')
    bobject=$(sed 's/#.*//' "$list" | awk -v n="$bname" '$1 == "linux-builtins" && $2 == n { print $3; exit }')
    [ -n "$bname" ] && [ -n "$bobject" ] || { echo "error: scripts/no-libm-una.txt has no linux-builtins entry" >&2; exit 1; }
    rm -rf "${work:?}/bundle"
    mkdir "$work/bundle"
    (cd "$work/bundle" && ar x "$work/libz3t.a" "$tobject" && ar rc "$work/libbundle.a" "$tobject") \
        || { echo "error: cannot make libbundle.a" >&2; exit 1; }
    z3lib libz3b "$tobject:$tname" "$bobject:$bname"
    traced_exe t_una libz3t ""
    traced_exe t_bundle libbundle "" una_libz3t_1
    traced_exe t_builtin libz3b "" "una_libz3b_1 una_libz3b_2"
    traced_exe t_own_builtin libz3t "$bname"
    traced_exe t_own libz3t "$tname"
    traced_exe t_cxx "libz3t libcxxt" ""
    traced_exe t_else libz3e ""
    # A link through scripts/link-trace.sh with gold is refused, on every Linux target (#2353).
    traced=$((traced + 1))
    cp "$work/t_una.rs" "$work/t_gold.rs"
    mkdir -p "$work/traces-gold"
    # shellcheck disable=SC2086 # linker_flags is empty or two words
    if (cd "$root" && UNA_LINK_TRACE_DIR=$work/traces-gold rustc --edition 2021 -O $linker_flags -o "$work/t_gold" "$work/t_gold.rs" \
        -L "$work" -l static=z3t -C link-arg=-fuse-ld=gold -C "linker=$root/scripts/link-trace.sh") > "$work/t_gold.log" 2>&1; then
        echo "error: canary trace-gold: a link with gold through scripts/link-trace.sh passes" >&2
        status=1
    elif ! grep -qF "link-trace: the link would use gold" "$work/t_gold.log"; then
        echo "error: canary trace-gold fails for another reason than the refusal of gold:" >&2
        cat "$work/t_gold.log" >&2
        status=1
    fi
    # On x86_64-unknown-linux-gnu, a link through scripts/link-trace.sh with rustc's default
    # rust-lld is refused, never read as a trace with no reference (#2340).
    if [ -n "$linker_flags" ]; then
        traced=$((traced + 1))
        cp "$work/t_una.rs" "$work/t_lld.rs"
        mkdir -p "$work/traces-lld"
        if (cd "$root" && UNA_LINK_TRACE_DIR=$work/traces-lld rustc --edition 2021 -O -o "$work/t_lld" "$work/t_lld.rs" \
            -L "$work" -l static=z3t -C "linker=$root/scripts/link-trace.sh") > "$work/t_lld.log" 2>&1; then
            echo "error: canary trace-lld: a link with rust-lld through scripts/link-trace.sh passes" >&2
            status=1
        elif ! grep -qE "link-trace: the link would use (lld|rustc's lld wrapper)" "$work/t_lld.log"; then
            echo "error: canary trace-lld fails for another reason than the refusal of lld:" >&2
            cat "$work/t_lld.log" >&2
            status=1
        fi
    fi
    # tpass NAME UNA_Z3_LIB TRACE UNA, treject NAME MESSAGE UNA_Z3_LIB TRACE UNA: the check in
    # its trace mode.
    tpass() { traced=$((traced + 1)); pass "$1" "$2" --link-trace "$3" "$4" "$work/clean_rt.a" "$(dir_of clean_exe)"; }
    treject() { traced=$((traced + 1)); reject "$1" "$2" "$3" --link-trace "$4" "$5" "$work/clean_rt.a" "$(dir_of clean_exe)"; }
    tpass trace-clean "$work/libz3t.a" "$work/t_una.trace" "$work/t_una"
    tpass trace-z3-object-in-another-archive "$work/libz3t.a" "$work/t_bundle.trace" "$work/t_bundle"
    tpass trace-builtins "$work/libz3b.a" "$work/t_builtin.trace" "$work/t_builtin"
    treject trace-builtins-own-reference "rcgu.o references $bname" \
        "$work/libz3t.a" "$work/t_own_builtin.trace" "$work/t_own_builtin"
    sed "s|^definition $bname .*|definition $bname /elsewhere/libcompiler_builtins-0.rlib(a.o)|" "$work/t_builtin.trace" > "$work/t_builtin_elsewhere.trace"
    treject trace-builtins-elsewhere "/elsewhere/libcompiler_builtins-0.rlib(a.o) defines $bname" \
        "$work/libz3b.a" "$work/t_builtin_elsewhere.trace" "$work/t_builtin"
    # compiler_builtins' definition of the builtins name, given as the definition of the solver's.
    cbdef=$(awk -v n="$bname" '$1 == "definition" && $2 == n && $3 ~ /libcompiler_builtins-/ { print $3; exit }' "$work/t_builtin.trace")
    [ -n "$cbdef" ] || { echo "error: the trace of t_builtin shows no definition of $bname by compiler_builtins" >&2; status=1; }
    { awk -v n="$tname" '!($1 == "definition" && $2 == n)' "$work/t_builtin.trace"; echo "definition $tname $cbdef"; } \
        > "$work/t_builtin_solver.trace"
    treject trace-builtins-not-for-solver-name "defines $tname" \
        "$work/libz3b.a" "$work/t_builtin_solver.trace" "$work/t_builtin"
    treject trace-own-reference "rcgu.o references $tname" \
        "$work/libz3t.a" "$work/t_own.trace" "$work/t_own"
    sed -n '3,$p' "$work/t_own.trace" | grep -q "^reference $tname .*t_own.*rcgu\.o$" \
        || { echo "error: the trace of t_own names no reference of its own object to $tname" >&2; status=1; }
    treject trace-libstdcxx-reference "$work/libcxxt.a(hash_tr1.o) references $tname" \
        "$work/libz3t.a" "$work/t_cxx.trace" "$work/t_cxx"
    treject trace-unlisted-object "$work/libz3e.a(elsewhere.cpp.o) references $tname" \
        "$work/libz3e.a" "$work/t_else.trace" "$work/t_else"
    treject trace-other-z3 "$work/libz3t.a($tobject) references $tname" \
        "$work/copy/libz3t.a" "$work/t_una.trace" "$work/t_una"
    treject trace-not-this-una "which is not $work/t_cxx byte for byte" \
        "$work/libz3t.a" "$work/t_una.trace" "$work/t_cxx"
    sed "2s/ $denied / /" "$work/t_una.trace" > "$work/t_untraced.trace"
    treject trace-name-not-traced "does not trace $denied" \
        "$work/libz3t.a" "$work/t_untraced.trace" "$work/t_una"
    sed "/^reference $tname /d" "$work/t_una.trace" > "$work/t_noref.trace"
    treject trace-no-reference "shows no reference of an object of the Z3 library to $tname" \
        "$work/libz3t.a" "$work/t_noref.trace" "$work/t_una"
    { cat "$work/t_una.trace"; echo "definition $tname $work/libmstatic.a(e_$tname.o)"; } > "$work/t_static.trace"
    treject trace-static-definition "$work/libmstatic.a(e_$tname.o) defines $tname, outside a shared library" \
        "$work/libz3t.a" "$work/t_static.trace" "$work/t_una"
    { cat "$work/t_una.trace"; echo "relocation $tname somewhere"; } > "$work/t_garbled.trace"
    treject trace-garbled "a line that is not a reference or a definition" \
        "$work/libz3t.a" "$work/t_garbled.trace" "$work/t_una"
    { sed -n 1p "$work/t_una.trace" | sed 's/^output: /output- /'; sed -n '2,$p' "$work/t_una.trace"; } > "$work/t_nooutput.trace"
    treject trace-no-output "its first line names no linked file" \
        "$work/libz3t.a" "$work/t_nooutput.trace" "$work/t_una"

    # The runtime's entries of the Owner's decisions of 2026-11-27 (#2354): a Rust object's
    # reference that resolves to compiler_builtins (`linux-rust-builtins`, by the exact member)
    # and a compiler_builtins member's own (`linux-rt-builtins`, by the function it defines).
    rentry=$(sed 's/#.*//' "$list" | awk '$1 ~ /^linux-rust-builtins(@|$)/ { print $1; exit }')
    rname=$(sed 's/#.*//' "$list" | awk '$1 ~ /^linux-rust-builtins(@|$)/ { print $2; exit }')
    robject=$(sed 's/#.*//' "$list" | awk -v n="$rname" '$1 ~ /^linux-rust-builtins(@|$)/ && $2 == n { print $3; exit }')
    rtentry=$(sed 's/#.*//' "$list" | awk '$1 ~ /^linux-rt-builtins(@|$)/ { print $1; exit }')
    rtname=$(sed 's/#.*//' "$list" | awk '$1 ~ /^linux-rt-builtins(@|$)/ { print $2; exit }')
    rtrole=$(sed 's/#.*//' "$list" | awk '$1 ~ /^linux-rt-builtins(@|$)/ { print $3; exit }')
    # Whether each entry holds here: on another processor than its `@MACHINE`, the cases that
    # it would allow must fail instead (#2365).
    holds() { case $1 in *@*) [ "${1#*@}" = "$machine" ] ;; *) true ;; esac; }
    # allowed CASE MESSAGE ENTRY ARGS...: passes where ENTRY holds, else fails with MESSAGE.
    allowed() {
        acase=$1 amessage=$2 aentry=$3
        shift 3
        if holds "$aentry"; then pass "$acase" "$@"; else reject "$acase-not-on-$machine" "$amessage" "$@"; fi
    }
    [ -n "$rname" ] && [ -n "$robject" ] && [ -n "$rtname" ] && [ -n "$rtrole" ] \
        || { echo "error: scripts/no-libm-una.txt has no linux-rust-builtins or linux-rt-builtins entry" >&2; exit 1; }
    # The legacy mangling of the role's path, which nm -C writes back as that path.
    mangled=_ZN$(printf '%s' "$rtrole" | awk -F'::' '{ for (i = 1; i <= NF; i++) printf "%d%s", length($i), $i }')17h0123456789abcdefE
    rtcase() { traced=$((traced + 1)); "$@"; }
    clean_dir=$(dir_of clean_exe)
    # The Rust object's reference, in the runtime: with compiler_builtins' definition beside it,
    # it passes; without that definition, from another member, it fails.
    members rt_rust_ok "$robject:$rname:una_rt_rust_1" "compiler_builtins-0.rcgu.o::$rname"
    members rt_rust_nodef "$robject:$rname:una_rt_rust_1"
    members rt_rust_other "other.rcgu.o:$rname:una_rt_rust_1" "compiler_builtins-0.rcgu.o::$rname"
    rtcase allowed rt-rust-builtins "rt_rust_ok.a($robject): $rname" "$rentry" "" "$work/clean_una" "$work/rt_rust_ok.a" "$clean_dir"
    rtcase reject rt-rust-builtins-no-definition "rt_rust_nodef.a($robject): $rname" \
        "" "$work/clean_una" "$work/rt_rust_nodef.a" "$clean_dir"
    rtcase reject rt-rust-builtins-other-member "rt_rust_other.a(other.rcgu.o): $rname" \
        "" "$work/clean_una" "$work/rt_rust_other.a" "$clean_dir"
    # compiler_builtins' own reference, by the member that defines the role: it passes beside
    # compiler_builtins' definition; from a member of another role, from a member that is not
    # compiler_builtins', or with no definition, it fails.
    members rt_cb_ok "compiler_builtins-1.rcgu.o:$rtname:$mangled" "compiler_builtins-2.rcgu.o::$rtname"
    members rt_cb_role "compiler_builtins-1.rcgu.o:$rtname:una_other_role" "compiler_builtins-2.rcgu.o::$rtname"
    members rt_cb_crate "other-1.rcgu.o:$rtname:$mangled" "compiler_builtins-2.rcgu.o::$rtname"
    members rt_cb_nodef "compiler_builtins-1.rcgu.o:$rtname:$mangled"
    rtcase allowed rt-cb-builtins "rt_cb_ok.a(compiler_builtins-1.rcgu.o): $rtname" "$rtentry" "" "$work/clean_una" "$work/rt_cb_ok.a" "$clean_dir"
    rtcase reject rt-cb-builtins-other-role "rt_cb_role.a(compiler_builtins-1.rcgu.o): $rtname" \
        "" "$work/clean_una" "$work/rt_cb_role.a" "$clean_dir"
    rtcase reject rt-cb-builtins-not-compiler-builtins "rt_cb_crate.a(other-1.rcgu.o): $rtname" \
        "" "$work/clean_una" "$work/rt_cb_crate.a" "$clean_dir"
    rtcase reject rt-cb-builtins-no-definition "rt_cb_nodef.a(compiler_builtins-1.rcgu.o): $rtname" \
        "" "$work/clean_una" "$work/rt_cb_nodef.a" "$clean_dir"
    # An executable that `una build` links stays strict: one that refers to either name fails.
    exe exe_rt_name "$rtname"
    exe exe_rust_name "$rname"
    rtcase reject rt-exe-strict "$rtname" "" "$work/clean_una" "$work/clean_rt.a" "$(dir_of exe_rt_name)"
    rtcase reject rt-exe-strict-rust "$rname" "" "$work/clean_una" "$work/clean_rt.a" "$(dir_of exe_rust_name)"
    # In the trace of `una`: the listed member's reference, bound to compiler_builtins, passes;
    # another member's fails; bound to the platform's libm, it fails, by `una`'s import of it.
    # (A Rust link binds the reference to compiler_builtins even with -lm, which it places
    # after compiler_builtins; the binding to libm is made here by a driver that names libm
    # first, whose shared definition then satisfies the reference.)
    # The stand-in of the crate's rlib, by the file name its member names (#2364): the member
    # `<crate>-<hash>.<...>.rcgu.o` inside `lib<crate>-<hash>.rlib`; the same member in an
    # archive of another name fails.
    rlib=lib${robject%%.*}.rlib
    members libtomls "$robject:$rname:una_libtomls_1"
    cp "$work/libtomls.a" "$work/$rlib"
    members libtomlo "other.rcgu.o:$rname:una_libtomlo_1"
    NOLM=1
    traced_exe t_rust "$rlib" "" una_libtomls_1
    traced_exe t_rust_other libtomlo ""
    traced_exe t_rust_elsewhere libtomls ""
    unset NOLM
    printf '#!/bin/sh\nexec cc -Wl,--no-as-needed -lm "$@"\n' > "$work/cc-libm-first"
    chmod +x "$work/cc-libm-first"
    UNA_LINK_TRACE_CC=$work/cc-libm-first
    export UNA_LINK_TRACE_CC
    traced_exe t_rust_libm "$rlib" "" una_libtomls_1
    unset UNA_LINK_TRACE_CC
    echo "== the trace of t_rust and t_rust_libm (#2354)"
    grep -E " $rname " "$work/t_rust.trace" "$work/t_rust_libm.trace" || true
    traced=$((traced + 1))
    allowed trace-rust-builtins "($robject) references $rname" "$rentry" \
        "$work/libz3t.a" --link-trace "$work/t_rust.trace" "$work/t_rust" "$work/clean_rt.a" "$(dir_of clean_exe)"
    treject trace-rust-builtins-other-member "(other.rcgu.o) references $rname" \
        "$work/libz3t.a" "$work/t_rust_other.trace" "$work/t_rust_other"
    treject trace-rust-builtins-other-archive "libtomls.a($robject) references $rname" \
        "$work/libz3t.a" "$work/t_rust_elsewhere.trace" "$work/t_rust_elsewhere"
    treject trace-rust-builtins-libm "$work/t_rust_libm: $rname" \
        "$work/libz3t.a" "$work/t_rust_libm.trace" "$work/t_rust_libm"
else
    traced=1
    reject trace-not-linux "a link trace is GNU ld's, read on Linux only" \
        "$work/z3.a" --link-trace "$work/z3.a" "$work/solver_una" "$work/clean_rt.a" "$(dir_of clean_exe)"
fi

# The release graph check. expect_verdict NAME EXPECT COMMAND...: the verdict of COMMAND, a
# function of scripts/no-libm-graph.sh, must be EXPECT (absent or finding).
. "$root/scripts/no-libm-graph.sh"
graph_cases=0
expect_verdict() {
    name=$1 expect=$2
    shift 2
    graph_cases=$((graph_cases + 1))
    if "$@" > "$work/graph-$name.log" 2>&1; then got=absent; else got=finding; fi
    if [ "$got" != "$expect" ]; then
        echo "error: graph canary $name is $got, not $expect:" >&2
        cat "$work/graph-$name.log" >&2
        status=1
    fi
}
# graph_case NAME EXPECT STATUS TREE ERR: the verdict on a `cargo tree -i num-bigint` that exited
# with STATUS and printed TREE and ERR.
graph_case() {
    name=$1 expect=$2 code=$3
    printf '%s' "$4" > "$work/graph-$name.tree"
    printf '%s' "$5" > "$work/graph-$name.err"
    expect_verdict "$name" "$expect" \
        no_libm_graph_verdict num-bigint "$code" "$work/graph-$name.tree" "$work/graph-$name.err"
}
nothing='warning: nothing to print.

To find dependencies that require specific target platforms, try to use option `--target all` first, and then narrow your search scope accordingly.
'
held='num-bigint v0.4.8
└── una-core v0.3.16
'
graph_case nothing-to-print absent 0 "" "$nothing"
graph_case no-match absent 101 "" \
    'error: package ID specification `num-bigint` did not match any packages
'
graph_case held finding 0 "$held" ""
graph_case held-and-warned finding 0 "$held" "$nothing"
graph_case silent finding 0 "" ""
graph_case other-warning finding 0 "" 'warning: something else
'
graph_case nothing-and-error finding 0 "" "${nothing}error: failed to load the lock file
"
graph_case nothing-but-failed finding 101 "" "$nothing"
graph_case no-match-other-crate finding 101 "" \
    'error: package ID specification `num-traits` did not match any packages
'
graph_case other-failure finding 101 "" 'error: the lock file needs to be updated
'
mkdir "$work/graph"
expect_verdict real-held finding no_libm_graph_check "$root" "$work/graph" una-core
expect_verdict real-not-in-graph absent no_libm_graph_check "$root" "$work/graph" num-bigint
expect_verdict real-not-locked absent no_libm_graph_check "$root" "$work/graph" una-no-such-crate
if ! grep -qF 'the release graph holds una-core' "$work/graph-real-held.log"; then
    echo "error: graph canary real-held fails for another reason than holding una-core:" >&2
    cat "$work/graph-real-held.log" >&2
    status=1
fi

if [ "$status" -ne 0 ]; then
    echo "no-libm-test: FAIL" >&2
    exit 1
fi
echo "no-libm-test: PASS (2 clean runs pass, 9 canaries fail as they must, $named names of the solver's list on $os each allowed in una when Z3 imports it from an object the list names and rejected in 5 scopes, 2 names in one una, $traced link-trace canaries, $graph_cases graph canaries)"
