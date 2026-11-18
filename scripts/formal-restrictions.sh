#!/bin/sh
# The re-check tells a model restriction from a rejection by the constructor of the checker's
# refusal, `Check.Err.restriction` or `Check.Err.reject`, never by its wording (#535). This check
# keeps the text and the constructor in step, both ways:
#
# 1. In the checker (Check.lean, CheckProgram.lean), every message that says "B1 restriction" or
#    "F1 restriction" is the argument of `.restriction` on its line, so no restriction is raised
#    as a rejection by an unmarked message.
# 2. In the checker and the importer, every `.restriction` is applied to a message that says
#    "B1 restriction" or "F1 restriction", so the output keeps saying why. A line that is not a
#    site (the constructor, a pattern, `Err.prefix`) says so with `restriction-check: not a site`,
#    and holds no string literal.
# 3. In the importer (Import.lean), the words appear only in comments and on lines tagged
#    `restriction-check: importer refusal` (a `throw` of the importer's reading monad: the
#    declaration is not read, so not re-checked) or `restriction-check: demoted` (a whole-program
#    reason, whose declarations are demoted to not re-checked). A tagged line inside a definition
#    whose signature names `Check.Result` or `Check.Err` fails: there a message would be a
#    rejection.
# 4. The words are never split: no "restriction (" without its "B1 "/"F1 " prefix, no "restriction"
#    right after an interpolation or a string's end, no "B1"/"F1" as a string of their own, and no
#    string continuation (a line ending in `\`) on a line holding "B1" or "F1".
#
# The limit of this check: it shows that a refusal's text and its constructor agree. It cannot tell
# a genuine rejection turned into a `.restriction` with a marker in its text, nor a restriction
# built through an alias of the constructor (a review note on #555), such as a definition that
# returns `.restriction` for a message it is given: rule 2 sees the alias's own line only. What
# catches those is the corpus and the replay, which must stay byte-identical, and the review of
# every new `.restriction` and every new tag. Of the same class (found in review): a tagged throw
# inside a lambda or a `where` definition typed `Check.Result` within a definition of the
# importer's own monad, whose signature does not name the checker's result.
#
# Usage: scripts/formal-restrictions.sh [dir]   (dir defaults to formal/UnaFormal/F1)
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "${1:-$root/formal/UnaFormal/F1}"

marker='[BF]1 restriction'
comment='^[^:]+:[0-9]+:[[:space:]]*--'
files='Check.lean CheckProgram.lean Import.lean'
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

bad=$(grep -nHE '\.restriction|\| restriction' $files | grep -vE "$comment" |
    grep -vE "\.restriction (s!)?\"$marker" | grep -v 'restriction-check: not a site' || true)
report "a \`.restriction\` whose message does not say which restriction:" "$bad"

bad=$(grep -nH 'restriction-check: not a site' $files | grep -E '"[^-]*-- restriction-check' || true)
report "a line tagged as not a site that holds a string:" "$bad"

bad=$(grep -nHE "$marker" Import.lean | grep -vE "$comment" |
    grep -vE 'restriction-check: (importer refusal|demoted)$' || true)
report "a restriction message in the importer without its tag:" "$bad"

# Rule 3's scope: the signature of the enclosing definition, from its first line to the `:=` at
# bracket depth 0 (a default argument's `:=` is inside its parentheses).
bad=$(awk '
    /^[[:space:]]*((private|partial|protected|noncomputable) )*(def|theorem|instance|abbrev|structure|inductive) / {
        sig = ""; insig = 1; depth = 0
    }
    insig {
        n = length($0)
        for (i = 1; i <= n && insig; i++) {
            ch = substr($0, i, 1)
            if (ch == "(" || ch == "[" || ch == "{") depth++
            else if (ch == ")" || ch == "]" || ch == "}") depth--
            else if (ch == ":" && substr($0, i + 1, 1) == "=" && depth == 0) {
                insig = 0
                sig = sig " " substr($0, 1, i)
                result = (sig ~ /Check\.Result|Check\.Err|Except Err/)
            }
        }
        if (insig) sig = sig " " $0
    }
    /restriction-check: (importer refusal|demoted)$/ && result { print FILENAME ":" FNR ":" $0 }
' Import.lean)
report "a tagged importer line inside a definition that returns the checker's result:" "$bad"

bad=$(grep -nHE 'restriction \(' $files | grep -vE "$comment" | grep -vE "$marker \(" |
    grep -v 'restriction-check: not a site' || true)
report "\"restriction (\" without its B1 or F1 prefix:" "$bad"

bad=$(grep -nHE '[}"][[:space:]]*restriction|"[BF]1"' $files | grep -vE "$comment" || true)
report "a restriction marker split by an interpolation or a string:" "$bad"

bad=$(grep -nHE '[BF]1.*\\$' $files | grep -vE "$comment" || true)
report "a string continuation on a line holding B1 or F1:" "$bad"

if [ "$status" -ne 0 ]; then exit 1; fi
echo "formal restrictions: PASS"
