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
# With --toolchain-fma (ADR 0120's amendment of 2026-11-27, accepted by the Owner; #2106), four
# symbols of the pinned toolchain's `compiler_builtins` are allowed too, by their exact
# demangled names and never as a prefix: the hardware paths of the IEEE `fma` and `fmaf`
# (TOOLCHAIN_FMA below), which Z3's `hwf.cpp` reaches in `una` and which `libuna_rt.a` carries
# with the whole of `compiler_builtins`. It is refused for any artifact but a file named `una`
# or `libuna_rt.a`, so an executable that `una build` writes keeps the allowlist alone.
#
# Usage: scripts/check-fma.sh [--allow PREFIX]... [--toolchain-fma] ARTIFACT...
#   --allow PREFIX   a prefix of the allowlist; without any, the two prefixes of
#                    una_core::dense::variants::ALLOWLIST below.
#   --toolchain-fma  also the four exact names of TOOLCHAIN_FMA, for `una` and `libuna_rt.a`.
# It disassembles with `objdump -d -C`, or `llvm-objdump -d -C` when there is no `objdump`, or
# the `objdump` of the Xcode toolchain (`xcrun --find objdump`); a disassembler that fails, or
# reads no instruction, fails the gate. The architecture comes from `objdump -f`; for a Mach-O
# file or an archive of Mach-O objects, which the `objdump -f` of LLVM refuses (#1356), from
# `otool -hv` (`otool`, else `xcrun otool`), which names the CPU type of each object. A file
# that neither names, and a universal (fat) Mach-O file, fail the gate.
set -eu

usage() {
    echo "usage: scripts/check-fma.sh [--allow PREFIX]... ARTIFACT..." >&2
    exit 2
}

allowlist=""
toolchain_fma=0
while [ "$#" -gt 0 ]; do
    case $1 in
        --toolchain-fma) toolchain_fma=1; shift ;;
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
# The exact names of ADR 0120's amendment of 2026-11-27: nothing else, no prefix.
TOOLCHAIN_FMA="compiler_builtins::math::libm_math::arch::x86::fma::fma_with_fma
compiler_builtins::math::libm_math::arch::x86::fma::fma_with_fma4
compiler_builtins::math::libm_math::arch::x86::fma::fmaf_with_fma
compiler_builtins::math::libm_math::arch::x86::fma::fmaf_with_fma4
"
exact=""
if [ "$toolchain_fma" = 1 ]; then
    for artifact in "$@"; do
        case ${artifact##*/} in
            una | libuna_rt.a) ;;
            *)
                echo "error: --toolchain-fma is for una and libuna_rt.a only, not $artifact (ADR 0120, amendment of 2026-11-27)" >&2
                exit 2
                ;;
        esac
    done
    exact=$TOOLCHAIN_FMA
fi

if command -v objdump >/dev/null 2>&1; then
    objdump=objdump
elif command -v llvm-objdump >/dev/null 2>&1; then
    objdump=llvm-objdump
elif command -v xcrun >/dev/null 2>&1 && xcrun --find objdump >/dev/null 2>&1; then
    objdump=$(xcrun --find objdump)
else
    echo "error: the FMA gate needs objdump or llvm-objdump (#1216)" >&2
    exit 1
fi
# The reader of Mach-O headers, when there is one (macOS): `otool`, else the Xcode toolchain's.
otool=
if command -v otool >/dev/null 2>&1; then
    otool=otool
elif command -v xcrun >/dev/null 2>&1 && xcrun --find otool >/dev/null 2>&1; then
    otool=$(xcrun --find otool)
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
printf '%s' "$allowlist" > "$tmp/allowlist"
printf '%s' "$exact" > "$tmp/exact"

status=0
for artifact in "$@"; do
    if [ ! -f "$artifact" ]; then
        echo "error: $artifact is not a file" >&2
        exit 1
    fi
    # The architecture: from `objdump -f` (ELF, and every format GNU objdump reads), else, for
    # Mach-O, from the CPU type that `otool -hv` prints for the file or each archive member.
    # An `objdump -f` that names no architecture, as LLVM's for a universal file, counts as one
    # that cannot read the file.
    if "$objdump" -f "$artifact" > "$tmp/headers" 2> "$tmp/errors" && grep -q 'architecture:' "$tmp/headers"; then
        if ! grep -q -i -E 'x86-64|x86_64' "$tmp/headers"; then
            echo "ok: FMA scan of $artifact not applicable: not x86-64 (AArch64's fmadd is baseline, ADR 0114)"
            continue
        fi
    elif [ -n "$otool" ] && "$otool" -hv "$artifact" > "$tmp/headers" 2> "$tmp/errors.otool"; then
        if grep -q -E '^Fat headers' "$tmp/headers" || "$otool" -f "$artifact" 2>/dev/null | grep -q -E '^Fat headers'; then
            echo "error: $artifact is a universal (fat) Mach-O file; scan each architecture's file (lipo -thin)" >&2
            exit 1
        fi
        # The CPU type is the second column of each header line, after the magic number.
        awk '$1 ~ /^MH_(MAGIC|CIGAM)(_64)?$/ { print $2 }' "$tmp/headers" | LC_ALL=C sort -u > "$tmp/cpus"
        if [ ! -s "$tmp/cpus" ]; then
            echo "error: $otool names no Mach-O header in $artifact; the FMA gate cannot tell its architecture" >&2
            exit 1
        fi
        if ! grep -q -x 'X86_64' "$tmp/cpus"; then
            if grep -q -v -x -E 'ARM64|ARM64E|ARM64_32' "$tmp/cpus"; then
                echo "error: $artifact holds Mach-O objects for $(tr '\n' ' ' < "$tmp/cpus")which the FMA gate does not know" >&2
                exit 1
            fi
            echo "ok: FMA scan of $artifact not applicable: not x86-64 (AArch64's fmadd is baseline, ADR 0114)"
            continue
        fi
    else
        echo "error: $objdump cannot read $artifact:" >&2
        cat "$tmp/errors" >&2
        [ -n "$otool" ] && cat "$tmp/errors.otool" >&2
        exit 1
    fi
    if ! "$objdump" -d -C "$artifact" > "$tmp/disassembly" 2> "$tmp/errors"; then
        echo "error: $objdump cannot disassemble $artifact:" >&2
        cat "$tmp/errors" >&2
        exit 1
    fi
    # Each symbol's header is `<address> <demangled name>:`; its instructions follow. A v0
    # demangled name may carry crate disambiguators, `una_core[1a2b]::`, which are dropped.
    # Every FMA instruction outside an allowlisted symbol is printed as `symbol<TAB>instruction`.
    if ! awk -v allowfile="$tmp/allowlist" -v exactfile="$tmp/exact" '
        BEGIN {
            while ((getline line < allowfile) > 0) {
                if (line != "") { allow[++count] = line }
            }
            while ((getline line < exactfile) > 0) {
                if (line != "") { exact[line] = 1 }
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
            if (name in exact) { allowed = 1 }
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
        [ -s "$tmp/exact" ] && sed 's/^/  (exact) /' "$tmp/exact" >&2
        status=1
    else
        echo "ok: $artifact has no FMA instruction outside the allowlist"
    fi
done
exit "$status"
