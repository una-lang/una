#!/bin/sh
# Checks the archive that scripts/release.sh packed for TARGET: its SHA-256 digest, its contents,
# the packed binary on the examples, and a native build of every example with the packed runtime;
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
# Every example, built natively with the packed runtime, prints what `una run` prints and exits
# with its status (spec/16 §5); each runs in its own directory, where its grants resolve.
for example in examples/*/; do
    example=${example%/}
    dir="$tmp/native-$(basename "$example")"
    cp -R "$example" "$dir"
    expected=$(cd "$dir" && "$una" run . 2>/dev/null || echo "exit $?")
    "$una" build "$dir" --out "$dir/native-exe" > /dev/null
    native=$(cd "$dir" && ./native-exe 2>/dev/null || echo "exit $?")
    if [ "$native" != "$expected" ]; then
        echo "error: the natively built $example printed \"$native\"; una run printed \"$expected\"" >&2
        exit 1
    fi
done
# The archive and what it builds carry no path of a machine (#382, S-13). The same program built
# by the archive unpacked in two places is the same executable; and neither the packed `una`, nor
# its runtime, nor that executable names a home directory or the source tree it was built from.
# Rust's own standard library names the machine it was built on, `/Users/runner/work/rust`.
cp -R examples/hello "$tmp/placed"
for install in "$tmp/install-one" "$tmp/a-second-install-elsewhere"; do
    mkdir -p "$install"
    cp -R "$tmp/$name/." "$install/"
    "$install/una" build "$tmp/placed" --out "$install/out/hello" > /dev/null
done
if ! cmp -s "$tmp/install-one/out/hello" "$tmp/a-second-install-elsewhere/out/hello"; then
    echo "error: examples/hello built by $name in two places gives two different executables" >&2
    exit 1
fi
# The builder's user name is refused too, in any form (`~name`, a home elsewhere), when it is long
# enough to mean something; inside Rust's own path, as a CI runner's user is, it is allowed.
user=$(id -un 2>/dev/null || true)
for file in "$tmp/$name/una" "$tmp/$name/lib/libuna_rt.a" "$tmp/install-one/out/hello"; do
    found=$(grep -a -o -E '(/Users|/home)/[^[:space:][:cntrl:]"]*' "$file" | grep -v '^/Users/runner/work/rust/' | LC_ALL=C sort -u | head -5 || true)
    if [ "${#user}" -ge 4 ] && [ -z "$found" ]; then
        found=$(LC_ALL=C grep -a -o -E ".{0,16}$user.{0,16}" "$file" | grep -v -F '/Users/runner/work/rust' | LC_ALL=C sort -u | head -5 || true)
    fi
    if [ -n "$found" ] || grep -a -q -F "$root" "$file"; then
        echo "error: $file names a path of the machine that built it:" >&2
        echo "${found:-$root}" >&2
        exit 1
    fi
done
echo "ok: $name.tar.gz checks and runs the examples, and builds each of them natively"
