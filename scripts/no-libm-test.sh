#!/bin/sh
# Canaries of scripts/no-libm.sh (#1277): small artifacts, compiled here with the pinned rustc,
# each of which leaves one C math library name undefined, must make the check fail with that
# name, and the clean ones must pass. They cover the scopes: a name of scripts/no-libm-una.txt
# is allowed in `una` alone and only when the Z3 library imports it, never in `una` built
# without the solver, in `libuna_rt.a` or in an executable that `una build` links, and any
# other name of scripts/no-libm.txt fails everywhere. The canaries declare the C function
# themselves (`extern "C"`), so that each artifact imports exactly the names it is meant to. The
# real artifacts are never changed.
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

# The first allowed name of the solver's list, and a denied name that it does not hold.
allowed=$(sed 's/#.*//' "$root/scripts/no-libm-una.txt" | awk 'NF { print $1; exit }')
[ -n "$allowed" ] || { echo "error: scripts/no-libm-una.txt lists no name" >&2; exit 1; }
denied=cbrt
if sed 's/#.*//' "$root/scripts/no-libm-una.txt" | grep -qxF "$denied"; then
    echo "error: the canary's denied name $denied is on scripts/no-libm-una.txt" >&2
    exit 1
fi

# rustc of the pinned toolchain, run from the repository so that rust-toolchain.toml applies.
compile() {
    (cd "$root" && rustc --edition 2021 -O "$@") > "$work/rustc.log" 2>&1 || {
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
        printf '#[cfg_attr(target_os = "linux", link(name = "m"))]\nextern "C" {\n'
        for name in "$@"; do printf '    fn %s(x: f64) -> f64;\n' "$name"; done
        echo '}'
        if [ "$kind" = lib ]; then
            printf '#[no_mangle]\npub extern "C" fn una_canary(x: f64) -> f64 {\n    let mut y = x;\n'
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
archive z3 "$allowed"           # a stand-in for the Z3 library, importing the solver's name
archive other_z3                # a Z3 library that does not import it

# run NAME UNA_Z3_LIB UNA LIBUNA_RT EXECUTABLES: runs the check, its output in $work/NAME.log,
# with `una` without the solver $rust (clean_una unless set).
run() {
    name=$1 z3=$2
    shift 2
    if [ -n "$z3" ]; then
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
echo "no-libm-test: PASS (2 clean runs pass, 8 canaries fail as they must, $graph_cases graph canaries)"
