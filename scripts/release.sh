#!/bin/sh
# Builds `una` and the native runtime library in release mode and packs them with the license
# and the readme into target/release-archives/una-<version>-<target>.tar.gz, with `una`,
# `lib/libuna_rt.a`, `LICENSE` and `README.md`, and its SHA-256 digest beside it in a .sha256
# file (ADR 0035 rule 3, ADR 0059). Each target builds natively on its own
# runner, so TARGET must be the host's.
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
RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$root=/una --remap-path-prefix=$cargo_home=/cargo --remap-path-prefix=$sysroot=/rust"
export RUSTFLAGS
# The C and assembly sources of dependencies (ring's), which the `cc` crate compiles, the same.
CFLAGS="${CFLAGS:-} -ffile-prefix-map=$root=/una -ffile-prefix-map=$cargo_home=/cargo"
export CFLAGS
built="$root/target/release-remapped"
cargo build --release --locked --target-dir "$built" --package una-cli --package una-rt
name="una-$version-$target"
out="$root/target/release-archives"
rm -rf "${out:?}/$name" "$out/$name.tar.gz" "$out/$name.tar.gz.sha256"
mkdir -p "$out/$name/lib"
cp "$built/release/una" LICENSE README.md "$out/$name/"
cp "$built/release/libuna_rt.a" "$out/$name/lib/"
tar -C "$out" -czf "$out/$name.tar.gz" "$name"
rm -rf "${out:?}/$name"
cd "$out"
if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$name.tar.gz" > "$name.tar.gz.sha256"
else
    shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256"
fi
echo "packed $out/$name.tar.gz"
