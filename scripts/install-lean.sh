#!/bin/sh
# Installs the Lean 4 release pinned in formal/lean-toolchain in DIR, with `lean` and `lake` in
# DIR/bin (the formal track, docs/plan/formal-track.md §6, #46).
#
# Usage: scripts/install-lean.sh DIR
#
# Each download is checked against the SHA-256 digest that GitHub publishes with the release
# asset, so changing the toolchain requires adding its assets and digests below, as
# scripts/install-z3.sh does for the solver. No `elan`: one pinned release, nothing fetched later.
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: scripts/install-lean.sh DIR" >&2
    exit 2
fi
dest=$1
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
toolchain=$(cat "$root/formal/lean-toolchain")
version=${toolchain#leanprover/lean4:v}
if [ "$version" = "$toolchain" ] || [ -z "$version" ]; then
    echo "error: formal/lean-toolchain names no leanprover/lean4 release: $toolchain" >&2
    exit 2
fi

platform="$(uname -s) $(uname -m)"
case "$version $platform" in
    "4.34.0 Linux x86_64")
        asset=lean-4.34.0-linux
        sha256=5f14e0f3762058a529b1598f4355e6787c5f155be1bcce72ae288b9c35e3adb4 ;;
    "4.34.0 Linux aarch64")
        asset=lean-4.34.0-linux_aarch64
        sha256=e66b1de1a849cc8eac59a79884462f58d4776965e40e1b4ecf7bb43fd87c612f ;;
    "4.34.0 Darwin arm64")
        asset=lean-4.34.0-darwin_aarch64
        sha256=b19b8afd6327112d45305fb71a420e60ddf0ef7d7049108b03e1451df4eda88e ;;
    "4.34.0 Darwin x86_64")
        asset=lean-4.34.0-darwin
        sha256=75a2a4b8e3d35680e8f51fae5a42c79702f463dd73ee1520fb61e5a9143e336c ;;
    *)
        echo "error: no pinned Lean $version build for $platform; add its release asset and SHA-256 digest to scripts/install-lean.sh" >&2
        exit 2 ;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
url="https://github.com/leanprover/lean4/releases/download/v$version/$asset.zip"
echo "downloading $url"
curl --fail --location --silent --show-error --retry 3 --output "$tmp/$asset.zip" "$url"
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$tmp/$asset.zip" | cut -d ' ' -f 1)
else
    actual=$(shasum -a 256 "$tmp/$asset.zip" | cut -d ' ' -f 1)
fi
if [ "$actual" != "$sha256" ]; then
    echo "error: SHA-256 mismatch for $asset.zip: expected $sha256, found $actual" >&2
    exit 1
fi
unzip -q "$tmp/$asset.zip" -d "$tmp"
mkdir -p "$dest"
cp -R "$tmp/$asset/." "$dest/"
echo "installed Lean $version at $dest/bin/lean"
