#!/bin/sh
# Installs the Z3 release pinned in verification.lock as DIR/bin/z3.
#
# Usage: scripts/install-z3.sh DIR
#
# The version comes from verification.lock (spec/12 §9). Each download is checked against the
# SHA-256 digest published with the GitHub release, so changing solver_version requires adding
# the new release assets and digests below.
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: scripts/install-z3.sh DIR" >&2
    exit 2
fi
dest=$1
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
version=$(sed -n 's/^solver_version *= *"\([^"]*\)"$/\1/p' "$root/verification.lock")
if [ -z "$version" ]; then
    echo "error: verification.lock has no solver_version" >&2
    exit 2
fi

platform="$(uname -s) $(uname -m)"
case "$version $platform" in
    "5.1.0 Linux x86_64")
        asset=z3-5.1.0-x64-glibc-2.39
        sha256=f47be8d27d3230e823bf1eeede2fe0abaca55bb78d0b59974370e6689a92284a ;;
    "5.1.0 Linux aarch64")
        asset=z3-5.1.0-arm64-glibc-2.38
        sha256=2832cfd43c6862fbaf78834b5f1e35d031b4a87e85456b696c1c6f609cd5609e ;;
    "5.1.0 Darwin arm64")
        asset=z3-5.1.0-arm64-osx-13.3
        sha256=81d29e934fd863079a74af35eecaeaef8047e0e12414d33ca322b358d68383db ;;
    "5.1.0 Darwin x86_64")
        asset=z3-5.1.0-x64-osx-13.3
        sha256=f25cabdb01a32942a42227c36902c0846c759dcaabef4aca3b2e86331447f6ff ;;
    *)
        echo "error: no pinned z3 $version build for $platform; add its release asset and SHA-256 digest to scripts/install-z3.sh" >&2
        exit 2 ;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
url="https://github.com/Z3Prover/z3/releases/download/z3-$version/$asset.zip"
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
unzip -q "$tmp/$asset.zip" "$asset/bin/z3" -d "$tmp"
mkdir -p "$dest/bin"
cp "$tmp/$asset/bin/z3" "$dest/bin/z3"
chmod +x "$dest/bin/z3"
echo "installed z3 $version at $dest/bin/z3"
