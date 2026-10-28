#!/bin/sh
# Builds, with a release `una`, the native executables that scripts/no-libm.sh reads (#1092):
# every runnable program of tests/conformance/runtime that tests/native/unsupported.txt does not
# list, tests/conformance/runtime/no_libm_run among them. A program is chosen as the native
# differential chooses it (crates/una-cli/tests/native.rs): a `.una` file whose leading `-- `
# lines hold `-- expect: run-output` or `-- grant: `, standing for the package around it (the
# nearest directory with `una.toml` or `ita.una`, within tests/conformance) or else for itself.
#
# Each program is copied into OUT/work first, so no build or proof cache touches the
# repository, and `una build` runs there with HOME=OUT/home and without UNA_RT_LIB, so it links
# the `libuna_rt.a` beside UNA. The executables go to OUT/executables, each named after its
# program's repository path with every `/` written as `__`. OUT must not exist; it is made here.
# It fails closed: a missing UNA, an existing OUT, a program that does not build, no program at
# all, or no executable of no_libm_run is a failure.
#
# Usage: scripts/no-libm-build.sh UNA OUT
set -u

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)

if [ "$#" -ne 2 ]; then
    echo "usage: scripts/no-libm-build.sh UNA OUT" >&2
    exit 2
fi
una=$1
out=$2

fail() {
    echo "no-libm-build: FAIL ($1)" >&2
    exit 1
}

case $una in /*) ;; *) una=$(pwd)/$una ;; esac
case $out in /*) ;; *) out=$(pwd)/$out ;; esac
[ -f "$una" ] && [ -x "$una" ] || fail "no una binary at $una"
[ -e "$out" ] && fail "$out exists; the executables go to a fresh directory"
mkdir -p "$out/work" "$out/home" "$out/executables" || fail "cannot make $out"

cases=$root/tests/conformance
suite=$cases/runtime
unsupported=$root/tests/native/unsupported.txt
[ -d "$suite" ] || fail "no directory $suite"
[ -f "$unsupported" ] || fail "no list $unsupported"

# The programs, by repository path, one per line, sorted and unique.
programs=$out/programs.txt
find "$suite" -type f -name '*.una' | LC_ALL=C sort > "$out/cases.txt" || fail "cannot list $suite"
: > "$programs.unsorted"
while IFS= read -r case; do
    awk 'substr($0, 1, 3) != "-- " { exit }
        /^-- expect: run-output/ || /^-- grant: / { found = 1 }
        END { exit found ? 0 : 1 }' "$case" || continue
    program=$case
    dir=$(dirname -- "$case")
    while :; do
        if [ -f "$dir/una.toml" ] || [ -f "$dir/ita.una" ]; then
            program=$dir
            break
        fi
        [ "$dir" = "$cases" ] && break
        dir=$(dirname -- "$dir")
    done
    printf '%s\n' "${program#"$root"/}" >> "$programs.unsorted"
done < "$out/cases.txt"
LC_ALL=C sort -u "$programs.unsorted" > "$programs" || fail "cannot sort the programs"
grep -v '^#' "$unsupported" | awk 'NF { print $1 }' | LC_ALL=C sort -u > "$out/unsupported.txt"
LC_ALL=C comm -23 "$programs" "$out/unsupported.txt" > "$out/build.txt" || fail "cannot drop the unsupported programs"
[ -s "$out/build.txt" ] || fail "no runnable program in $suite"
grep -qx 'tests/conformance/runtime/no_libm_run' "$out/build.txt" \
    || fail "tests/conformance/runtime/no_libm_run is not among the programs"

failures=0
count=0
index=0
while IFS= read -r program; do
    index=$((index + 1))
    work=$out/work/$index
    mkdir -p "$work" || fail "cannot make $work"
    cp -R "$root/$program" "$work/" || fail "cannot copy $program"
    copied=$work/$(basename -- "$program")
    if [ -d "$copied" ]; then
        dir=$copied
        entry=.
    else
        dir=$work
        entry=$(basename -- "$program")
    fi
    exe=$out/executables/$(printf '%s' "$program" | sed 's|/|__|g')
    if (cd "$dir" && env -u UNA_RT_LIB HOME="$out/home" NO_COLOR=1 "$una" build "$entry" --out "$exe") \
        > "$work/build.log" 2>&1 && [ -f "$exe" ]; then
        count=$((count + 1))
    else
        echo "no-libm-build: $program did not build:" >&2
        cat "$work/build.log" >&2
        failures=$((failures + 1))
    fi
done < "$out/build.txt"

[ "$failures" -eq 0 ] || fail "$failures programs did not build"
[ -f "$out/executables/tests__conformance__runtime__no_libm_run" ] || fail "no executable of no_libm_run"
echo "no-libm-build: built $count executables with $una in $out/executables"
