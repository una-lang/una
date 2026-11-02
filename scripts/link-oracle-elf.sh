#!/bin/sh
# The per-type oracle of the ELF relocation engine against GNU `ld` (ADR 0124 rule 33, step 4b).
# For each musl target, the host's `clang` assembles the fixture
# crates/una-link/tests/elf-oracle/<arch>.s, with GNU_LD defined (which leaves out the few cases
# GNU ld 2.42 refuses or does not know: a narrow absolute relocation against an absolute symbol,
# which it treats as moving with the executable; MOVW_UABS on arm64; LDST128_TPREL_LO12_NC,
# which its assembler and linker do not know; `ld.lld` is their oracle), and absolute.s; GNU
# `ld` links them into a static-pie executable in the bare local-ci container of the
# architecture (linux/arm64 runs natively on the gate machine, linux/amd64 under emulation,
# which does not change what a linker writes; no network, a read-only root, all capabilities
# dropped, and the run's own output directory, which holds the objects and receives the
# executable, the only mount); then, on the host, `una_link::elf_oracle` runs the engine on the fixture's
# sections with the addresses `ld` chose and compares the bytes at every place
# (examples/elf-oracle.rs): equal, or one of the known choices of the same meaning, which it
# lists (EQUIVALENT). tests/elf_oracle.rs does the same against `ld.lld` in every `cargo test`
# on macOS, where every place must be equal. Release-time evidence (docs/release-gates.md); the
# `link-oracle` gate of step 4f runs it. macOS only (Apple's `clang` assembles ELF for both
# architectures).
#
# The images are the local-ci ones (scripts/local-ci.sh builds them); Docker is reached through
# the caller's environment (DOCKER_CONTEXT=desktop-linux on a Mac).
#
# Usage: scripts/link-oracle-elf.sh [OUT]   (OUT: a directory for the objects, default a temporary one)
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
fail() {
    echo "error: $*" >&2
    exit 1
}
toolchain=$(sed -n 's/^channel = "\(.*\)"$/\1/p' rust-toolchain.toml)
[ -n "$toolchain" ] || fail "rust-toolchain.toml names no channel"
out=${1:-$(mktemp -d "${TMPDIR:-/tmp}/una-link-oracle.XXXXXX")}
mkdir -p "$out"
fixtures=$root/crates/una-link/tests/elf-oracle
command -v clang > /dev/null || fail "clang is missing (Apple's assembles ELF for both architectures)"
status=0
for arch in aarch64 x86_64; do
    case $arch in
        aarch64) platform=linux/arm64 image="una-local-ci:$toolchain-arm64" ;;
        x86_64) platform=linux/amd64 image="una-local-ci:$toolchain" ;;
    esac
    docker image inspect "$image" > /dev/null 2>&1 \
        || fail "the image $image is missing: scripts/local-ci.sh builds it"
    mkdir -p "$out/$arch"
    echo "== $arch: clang assembles the fixture"
    clang -target "$arch-unknown-linux-musl" -Wa,--defsym,GNU_LD=1 -c "$fixtures/$arch.s" \
        -o "$out/$arch/fixture.o" || fail "$arch: clang cannot assemble the fixture"
    clang -target "$arch-unknown-linux-musl" -c "$fixtures/absolute.s" \
        -o "$out/$arch/absolute.o" || fail "$arch: clang cannot assemble absolute.s"
    echo "== $arch: GNU ld in $image ($platform)"
    docker run --rm --platform "$platform" \
        --network none \
        --read-only \
        --cap-drop ALL \
        --security-opt no-new-privileges \
        --user "$(id -u):$(id -g)" \
        --volume "$out/$arch:/out" \
        "$image" sh -c '
            set -e
            ld --version | sed -n 1p
            ld -static -pie --no-dynamic-linker -z text -nostdlib -e _start \
                -o /out/fixture.bfd /out/fixture.o /out/absolute.o' \
        || fail "$arch: GNU ld failed"
    echo "== $arch: the engine against GNU ld"
    cargo run --quiet --locked -p una-link --example elf-oracle -- \
        "$out/$arch/fixture.o" "$out/$arch/fixture.bfd" || status=1
done
[ "$status" -eq 0 ] || fail "the engine and GNU ld differ (MISMATCH lines above)"
echo "link-oracle-elf: PASS"
