#!/bin/sh
# Builds the native runtime library and then `una`, with that runtime built into it, in release
# mode (ADR 0124 rules 12, 14 and 18, step 2), and packs `una` with the license and the readme
# into target/release-archives/una-<version>-<target>.tar.gz, with `una`, `LICENSE`, `README.md`
# and the four guides under `docs/guide/` (#410, #430) and no `lib/` (ADR 0059's amendment of
# ADR 0124), and its SHA-256 digest beside it in a .sha256 file (ADR 0035 rule 3). The runtime
# that `una` embeds stays at target/release-remapped/embed/libuna_rt.a, where
# scripts/check-release.sh scans it. Each target builds natively on its own
# runner, so TARGET must be the host's. On Linux, `una` also embeds the musl inputs of its own
# architecture from the pinned toolchain (ADR 0124 step 4a). Two builds of one commit give the same archive (#679):
# its entries take the commit's time, SOURCE_DATE_EPOCH when set, else `git log -1 %ct`.
#
# Usage: scripts/release.sh TARGET
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: scripts/release.sh TARGET" >&2
    exit 2
fi
target=$1
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
version=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([^"]*\)"$/\1/p' Cargo.toml)
if [ -z "$version" ]; then
    echo "error: Cargo.toml has no [workspace.package] version" >&2
    exit 2
fi
# Only a shipped target is packed (scripts/release-targets.txt, #1903); an unreadable or empty
# list refuses every target.
. scripts/release-targets.sh
shipped=$(release_targets scripts/release-targets.txt) || exit 2
if ! printf '%s\n' "$shipped" | grep -qxF -- "$target"; then
    echo "error: $target is not a shipped target; scripts/release-targets.txt lists the targets a release packs" >&2
    exit 2
fi
host=$(rustc -vV | sed -n 's/^host: //p')
if [ "$target" != "$host" ]; then
    echo "error: this machine builds $host, not $target; build each target on its own runner" >&2
    exit 2
fi

# The released `una` and runtime carry no path of the machine that builds them (#382, S-13):
# panic messages and debug information name source files, so the source root, the cargo home
# (the registry's sources) and the toolchain's sysroot are written as `/una`, `/cargo` and
# `/rust`, for Rust and for C. The flags change every artifact, so the release builds in its own target directory
# and leaves the one of the tests as it is.
cargo_home=${CARGO_HOME:-$HOME/.cargo}
cargo_home=$(CDPATH='' cd -- "$cargo_home" && pwd)
sysroot=$(rustc --print sysroot)
case "$root$cargo_home$sysroot" in
*" "*)
    echo "error: a path to remap has a space, which RUSTFLAGS cannot hold: $root, $cargo_home, $sysroot" >&2
    exit 2
    ;;
esac
# The process key of keyed hashing comes from the operating system's random source through
# `getrandom` (ADR 0119 rule 9). That crate's source can be switched at compile time with
# `--cfg getrandom_backend=...`, to a custom or an unsupported one, so a release refuses any
# inherited flag that sets it, in the environment or in a cargo configuration file.
for flags in "${RUSTFLAGS:-}" "${CARGO_ENCODED_RUSTFLAGS:-}" "${CARGO_BUILD_RUSTFLAGS:-}"; do
    case $flags in
    *getrandom_backend*)
        echo "error: the build flags set getrandom_backend, which would change the random source that getrandom gives the shipped binaries (ADR 0119)" >&2
        exit 2
        ;;
    esac
done
for config in "$root/.cargo/config.toml" "$root/.cargo/config" "$cargo_home/config.toml" "$cargo_home/config"; do
    if [ -f "$config" ] && grep -q getrandom_backend "$config"; then
        echo "error: $config sets getrandom_backend, which would change the random source that getrandom gives the shipped binaries (ADR 0119)" >&2
        exit 2
    fi
done
RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$root=/una --remap-path-prefix=$cargo_home=/cargo --remap-path-prefix=$sysroot=/rust"
# The Z3 built into the release una (ADR 0124 rules 1 and 18): the library whose digest
# supply-chain/z3.lock records for this target, taken from the cache by that digest, never
# rebuilt here; una-cli's build checks the digest again. On Linux, libstdc++ is linked
# statically into una by una-z3's build script (`static:-bundle=stdc++`: the driver flag
# -static-libstdc++ below does not reach an explicit -lstdc++, #1956), and GCC's libgcc.a by
# -static-libgcc, so the C++ runtime adds no shared library (rule 1); the unwinder
# libgcc_s.so.1, which Rust's standard library links, stays (scripts/load-commands.txt).
z3_digest=$(sed -n '/^\[library\]$/,$s/^"'"$target"'" *= *"\([0-9a-f]*\)"$/\1/p' supply-chain/z3.lock | head -n 1)
if [ -z "$z3_digest" ]; then
    echo "error: supply-chain/z3.lock records no library digest for $target; build it with scripts/build-z3.sh --reproducible and record its digest in the [library] table" >&2
    exit 2
fi
z3_cache=${UNA_Z3_CACHE:-$HOME/.cache/una/z3}
z3_lib=
for recorded in "$z3_cache"/*/library.sha256; do
    [ -f "$recorded" ] || continue
    if [ "$(cat "$recorded")" = "$z3_digest" ]; then
        z3_lib=$(dirname "$recorded")/lib/libz3shell.a
        break
    fi
done
[ -n "$z3_lib" ] && [ -f "$z3_lib" ] \
    || { echo "error: no entry of $z3_cache holds the Z3 library $z3_digest that the lock records for $target; build it with scripts/build-z3.sh --reproducible" >&2; exit 2; }
UNA_EMBED_Z3=$z3_lib
UNA_EMBED_Z3_SHA256=$z3_digest
export UNA_EMBED_Z3 UNA_EMBED_Z3_SHA256
# A development library that the caller's environment names (UNA_Z3_LIB, which
# scripts/local-ci.sh and scripts/z3-library.sh --export set) never reaches the release build:
# una-z3's build refuses both variables at once, and the release embeds only the library above,
# by its digest (#1954).
unset UNA_Z3_LIB
# On Linux every link also goes through scripts/link-trace.sh, which runs `cc` with rustc's
# arguments and GNU ld's --trace-symbol for each name of scripts/no-libm.txt, which changes no
# byte of the output, and keeps ld's record of which object refers to each of them: the trace of
# the `una` link is kept as $built/una.link-trace, which scripts/check-release.sh gives
# scripts/no-libm.sh --link-trace, so that the C math library check reads the shipped `una`,
# libstdc++.a's objects inside it included, object by object (#1953).
built="$root/target/release-remapped"
case $target in
    *-linux-*)
        RUSTFLAGS="$RUSTFLAGS -C link-arg=-static-libstdc++ -C link-arg=-static-libgcc -C linker=$root/scripts/link-trace.sh"
        # Since Rust 1.90, rustc links x86_64-unknown-linux-gnu with its own rust-lld by default,
        # whose --trace-symbol lines go to stdout in another shape. The release links with the
        # image's GNU ld instead (gcc-toolset-13-binutils, whose version supply-chain/z3.lock
        # records), as on aarch64, so that the trace is GNU ld's (#2340).
        case $target in
            x86_64-unknown-linux-gnu) RUSTFLAGS="$RUSTFLAGS -C linker-features=-lld" ;;
        esac
        UNA_LINK_TRACE_DIR="$built/link-traces"
        rm -rf "${UNA_LINK_TRACE_DIR:?}" "${built:?}/una.link-trace"
        mkdir -p "$UNA_LINK_TRACE_DIR"
        export UNA_LINK_TRACE_DIR
        ;;
esac
export RUSTFLAGS
# The C and assembly sources of dependencies (ring's), which the `cc` crate compiles, the same.
CFLAGS="${CFLAGS:-} -ffile-prefix-map=$root=/una -ffile-prefix-map=$cargo_home=/cargo"
export CFLAGS
# The build order of ADR 0124 rule 18: the runtime first, from this commit, then `una`, which
# embeds it. The runtime loses its debug sections (rule 12), so that no path or line table of
# this machine reaches an executable and the embedded bytes are smaller: the Xcode toolchain's
# `strip -S` on macOS, by `xcrun`, so that no other `strip` on PATH is taken, with
# ZERO_AR_DATE=1 so that the archive's member headers carry no time, and GNU strip's
# `--strip-debug` with deterministic archive headers (`-D`) on Linux. The tool is printed on a
# line `strip: <target>: <tool>`, its version and the SHA-256 of its binary, which the release
# gates copy into the validation record and its summary (#1359); scripts/check-release.sh
# checks that no debug section is left. una-cli's build script checks the stripped file
# against the digest given here and embeds it; a built `una` never reads these variables.
cargo build --release --locked --target-dir "$built" --package una-rt
embed="$built/embed"
rm -rf "${embed:?}"
mkdir -p "$embed"
cp "$built/release/libuna_rt.a" "$embed/libuna_rt.a"
sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    else
        shasum -a 256 "$1" | cut -d ' ' -f 1
    fi
}
case $target in
    *-apple-darwin)
        strip_binary=$(xcrun --find strip) \
            || { echo "error: xcrun finds no strip; install Xcode or its Command Line Tools" >&2; exit 2; }
        ZERO_AR_DATE=1 xcrun strip -S "$embed/libuna_rt.a"
        # Apple's strip prints no version: the Xcode release it comes with names it, or, with
        # the Command Line Tools alone, their package version, and its digest pins the binary.
        toolchain=$(xcodebuild -version 2>/dev/null | sed -n -e 's/^Xcode \([0-9.]*\)$/Xcode \1/p' -e 's/^Build version \([0-9A-Za-z]*\)$/build \1/p' | paste -sd ' ' -)
        case $toolchain in
            "Xcode "*" build "*) ;;
            *)
                toolchain=$(pkgutil --pkg-info=com.apple.pkg.CLTools_Executables 2>/dev/null | sed -n 's/^version: \([0-9.]*\)$/Command Line Tools \1/p')
                ;;
        esac
        [ -n "$toolchain" ] || { echo "error: neither xcodebuild nor pkgutil names the toolchain of $strip_binary" >&2; exit 2; }
        strip_tool="xcrun strip -S ($toolchain), sha256 $(sha256_of "$strip_binary")"
        ;;
    *-linux-*)
        strip --strip-debug -D "$embed/libuna_rt.a"
        strip_tool="$(strip --version | head -n 1) --strip-debug -D, sha256 $(sha256_of "$(command -v strip)")"
        ;;
    *)
        echo "error: no strip recipe for $target" >&2
        exit 2
        ;;
esac
if command -v sha256sum >/dev/null 2>&1; then
    runtime_digest=$(sha256sum "$embed/libuna_rt.a" | cut -d ' ' -f 1)
else
    runtime_digest=$(shasum -a 256 "$embed/libuna_rt.a" | cut -d ' ' -f 1)
fi
echo "runtime: $embed/libuna_rt.a, sha256 $runtime_digest, stripped with $strip_tool"
echo "strip: $target: $strip_tool"
UNA_EMBED_RUNTIME=$embed/libuna_rt.a
UNA_EMBED_RUNTIME_SHA256=$runtime_digest
export UNA_EMBED_RUNTIME UNA_EMBED_RUNTIME_SHA256
# The musl inputs of a Linux `una` (ADR 0124 rules 12 and 20, step 4a): musl's C library, the
# start-up objects of a static-pie executable and libunwind.a, from the pinned toolchain's
# self-contained directory of the musl target of this machine's architecture (the rust-std
# component that rust-toolchain.toml's targets install), never from the building machine's own
# C library. Each file's digest must be the one supply-chain/musl.lock pins for that musl target;
# it is printed on a line `musl: <target>: <file> sha256 <digest>`, which scripts/release-gates.sh
# copies into the validation record's section "musl inputs" (#1971),
# and passed with the directory to una-cli's build script, which checks each file against the
# lock again and embeds it. Only this target's inputs are carried (rule 12). The internal linker
# of Linux links them from slice 4c on; until then `una build` on Linux links through `cc` with
# the runtime built above, and checks these bytes before each link.
case $target in
    *-linux-*)
        musl_target=${target%%-*}-unknown-linux-musl
        musl_dir="$sysroot/lib/rustlib/$musl_target/lib/self-contained"
        musl_digests=
        for file in libc.a rcrt1.o crti.o crtn.o crtbeginS.o crtendS.o libunwind.a; do
            [ -f "$musl_dir/$file" ] \
                || { echo "error: $musl_dir/$file is missing; install the toolchain's $musl_target target (rustup target add $musl_target), which rust-toolchain.toml lists" >&2; exit 2; }
            digest=$(sha256_of "$musl_dir/$file")
            pinned=$(sed -n '/^\['"$musl_target"'\]$/,/^\[/s/^"'"$file"'" *= *"\([0-9a-f]*\)"$/\1/p' supply-chain/musl.lock | head -n 1)
            [ -n "$pinned" ] \
                || { echo "error: supply-chain/musl.lock records no digest of $file for $musl_target" >&2; exit 2; }
            [ "$digest" = "$pinned" ] \
                || { echo "error: $musl_dir/$file has SHA-256 $digest, but supply-chain/musl.lock pins $pinned for $musl_target; a toolchain bump updates the lock after scripts/scan-musl.sh passes" >&2; exit 2; }
            echo "musl: $target: $file sha256 $digest"
            musl_digests="$musl_digests${musl_digests:+ }$file=$digest"
        done
        UNA_EMBED_MUSL_DIR=$musl_dir
        UNA_EMBED_MUSL_SHA256S=$musl_digests
        export UNA_EMBED_MUSL_DIR UNA_EMBED_MUSL_SHA256S
        ;;
esac
# The Linux toolchain that links `una` (#1926, #1958): scripts/check-toolchain.sh refuses
# (exit 2) an image whose Debian packages or whose files that the linker puts into `una`
# (libstdc++.a, libgcc.a, the start-up objects) differ from supply-chain/z3.lock, before `una`
# is built, and prints a line `toolchain: <target>: ...` for each, which scripts/release-gates.sh
# copies into the validation record's section "Linux toolchain" (#1971).
case $target in
    *-linux-*) scripts/check-toolchain.sh "$target" || exit 2 ;;
esac
cargo build --release --locked --target-dir "$built" --package una-cli
# The trace of the link that wrote this `una`: exactly one trace names a file that is it byte
# for byte (cargo copies or links rustc's output to release/una).
case $target in
    *-linux-*)
        una_trace=
        for linked_trace in "$UNA_LINK_TRACE_DIR"/link.*; do
            [ -f "$linked_trace" ] || continue
            linked=$(sed -n '1s/^output: //p' "$linked_trace")
            if [ -n "$linked" ] && [ -f "$linked" ] && cmp -s "$linked" "$built/release/una"; then
                [ -z "$una_trace" ] || { echo "error: two link traces of $UNA_LINK_TRACE_DIR name the release una" >&2; exit 2; }
                una_trace=$linked_trace
            fi
        done
        [ -n "$una_trace" ] || { echo "error: no link trace of $UNA_LINK_TRACE_DIR names the release una" >&2; exit 2; }
        cp "$una_trace" "$built/una.link-trace"
        echo "link trace: $target: $built/una.link-trace, of $(sed -n '1s/^output: //p' "$una_trace")"
        ;;
esac
name="una-$version-$target"
out="$root/target/release-archives"
rm -rf "${out:?}/$name" "$out/$name.tar.gz" "$out/$name.tar.gz.sha256"
mkdir -p "$out/$name/docs/guide"
cp "$built/release/una" LICENSE README.md "$out/$name/"
# The guides, at the paths the README links, so the archive alone teaches Una (#410, N-65).
cp docs/guide/getting-started.md docs/guide/syntax-reference.md docs/guide/una-compact.md docs/guide/una-for-agents.md "$out/$name/docs/guide/"

# The archive is a function of the commit, so two builds of one commit give the same bytes
# (#679): the binaries already are, with the paths above remapped, and the packing fixes what
# the file system and the clock would add. Every entry has the commit's time, modes 755 for the
# directories and `una` and 644 for the rest, owner 0:0 with no user or group name, and the
# entries go in byte order of their paths; gzip stores no name and no time. Packing is linear in
# the size of the files, and sorting the dozen paths is negligible.
epoch=${SOURCE_DATE_EPOCH:-}
if [ -z "$epoch" ]; then
    epoch=$(git -C "$root" log -1 --format=%ct HEAD 2>/dev/null) || epoch=""
fi
case "$epoch" in
'' | *[!0-9]*)
    echo "error: no commit time for the archive's entries; set SOURCE_DATE_EPOCH to the commit's (git log -1 --format=%ct)" >&2
    exit 2
    ;;
esac
# `touch -t` takes local time, so the stamp and touch both use UTC; BSD date reads seconds with
# -r, GNU date with -d @.
stamp=$(TZ=UTC date -r "$epoch" +%Y%m%d%H%M.%S 2>/dev/null || TZ=UTC date -d "@$epoch" +%Y%m%d%H%M.%S) || {
    echo "error: date cannot read $epoch as seconds since the epoch" >&2
    exit 2
}
find "$out/$name" -type d -exec chmod 755 {} +
find "$out/$name" -type f -exec chmod 644 {} +
chmod 755 "$out/$name/una"
find "$out/$name" -exec env TZ=UTC touch -t "$stamp" {} +
list="$out/$name.list"
(cd "$out" && find "$name" | LC_ALL=C sort) > "$list"
if tar --version 2>/dev/null | grep -q 'GNU tar'; then
    tar -C "$out" --format=gnu --no-recursion --numeric-owner --owner=0 --group=0 \
        -cf "$out/$name.tar" -T "$list"
elif tar --version 2>/dev/null | grep -q 'bsdtar'; then
    # macOS adds extended attributes, ACLs and resource forks unless told not to.
    tar -C "$out" --format ustar -n --uid 0 --gid 0 --uname '' --gname '' \
        --no-xattrs --no-acls --no-mac-metadata -cf "$out/$name.tar" -T "$list"
else
    echo "error: tar is neither GNU tar nor bsdtar, so the archive's packing is not pinned" >&2
    exit 2
fi
gzip -n -f "$out/$name.tar"
rm -rf "${out:?}/$name" "$list"
cd "$out"
if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$name.tar.gz" > "$name.tar.gz.sha256"
else
    shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256"
fi
echo "packed $out/$name.tar.gz"
