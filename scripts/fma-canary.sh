#!/bin/sh
# The x86-64 contraction canary (#1228; ADR 0114 Appendix C rule 7, as ADR 0120 Appendix A amends
# it in its amendment of ADR 0106, item 3, kept by ADR 0106's amendment of 2026-10-31, §2): it
# proves that the FMA disassembly gate, scripts/check-fma.sh (#1216), detects a contraction.
#
# It builds the contraction-sensitive case of the float corpus, the example
# crates/una-core/examples/contraction-canary.rs (`a * b + c` with a = 1 + 2^-52, b = 1 - 2^-52,
# c = -1, and the same triple through una-core's scalar `fma` of x86-64 without FMA3), twice, for
# this host's target, in the release profile:
#
#   1. with the shipped flags: the gate MUST pass on the executable, and the executable MUST print
#      the corpus's bits, two roundings for `a * b + c` (0.0) and the fused result for `fma`;
#   2. with contraction forced on, `-C target-feature=+fma -C llvm-args=-fp-contract=fast` added to
#      RUSTFLAGS for the whole build (una-core included), Rust's equivalent of the `-mfma
#      -ffp-contract=fast` of the rule: the gate MUST fail, and must name the case's function.
#      Where the processor has FMA, the executable then MUST print the fused bits for
#      `a * b + c`: the corpus fails on its contraction-sensitive case. Without FMA the run is
#      skipped, and said so.
#
# Rust contracts no `a * b + c` with FMA enabled alone (`+fma` without `-fp-contract=fast` emits
# `vmulsd` and `vaddsd`), so the canary forces both, as the rule's C flags did. The shipped
# artifacts are never changed: build 1 uses the caller's target directory (CARGO_TARGET_DIR, else
# target/), build 2 its own, <that>/fma-canary.
#
# On a host whose target is not x86-64 it prints that the canary is not applicable and exits 0,
# as scripts/check-fma.sh does for an artifact that is not x86-64: AArch64's `fmadd` is part of
# the baseline and allowed (ADR 0114). The x86-64 leg is scripts/local-ci.sh --linux.
#
# Usage: scripts/fma-canary.sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

# The host's target, from the pinned rustc; a rustc that fails or names none fails the canary,
# never makes it not applicable.
host=$(rustc -vV 2>"${TMPDIR:-/tmp}/fma-canary-rustc.$$" | sed -n 's/^host: //p') || host=
if [ -z "$host" ]; then
    cat "${TMPDIR:-/tmp}/fma-canary-rustc.$$" >&2
    rm -f "${TMPDIR:-/tmp}/fma-canary-rustc.$$"
    echo "error: rustc -vV names no host target; the contraction canary cannot tell whether it applies" >&2
    exit 1
fi
rm -f "${TMPDIR:-/tmp}/fma-canary-rustc.$$"
case $host in
    x86_64-*) ;;
    *)
        echo "ok: the contraction canary is not applicable on $host: not x86-64 (AArch64's fmadd is baseline, ADR 0114)"
        exit 0
        ;;
esac
# CARGO_ENCODED_RUSTFLAGS takes precedence over RUSTFLAGS, so with it set the forced flags would
# not reach the build, and the canary would test nothing.
if [ -n "${CARGO_ENCODED_RUSTFLAGS:-}" ]; then
    echo "error: CARGO_ENCODED_RUSTFLAGS is set; the contraction canary adds its flags through RUSTFLAGS, which it would override" >&2
    exit 2
fi

base=${CARGO_TARGET_DIR:-$root/target}
forced_dir=$base/fma-canary
forced_flags="-C target-feature=+fma -C llvm-args=-fp-contract=fast"
case_symbol=contraction_canary::contraction_case
two_roundings=0x0000000000000000
fused=0xb970000000000000
work=$(mktemp -d "${TMPDIR:-/tmp}/fma-canary.XXXXXX")
trap 'rm -rf "${work:?}"' EXIT

# build DIR [RUSTFLAGS]: the example in the release profile, into the target directory DIR.
build() {
    if ! env CARGO_TARGET_DIR="$1" RUSTFLAGS="$2" \
        cargo build --quiet --release --locked -p una-core --example contraction-canary \
        > "$work/build.log" 2>&1; then
        cat "$work/build.log" >&2
        echo "error: the contraction canary's build into $1 failed" >&2
        exit 1
    fi
}

# has_fma: whether this processor runs FMA instructions (Linux: /proc/cpuinfo; macOS: sysctl).
has_fma() {
    if [ -r /proc/cpuinfo ]; then
        grep -q -w fma /proc/cpuinfo
    else
        sysctl -n machdep.cpu.features 2>/dev/null | grep -q -w FMA
    fi
}

# 1. The shipped flags: the gate passes, and the bits are the corpus's.
build "$base" "${RUSTFLAGS:-}"
shipped=$base/release/examples/contraction-canary
if ! scripts/check-fma.sh "$shipped" > "$work/shipped.scan" 2>&1; then
    cat "$work/shipped.scan" >&2
    echo "error: the FMA gate fails on the contraction canary built with the shipped flags (ADR 0114 Appendix C rule 8)" >&2
    exit 1
fi
cat "$work/shipped.scan"
"$shipped" > "$work/shipped.out"
if [ "$(cat "$work/shipped.out")" != "case $two_roundings
emulated $fused" ]; then
    echo "error: the contraction canary built with the shipped flags printed, instead of case $two_roundings and emulated $fused (ADR 0114 rule 6.2):" >&2
    cat "$work/shipped.out" >&2
    exit 1
fi
echo "ok: built with the shipped flags, a * b + c is two roundings ($two_roundings) and fma is fused ($fused)"

# 2. Contraction forced on: the gate fails on the case, and the corpus's bits change.
build "$forced_dir" "${RUSTFLAGS:+$RUSTFLAGS }$forced_flags"
forced=$forced_dir/release/examples/contraction-canary
status=0
scripts/check-fma.sh "$forced" > "$work/forced.scan" 2>&1 || status=$?
if [ "$status" -ne 1 ]; then
    cat "$work/forced.scan" >&2
    echo "error: the FMA gate did not fail (exit $status) on the contraction canary built with $forced_flags: it cannot detect a contraction (ADR 0114 Appendix C rule 7)" >&2
    exit 1
fi
if ! grep -q -F "$case_symbol	" "$work/forced.scan"; then
    cat "$work/forced.scan" >&2
    echo "error: the FMA gate failed on the canary built with $forced_flags, but not on $case_symbol, the contraction-sensitive case" >&2
    exit 1
fi
echo "ok: built with $forced_flags, the FMA gate fails on $case_symbol, as it must:"
sed 's/^/  /' "$work/forced.scan"
if has_fma; then
    "$forced" > "$work/forced.out"
    if [ "$(sed -n 's/^case //p' "$work/forced.out")" != "$fused" ]; then
        echo "error: the contraction canary built with $forced_flags printed, instead of case $fused, the fused result:" >&2
        cat "$work/forced.out" >&2
        exit 1
    fi
    echo "ok: built with $forced_flags, a * b + c is fused ($fused): the corpus fails on its contraction-sensitive case ($(sed -n 's/^emulated /emulated /p' "$work/forced.out"))"
else
    echo "skipped: the run of the forced build, since this processor has no FMA; the gate's verdict above stands"
fi
echo "ok: the contraction canary passes"
