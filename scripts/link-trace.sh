#!/bin/sh
# The linker that scripts/release.sh gives rustc on Linux (`-C linker`, #1953): it runs the C
# compiler driver (`cc`, or UNA_LINK_TRACE_CC) with rustc's arguments, unchanged, and asks GNU ld
# to trace every name of scripts/no-libm.txt (`-Wl,--trace-symbol=NAME`), which changes no byte
# of the output. ld writes, on its standard error, one line for each input file of the link that
# refers to or defines a traced name; a linked archive member is named `archive(member)`. Those
# lines go to a new file in UNA_LINK_TRACE_DIR, the link trace, which scripts/no-libm.sh
# --link-trace reads: its first line is `output: PATH`, the file the link wrote (rustc's `-o`,
# made absolute), its second `traced: NAME...`, the names traced, then one line per traced
# input, `reference NAME FILE` or `definition NAME FILE`. Every other line of ld's standard
# error is passed on to rustc's, so that its warnings stay what they were. The exit status is
# the driver's.
#
# It fails closed: no UNA_LINK_TRACE_DIR, or no name to trace, stops the link (exit 2).
set -u

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
dir=${UNA_LINK_TRACE_DIR:-}
if [ -z "$dir" ] || [ ! -d "$dir" ]; then
    echo "link-trace: UNA_LINK_TRACE_DIR names no directory for the link trace" >&2
    exit 2
fi
names=$(sed 's/#.*//' "$root/scripts/no-libm.txt" 2> /dev/null | awk 'NF { print $1 }')
if [ -z "$names" ]; then
    echo "link-trace: no name to trace in $root/scripts/no-libm.txt" >&2
    exit 2
fi

# The output file: the argument after `-o`, given directly or in a response file (`@FILE`, one
# argument per line, which rustc writes when the command line is too long).
out=""
prev=""
for arg in "$@"; do
    if [ "$prev" = -o ]; then out=$arg; fi
    case $arg in
    @*)
        if [ -z "$out" ] && [ -f "${arg#@}" ]; then
            out=$(awk 'prev == "-o" { print; exit } { prev = $0 }' "${arg#@}")
        fi
        ;;
    esac
    prev=$arg
done
case $out in
'' | /*) ;;
*) out=$(pwd)/$out ;;
esac

trace=$(mktemp "$dir/link.XXXXXX") || {
    echo "link-trace: cannot make a file in $dir" >&2
    exit 2
}
for name in $names; do
    set -- "$@" "-Wl,--trace-symbol=$name"
done
"${UNA_LINK_TRACE_CC:-cc}" "$@" 2> "$trace.err"
status=$?
{
    printf 'output: %s\n' "$out"
    printf 'traced: %s\n' "$(printf '%s\n' "$names" | tr '\n' ' ' | sed 's/ $//')"
    sed -n -e 's/^[^ :]*: \(.*\): reference to \([^ ]*\)$/reference \2 \1/p' \
        -e 's/^[^ :]*: \(.*\): definition of \([^ ]*\)$/definition \2 \1/p' "$trace.err"
} > "$trace"
grep -v -e '^[^ :]*: .*: reference to [^ ]*$' -e '^[^ :]*: .*: definition of [^ ]*$' "$trace.err" >&2
rm -f "$trace.err"
exit "$status"
