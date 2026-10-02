#!/bin/sh
# Checks the archive that scripts/release.sh packed for TARGET: its SHA-256 digest, its contents,
# the packed binary on the examples, and a native build of examples/hello with the packed runtime;
# it needs the pinned Z3 on PATH and the system `cc` (ADR 0035 rule 3, ADR 0059).
#
# Usage: scripts/check-release.sh TARGET
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: scripts/check-release.sh TARGET" >&2
    exit 2
fi
target=$1
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
version=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([^"]*\)"$/\1/p' Cargo.toml)
name="una-$version-$target"
out="$root/target/release-archives"

(
    cd "$out"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum --check "$name.tar.gz.sha256"
    else
        shasum -a 256 --check "$name.tar.gz.sha256"
    fi
)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
tar -xzf "$out/$name.tar.gz" -C "$tmp"
contents=$(cd "$tmp/$name" && LC_ALL=C ls | tr '\n' ' ')
if [ "$contents" != "LICENSE README.md lib una " ]; then
    echo "error: $name.tar.gz holds $contents; expected LICENSE README.md lib una" >&2
    exit 1
fi
if [ ! -f "$tmp/$name/lib/libuna_rt.a" ]; then
    echo "error: $name.tar.gz has no lib/libuna_rt.a" >&2
    exit 1
fi
una="$tmp/$name/una"
"$una" check examples/events
"$una" test examples/events > /dev/null
greeting=$("$una" run examples/hello)
if [ "$greeting" != "Hello, Una" ]; then
    echo "error: the packed una printed \"$greeting\" for examples/hello" >&2
    exit 1
fi
cp -R examples/hello "$tmp/hello"
"$una" build "$tmp/hello" > /dev/null
native=$("$tmp/hello/target/una/hello")
if [ "$native" != "Hello, Una" ]; then
    echo "error: the natively built examples/hello printed \"$native\"" >&2
    exit 1
fi
echo "ok: $name.tar.gz checks and runs the examples, and builds examples/hello natively"
