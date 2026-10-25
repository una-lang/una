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
# Each command that runs the packed `una` or what it builds runs under a time limit, so that a
# hang fails the check in minutes instead of holding the leg for days (#1133). Under emulation
# (the x86_64 leg on an arm64 Mac, through Rosetta), a multithreaded `una` that starts several
# solver processes at once can deadlock inside the translator; the leg sets
# UNA_RELEASE_CHECK_RETRY=1, and a command that runs out of time is then tried once more, after a
# dump of its threads and its children's, with the retry and its cause in the log
# (docs/release-gates.md).
dump_threads() {
    echo "-- threads of $1 and its children, at $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    ps -o pid,ppid,stat,command -p "$1" 2>/dev/null || true
    for pid in "$1" $(pgrep -P "$1" 2>/dev/null || true); do
        [ -d "/proc/$pid/task" ] || continue
        for task in "/proc/$pid/task"/*; do
            echo "pid $pid tid ${task##*/} $(cat "$task/comm" 2>/dev/null) wchan=$(cat "$task/wchan" 2>/dev/null) syscall=$(cut -d' ' -f1 "$task/syscall" 2>/dev/null)"
        done
    done
}
# bounded SECONDS OUT COMMAND...: runs COMMAND with its stdout in OUT and its stderr in OUT.err;
# exits with COMMAND's status, or 124 when it is still running after SECONDS, then dumped and
# killed with its children.
bounded() {
    limit=$1 into=$2
    shift 2
    "$@" > "$into" 2> "$into.err" &
    pid=$!
    ticks=0
    while kill -0 "$pid" 2>/dev/null; do
        if [ "$ticks" -ge $((limit * 5)) ]; then
            dump_threads "$pid" >&2
            pkill -9 -P "$pid" 2>/dev/null || true
            kill -9 "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
            return 124
        fi
        sleep 0.2
        ticks=$((ticks + 1))
    done
    wait "$pid"
}
# limited SECONDS OUT COMMAND...: `bounded`, tried once more under UNA_RELEASE_CHECK_RETRY=1 when
# the first try ran out of time. A second timeout, or a timeout without the retry, is reported and
# leaves "$tmp/timed-out", which the check reads after every command, even one run inside `$(...)`.
limited() {
    status=0
    bounded "$@" || status=$?
    if [ "$status" = 124 ] && [ "${UNA_RELEASE_CHECK_RETRY:-}" = 1 ]; then
        echo "retry: \`$3\` ran out of its $1 s; emulator deadlock suspected (#1133); trying it once more" >&2
        status=0
        bounded "$@" || status=$?
    fi
    if [ "$status" = 124 ]; then
        seconds=$1
        shift 2
        echo "error: \`$*\` ran out of its time limit of $seconds s (#1133)" >&2
        : > "$tmp/timed-out"
    fi
    return "$status"
}
# timed_out: fails the check when a command ran out of its time limit.
timed_out() {
    if [ -e "$tmp/timed-out" ]; then
        exit 1
    fi
}
# shown DIR SECONDS COMMAND...: what `$(cd DIR && COMMAND 2>/dev/null || echo "exit $?")` gives,
# under the time limit: COMMAND's stdout, and `exit N` on a line of its own when it fails.
shown() {
    place=$1 seconds=$2
    shift 2
    code=0
    (cd "$place" && limited "$seconds" "$tmp/shown" "$@") || code=$?
    if [ "$code" = 124 ]; then
        return 124
    fi
    cat "$tmp/shown"
    if [ "$code" != 0 ]; then
        echo "exit $code"
    fi
}
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
# Keyed hashing's test hooks (a fixed key, a probe counter, a constant hash) exist only in the
# toolchain's own test builds (ADR 0119 rule 9). Their module holds a marker that its key path
# keeps: neither the symbol, by `nm`, nor its bytes, which a stripped binary still holds, may be
# in the shipped `una` or `libuna_rt.a`.
marker=UNA_HASH_TEST_HOOKS_MARKER
for artifact in "$tmp/$name/una" "$tmp/$name/lib/libuna_rt.a"; do
    if nm -a "$artifact" 2>/dev/null | grep -q "$marker" || LC_ALL=C grep -a -q "$marker" "$artifact"; then
        echo "error: $artifact was built with keyed hashing's test hooks (ADR 0119 rule 9)" >&2
        exit 1
    fi
done
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
una="$tmp/$name/una"
limited 300 "$tmp/step" "$una" check examples/events || { cat "$tmp/step" "$tmp/step.err" >&2; exit 1; }
limited 300 "$tmp/step" "$una" test examples/events || { cat "$tmp/step" "$tmp/step.err" >&2; exit 1; }
limited 300 "$tmp/step" "$una" run examples/hello || { cat "$tmp/step.err" >&2; exit 1; }
greeting=$(cat "$tmp/step")
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
    expected=$(shown "$dir" 300 "$una" run .) || true
    timed_out
    limited 600 "$tmp/step" "$una" build "$dir" --out "$dir/native-exe" || { cat "$tmp/step.err" >&2; exit 1; }
    native=$(shown "$dir" 120 ./native-exe) || true
    timed_out
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
    limited 600 "$tmp/step" "$install/una" build "$tmp/placed" --out "$install/out/hello" || { cat "$tmp/step.err" >&2; exit 1; }
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
