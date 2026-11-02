#!/bin/sh
# The scan of the musl inputs of ADR 0124 rules 12 and 24 (step 4a): the C library, start-up
# objects and libunwind.a of the pinned toolchain's self-contained musl directory, and the Rust
# standard library of that target, through the internal linker's ELF reader
# (`una_link::scan_elf`). It fails (U8001) on an R_*_IRELATIVE relocation or an indirect
# function anywhere, on a relocation type outside rule 24's list in a loaded section, on a
# narrow absolute relocation against a symbol that is not absolute, on a 64-bit absolute one in
# a section that is not writable, and on a common symbol in the standard library; it prints
# every relocation by type, section kind and referent, musl's common symbols and musl's version.
# The standard library's libprofiler_builtins, which only -C instrument-coverage and
# -C profile-generate link, is not scanned: no build of the runtime links it.
# Run it at every bump of the Rust toolchain, before the bump merges (rule 24).
#
# Usage: scripts/scan-musl.sh TARGET   (x86_64-unknown-linux-musl or aarch64-unknown-linux-musl)
set -eu
[ "$#" -eq 1 ] || { echo "usage: scripts/scan-musl.sh TARGET" >&2; exit 2; }
target=$1
case $target in
    x86_64-unknown-linux-musl | aarch64-unknown-linux-musl) ;;
    *) echo "error: $target is not a musl target of rust-toolchain.toml" >&2; exit 2 ;;
esac
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
lib="$(rustc --print sysroot)/lib/rustlib/$target/lib"
dir="$lib/self-contained"
for file in libc.a rcrt1.o crti.o crtn.o crtbeginS.o crtendS.o libunwind.a; do
    [ -f "$dir/$file" ] || { echo "error: $dir/$file is missing; rust-toolchain.toml lists $target, so rustup installs it (rustup target add $target)" >&2; exit 2; }
done
set -- "$dir/libc.a" "$dir/rcrt1.o" "$dir/crti.o" "$dir/crtn.o" "$dir/crtbeginS.o" "$dir/crtendS.o" "$dir/libunwind.a" --std
found=
for rlib in "$lib"/*.rlib; do
    case ${rlib##*/} in
        libprofiler_builtins-*) ;;
        *) set -- "$@" "$rlib"; found=1 ;;
    esac
done
[ -n "$found" ] || { echo "error: no standard library in $lib" >&2; exit 2; }
out=$(cargo run --quiet --release --locked -p una-link --example scan-elf -- "$@")
printf '%s\n' "$out"
printf '%s\n' "$out" | grep -q '^musl [0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*$' \
    || { echo "error: $dir/libc.a does not say its musl version (__libc_version)" >&2; exit 1; }
