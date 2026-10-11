#!/bin/sh
# The re-check tells a model restriction from a rejection by the constructor of the checker's
# refusal, `Check.Err.restriction` or `Check.Err.reject`, never by its wording (#535). This check
# keeps the two in step, both ways:
#
# 1. In the checker (Check.lean, CheckProgram.lean), every message that says "B1 restriction" or
#    "F1 restriction" is the argument of `.restriction` on its line, so no restriction is raised
#    as a rejection by an unmarked message (which `Coe String Err` reads as `.reject`).
# 2. In the checker and the importer, every `.restriction` is applied to a message that says
#    "B1 restriction" or "F1 restriction", so the output keeps saying why. A line that is not a
#    site (a pattern, `Err.prefix`) says so with `restriction-check: not a site`.
# 3. In the importer (Import.lean), the words appear only in comments, in the importer's own
#    refusals (`throw` in its reading monad, a declaration not read, which is not re-checked), and
#    in the whole-program reason `docWhy`, whose declarations are demoted to not re-checked.
#
# Usage: scripts/formal-restrictions.sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root/formal/UnaFormal/F1"

marker='[BF]1 restriction'
comment='^[^:]+:[0-9]+:[[:space:]]*--'
status=0

report() {
    if [ -n "$2" ]; then
        echo "error: $1" >&2
        printf '%s\n' "$2" >&2
        status=1
    fi
}

bad=$(grep -nHE "$marker" Check.lean CheckProgram.lean | grep -vE "$comment" |
    grep -vE "\.restriction (s!)?\"$marker" || true)
report "a restriction message outside a \`.restriction\` argument:" "$bad"

bad=$(grep -nHE '\.restriction' Check.lean CheckProgram.lean Import.lean | grep -vE "$comment" |
    grep -v 'restriction-check: not a site' | grep -vE "\.restriction (s!)?\"$marker" || true)
report "a \`.restriction\` whose message does not say which restriction:" "$bad"

bad=$(grep -nHE "$marker" Import.lean | grep -vE "$comment" |
    grep -vE "throw s!\"$marker" | grep -vE "then \"B1 restriction\"$" || true)
report "a restriction message in the importer outside its own refusals and \`docWhy\`:" "$bad"

if [ "$status" -ne 0 ]; then exit 1; fi
echo "formal restrictions: PASS"
