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
# Every entry is owned by 0:0 with no user or group name, so the archive does not carry the
# account that built it (#679). The listing's owner columns differ between the two tars.
if tar --version 2>/dev/null | grep -q 'GNU tar'; then
    owners=$(tar -tvzf "$out/$name.tar.gz" | awk '{ print $2 }' | LC_ALL=C sort -u | tr '\n' ' ')
    expected="0/0 "
else
    owners=$(tar -tvzf "$out/$name.tar.gz" | awk '{ print $3 "/" $4 }' | LC_ALL=C sort -u | tr '\n' ' ')
    expected="0/0 "
fi
if [ "$owners" != "$expected" ]; then
    echo "error: $name.tar.gz has entries owned by $owners; expected only 0/0 with no names" >&2
    exit 1
fi
contents=$(cd "$tmp/$name" && LC_ALL=C ls | tr '\n' ' ')
if [ "$contents" != "LICENSE README.md docs lib una " ]; then
    echo "error: $name.tar.gz holds $contents; expected LICENSE README.md docs lib una" >&2
    exit 1
fi
# The guides, byte for byte the tree's, and nothing else under docs/ (#410, N-65): an agent's
# first contact with Una is the archive.
guides=$(cd "$tmp/$name/docs" && LC_ALL=C find . -type f | LC_ALL=C sort | tr '\n' ' ')
if [ "$guides" != "./guide/getting-started.md ./guide/syntax-reference.md ./guide/una-compact.md ./guide/una-for-agents.md " ]; then
    echo "error: $name.tar.gz holds docs/ $guides; expected the four guides under docs/guide/" >&2
    exit 1
fi
for guide in getting-started.md syntax-reference.md una-compact.md una-for-agents.md; do
    if ! cmp -s "$tmp/$name/docs/guide/$guide" "docs/guide/$guide"; then
        echo "error: $name.tar.gz holds a docs/guide/$guide that differs from the tree's" >&2
        exit 1
    fi
done
if [ ! -f "$tmp/$name/lib/libuna_rt.a" ]; then
    echo "error: $name.tar.gz has no lib/libuna_rt.a" >&2
    exit 1
fi
# No part of the hunt's oracle runner (ADR 0115), which prints labeled values: the listing above
# already admits no `una-oracle` binary, and neither the binary nor the runtime may hold its entry
# points or its schema, which only a build with the `oracle` feature has.
for file in "$tmp/$name/una" "$tmp/$name/lib/libuna_rt.a"; do
    for mark in una.oracle-run/1 run_observed run_main_observed una-oracle; do
        if LC_ALL=C grep -a -q -F -e "$mark" "$file"; then
            echo "error: $name.tar.gz has \"$mark\" in ${file#"$tmp/$name/"}: the oracle runner never ships (ADR 0115)" >&2
            exit 1
        fi
    done
done
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
# `una --json run` is the same with or without the hunt's oracle runner in the build (ADR 0115):
# `UNA_ORACLE_UNA`, a `una` built with the `oracle` feature (scripts/local-ci.sh builds one for
# this), gives the same document of every example, byte for byte, as the packed `una`, which has
# none of it. Where it is set, the comparison is required: the binary must exist and have the
# feature, or the check fails.
if [ -n "${UNA_ORACLE_UNA:-}" ]; then
    oracle_una=$UNA_ORACLE_UNA
    if [ ! -x "$oracle_una" ]; then
        echo "error: UNA_ORACLE_UNA names $oracle_una, which is not an executable" >&2
        exit 1
    fi
    if ! LC_ALL=C grep -a -q -F -e una.oracle-run/1 "$oracle_una"; then
        echo "error: $oracle_una lacks the oracle feature, so comparing with it proves nothing" >&2
        exit 1
    fi
    # The comparison masks one host-supplied field, each `Clock.now` event's outcome value, which
    # differs between any two runs; it must catch every other difference, which its self-test
    # checks first.
    python3 -B scripts/compare-run-docs.py --self-test || exit 1
    for example in examples/*/; do
        example=${example%/}
        compared=$(basename "$example")
        # Each binary runs its own fresh copy, so no state of one run reaches the other.
        cp -R "$example" "$tmp/compare-$compared-packed"
        cp -R "$example" "$tmp/compare-$compared-oracle"
        (cd "$tmp/compare-$compared-packed" && "$una" --json run . >"$tmp/compare-$compared-packed.json" 2>/dev/null || true)
        (cd "$tmp/compare-$compared-oracle" && "$oracle_una" --json run . >"$tmp/compare-$compared-oracle.json" 2>/dev/null || true)
        if ! masked=$(python3 -B scripts/compare-run-docs.py "$tmp/compare-$compared-packed.json" "$tmp/compare-$compared-oracle.json"); then
            echo "error: una --json run of $example differs between the packed una and $oracle_una (ADR 0115)" >&2
            exit 1
        fi
        echo "ok: $example: $masked"
    done
    echo "ok: una --json run of every example is the same with the oracle feature built in"
else
    echo "note: UNA_ORACLE_UNA is not set, so una --json run is not compared with an oracle build here; this is not a check (scripts/local-ci.sh runs it)"
fi
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
