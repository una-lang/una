#!/bin/sh
# Builds `una` in release mode and packs it with the license and the readme into
# target/release-archives/una-<version>-<target>.tar.gz, with its SHA-256 digest beside it in
# a .sha256 file (ADR 0035 rule 3). Each target builds natively on its own
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

cargo build --release --locked --package una-cli
name="una-$version-$target"
out="$root/target/release-archives"
rm -rf "${out:?}/$name" "$out/$name.tar.gz" "$out/$name.tar.gz.sha256"
mkdir -p "$out/$name"
cp target/release/una LICENSE README.md "$out/$name/"
tar -C "$out" -czf "$out/$name.tar.gz" "$name"
rm -rf "${out:?}/$name"
cd "$out"
if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$name.tar.gz" > "$name.tar.gz.sha256"
else
    shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256"
fi
echo "packed $out/$name.tar.gz"
