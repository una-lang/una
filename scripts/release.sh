#!/bin/sh
# Builds `una` and the native runtime library in release mode and packs them with the license
# and the readme into target/release-archives/una-<version>-<target>.tar.gz, with `una`,
# `lib/libuna_rt.a`, `LICENSE`, `README.md` and the four guides under `docs/guide/` (#410, #430), and its SHA-256 digest beside it in a .sha256
# file (ADR 0035 rule 3, ADR 0059). Each target builds natively on its own
# runner, so TARGET must be the host's. Two builds of one commit give the same archive (#679):
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
# rebuilt here; una-cli's build checks the digest again. On Linux, libstdc++ and libgcc are
# linked statically into una, so it depends on glibc alone (rule 1).
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
case $target in
    *-linux-*) RUSTFLAGS="$RUSTFLAGS -C link-arg=-static-libstdc++ -C link-arg=-static-libgcc" ;;
esac
export RUSTFLAGS
# The C and assembly sources of dependencies (ring's), which the `cc` crate compiles, the same.
CFLAGS="${CFLAGS:-} -ffile-prefix-map=$root=/una -ffile-prefix-map=$cargo_home=/cargo"
export CFLAGS
built="$root/target/release-remapped"
cargo build --release --locked --target-dir "$built" --package una-cli --package una-rt
name="una-$version-$target"
out="$root/target/release-archives"
rm -rf "${out:?}/$name" "$out/$name.tar.gz" "$out/$name.tar.gz.sha256"
mkdir -p "$out/$name/lib" "$out/$name/docs/guide"
cp "$built/release/una" LICENSE README.md "$out/$name/"
# The guides, at the paths the README links, so the archive alone teaches Una (#410, N-65).
cp docs/guide/getting-started.md docs/guide/syntax-reference.md docs/guide/una-compact.md docs/guide/una-for-agents.md "$out/$name/docs/guide/"
cp "$built/release/libuna_rt.a" "$out/$name/lib/"

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
