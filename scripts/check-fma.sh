#!/bin/sh
# The FMA disassembly gate (ADR 0114 Appendix C rules 7 and 8, as ADR 0120 Appendix A amends
# them in its amendment of ADR 0106, item 3; #1216): an x86-64 artifact fails on any FMA
# instruction (`vfmadd`, `vfmsub`, `vfnmadd`, `vfnmsub`, their `...sub`/`...add` forms
# included) in a symbol whose demangled name starts with no prefix of the allowlist. The
# allowlist is the dispatched kernel variants of `una-core`, `una_core::dense::variants::ALLOWLIST`,
# which the build report records as `native.kernels.fma_allowlist` (`spec/13 §6`).
#
# An artifact for another architecture is reported as not applicable: on AArch64 the FMA
# instruction, `fmadd`, is part of the baseline and allowed everywhere (ADR 0114).
#
# Usage: scripts/check-fma.sh [--allow PREFIX]... ARTIFACT...
#   --allow PREFIX  a prefix of the allowlist; without any, the two prefixes of
#                   una_core::dense::variants::ALLOWLIST below.
# It disassembles with `objdump -d -C`, or `llvm-objdump -d -C` when there is no `objdump`;
# a disassembler that fails, or reads no instruction, fails the gate.
set -eu

usage() {
    echo "usage: scripts/check-fma.sh [--allow PREFIX]... ARTIFACT..." >&2
    exit 2
}

allowlist=""
while [ "$#" -gt 0 ]; do
    case $1 in
        --allow)
            [ "$#" -ge 2 ] || usage
            allowlist="$allowlist$2
"
            shift 2
            ;;
        --) shift; break ;;
        -*) usage ;;
        *) break ;;
    esac
done
[ "$#" -ge 1 ] || usage
if [ -z "$allowlist" ]; then
    # The same list as una_core::dense::variants::ALLOWLIST (crates/una-core/src/dense/variants.rs).
    allowlist="una_core::dense::variants::x86_64_v3
una_core::dense::variants::x86_64_v4
"
fi

if command -v objdump >/dev/null 2>&1; then
    objdump=objdump
elif command -v llvm-objdump >/dev/null 2>&1; then
    objdump=llvm-objdump
else
    echo "error: the FMA gate needs objdump or llvm-objdump (#1216)" >&2
    exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
printf '%s' "$allowlist" > "$tmp/allowlist"

status=0
for artifact in "$@"; do
    if [ ! -f "$artifact" ]; then
        echo "error: $artifact is not a file" >&2
        exit 1
    fi
    if ! "$objdump" -f "$artifact" > "$tmp/headers" 2> "$tmp/errors"; then
        echo "error: $objdump cannot read $artifact:" >&2
        cat "$tmp/errors" >&2
        exit 1
    fi
    if ! grep -q -i -E 'x86-64|x86_64' "$tmp/headers"; then
        echo "ok: FMA scan of $artifact not applicable: not x86-64 (AArch64's fmadd is baseline, ADR 0114)"
        continue
    fi
    if ! "$objdump" -d -C "$artifact" > "$tmp/disassembly" 2> "$tmp/errors"; then
        echo "error: $objdump cannot disassemble $artifact:" >&2
        cat "$tmp/errors" >&2
        exit 1
    fi
    # Each symbol's header is `<address> <demangled name>:`; its instructions follow. A v0
    # demangled name may carry crate disambiguators, `una_core[1a2b]::`, which are dropped.
    # Every FMA instruction outside an allowlisted symbol is printed as `symbol<TAB>instruction`.
    if ! awk -v allowfile="$tmp/allowlist" '
        BEGIN {
            while ((getline line < allowfile) > 0) {
                if (line != "") { allow[++count] = line }
            }
            symbol = ""
            instructions = 0
        }
        /^[0-9a-fA-F]+ <.*>:[[:space:]]*$/ {
            start = index($0, "<")
            name = substr($0, start + 1)
            sub(/>:[[:space:]]*$/, "", name)
            gsub(/\[[0-9a-f]+\]/, "", name)
            symbol = name
            allowed = 0
            for (i = 1; i <= count; i++) {
                prefix = allow[i]
                if (name == prefix || index(name, prefix "::") == 1) { allowed = 1 }
            }
            next
        }
        /^[[:space:]]*[0-9a-fA-F]+:[[:space:]]/ {
            instructions++
            if ($0 ~ /[[:space:]]v(fmadd|fmsub|fnmadd|fnmsub)[0-9a-z]*([[:space:]]|$)/ && !allowed) {
                line = $0
                sub(/^[[:space:]]+/, "", line)
                print (symbol == "" ? "(no symbol)" : symbol) "\t" line
            }
        }
        END {
            if (instructions == 0) { exit 3 }
        }
    ' "$tmp/disassembly" > "$tmp/found"; then
        echo "error: $objdump printed no instruction for $artifact; the FMA gate cannot scan it" >&2
        exit 1
    fi
    if [ -s "$tmp/found" ]; then
        echo "error: $artifact has FMA instructions outside the allowlist (ADR 0114 Appendix C rule 7, ADR 0120 Appendix A):" >&2
        head -20 "$tmp/found" >&2
        echo "the allowlist:" >&2
        sed 's/^/  /' "$tmp/allowlist" >&2
        status=1
    else
        echo "ok: $artifact has no FMA instruction outside the allowlist"
    fi
done
exit "$status"
