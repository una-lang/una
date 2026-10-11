#!/bin/sh
# Mutation tests of scripts/formal-restrictions.sh (#535): each mutant appends one forbidden line
# to a copy of the checker's or the importer's source, and the check must fail with the rule's
# message; the unmutated copy must pass. The sources are never changed.
#
# Usage: scripts/formal-restrictions-test.sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
src="$root/formal/UnaFormal/F1"
check="$root/scripts/formal-restrictions.sh"
work=$(mktemp -d)
trap 'rm -rf "${work:?}"' EXIT
status=0

fresh() {
    rm -rf "${work:?}/m"
    mkdir "$work/m"
    cp "$src/Check.lean" "$src/CheckProgram.lean" "$src/Import.lean" "$work/m/"
}

# expect <name> <file> <expected message> <line>...: the lines appended to <file> must fail.
expect() {
    name=$1 file=$2 message=$3
    shift 3
    fresh
    for line in "$@"; do printf '%s\n' "$line" >> "$work/m/$file"; done
    if out=$("$check" "$work/m" 2>&1); then
        echo "error: mutant $name passes the check" >&2
        status=1
    elif ! printf '%s\n' "$out" | grep -qF "$message"; then
        echo "error: mutant $name fails for another reason:" >&2
        printf '%s\n' "$out" >&2
        status=1
    fi
}

fresh
"$check" "$work/m" > /dev/null || { echo "error: the unmutated sources fail the check" >&2; exit 1; }

expect unmarked Check.lean 'outside a `.restriction` argument' \
    'def mutant : Result Unit := need false "F1 restriction (x)"'
expect unnamed Check.lean 'does not say which restriction' \
    'def mutant : Result Unit := need false (.restriction "x")'
expect not-a-site Check.lean 'tagged as not a site that holds a string' \
    'def mutant : Err := .restriction "x" -- restriction-check: not a site'
expect untagged Import.lean 'in the importer without its tag' \
    'def mutant : M Unit := throw s!"F1 restriction (x)"'
expect check-result Import.lean 'inside a definition that returns the checker' \
    'def mutant (b : Bool) : Check.Result Unit := do' \
    '  throw s!"F1 restriction (x)" -- restriction-check: importer refusal'
expect interpolation Check.lean 'split by an interpolation or a string' \
    'def mutant : Result Unit := need false s!"{"F1"} restriction (x)"'
expect concatenation Check.lean 'without its B1 or F1 prefix' \
    'def mutant : Result Unit := need false ("F" ++ "1 restriction (x)")'
expect continuation Check.lean 'string continuation on a line holding B1 or F1' \
    'def mutant : Result Unit := need false (.restriction s!"F1 \' \
    '  restriction (x)")'

# The real case (found in the recheck of #555): a tagged throw inside `checkWith` itself, whose
# signature has a default argument's `:=` before its result type.
fresh
awk '{ print } /^  Check\.checkFun p fn$/ && !done {
    print "  throw s!\"F1 restriction (x)\" -- restriction-check: importer refusal"; done = 1 }' \
    "$src/Import.lean" > "$work/m/Import.lean"
if out=$("$check" "$work/m" 2>&1); then
    echo "error: mutant checkWith passes the check" >&2
    status=1
elif ! printf '%s\n' "$out" | grep -qF 'inside a definition that returns the checker'; then
    echo "error: mutant checkWith fails for another reason:" >&2
    printf '%s\n' "$out" >&2
    status=1
fi

if [ "$status" -ne 0 ]; then exit 1; fi
echo "formal restrictions, mutants: PASS (9 mutants)"
