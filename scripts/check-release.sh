#!/bin/sh
# Checks the archive that scripts/release.sh packed for TARGET: its SHA-256 digest, its contents,
# the packed binary on the examples, and a native build of every example with the runtime built
# into the packed `una` (ADR 0035 rule 3, ADR 0059 and its amendment, ADR 0124 step 2). The
# archive holds no `lib/`: the runtime is scanned as scripts/release.sh left it,
# target/release-remapped/embed/libuna_rt.a, once its digest is the one the packed `una`
# reports. On macOS `una build` links by itself (ADR 0124 step 3); on Linux it still needs the
# system `cc` until step 4.
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
# Only a shipped target's archive is checked (scripts/release-targets.txt, #1903).
. scripts/release-targets.sh
shipped=$(release_targets scripts/release-targets.txt) || exit 2
if ! printf '%s\n' "$shipped" | grep -qxF -- "$target"; then
    echo "error: $target is not a shipped target; scripts/release-targets.txt lists the targets a release checks" >&2
    exit 2
fi
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
# Every entry lies under the archive's own directory, `$name/`, with no `..` component, and is a
# file or a directory: no link, hard or symbolic, which could lead an unpacking outside it or
# install something the archive does not hold (#1359). The listing's first column is the mode,
# whose first character is the entry's type in both tars.
entries=$(tar -tzf "$out/$name.tar.gz")
outside=$(printf '%s\n' "$entries" | awk -v top="$name" '
    $0 != top && $0 != top "/" && index($0, top "/") != 1 { print; next }
    $0 ~ /(^|\/)\.\.(\/|$)/ { print }' | head -5)
if [ -n "$outside" ]; then
    echo "error: $name.tar.gz has entries outside $name/:" >&2
    echo "$outside" >&2
    exit 1
fi
links=$(tar -tvzf "$out/$name.tar.gz" | awk 'substr($1, 1, 1) != "-" && substr($1, 1, 1) != "d"' | head -5)
if [ -n "$links" ]; then
    echo "error: $name.tar.gz has entries that are neither files nor directories:" >&2
    echo "$links" >&2
    exit 1
fi
# The runtime is built into `una` (ADR 0124 step 2): an archive with a `lib/` entry, which an
# older `una` read its runtime from, is refused.
if [ -e "$tmp/$name/lib" ] || tar -tzf "$out/$name.tar.gz" | grep -q "^$name/lib\(/\|$\)"; then
    echo "error: $name.tar.gz has a lib/ entry; the runtime is built into una (ADR 0124 step 2)" >&2
    exit 1
fi
# `ls -A`, so that a dotfile at the top level is seen too (#1359).
contents=$(cd "$tmp/$name" && LC_ALL=C ls -A | tr '\n' ' ')
if [ "$contents" != "LICENSE README.md docs una " ]; then
    echo "error: $name.tar.gz holds $contents; expected LICENSE README.md docs una" >&2
    exit 1
fi
una="$tmp/$name/una"
# A shipped una is never a development build: it has the release solver and the runtime built
# in (ADR 0124 rule 19).
if "$una" --version | grep -q '^development build$'; then
    echo "error: $una is a development build, without the release solver or the runtime built in (ADR 0124 rule 19)" >&2
    exit 1
fi
# The glibc floor of a Linux `una` (#2105): the packed `una` asks the loader for no `GLIBC_`
# symbol version above the floor that install.sh refuses a system below (its `glibc_floor`,
# read here so that the two cannot differ), by scripts/glibc-floor.sh. The Z3 library and the
# C++ runtime are linked into `una`, so their needs are read with it. The same scan with the
# floor 2.17, the first glibc of AArch64, which every executable linked against a later glibc
# exceeds, is the canary: it must fail, or the scan proves nothing.
glibc_floor=
case $target in
    *-linux-*)
        glibc_floor=$(sed -n 's/^glibc_floor=\([0-9][0-9]*\.[0-9][0-9]*\)$/\1/p' install.sh)
        if [ -z "$glibc_floor" ] || [ "$(printf '%s\n' "$glibc_floor" | wc -l)" -ne 1 ]; then
            echo "error: install.sh does not set glibc_floor to one MAJOR.MINOR; the glibc floor of $una cannot be checked" >&2
            exit 1
        fi
        if scripts/glibc-floor.sh 2.17 "$una" > /dev/null 2>&1; then
            echo "error: the glibc floor scan accepts $una at the floor 2.17; it cannot be trusted" >&2
            exit 1
        fi
        scripts/glibc-floor.sh "$glibc_floor" "$una"
        ;;
esac
# The runtime built into the packed `una`, as scripts/release.sh stripped and embedded it: its
# digest must be the one the packed `una` reports (`native.runtime_sha256`, ADR 0124 rule 28),
# so the scans below read the bytes that every executable links.
runtime="$root/target/release-remapped/embed/libuna_rt.a"
if [ ! -f "$runtime" ]; then
    echo "error: $runtime, the runtime scripts/release.sh embedded, is missing" >&2
    exit 1
fi
limited 300 "$tmp/report" "$una" --json report examples/events || true
timed_out
reported=$(grep -o '"runtime_sha256":"[0-9a-f]*"' "$tmp/report" | sed -e 's/^"runtime_sha256":"//' -e 's/"$//' || true)
if command -v sha256sum >/dev/null 2>&1; then
    embedded=$(sha256sum "$runtime" | cut -d ' ' -f 1)
else
    embedded=$(shasum -a 256 "$runtime" | cut -d ' ' -f 1)
fi
if [ -z "$reported" ] || [ "$reported" != "$embedded" ]; then
    echo "error: the packed una reports the runtime \"$reported\"; $runtime is $embedded" >&2
    exit 1
fi
# A Linux `una` carries the musl inputs of its own architecture (ADR 0124 rules 12 and 20, step
# 4a), exactly those supply-chain/musl.lock pins: each pinned digest is a string of its build
# record (`musl_sha256s`, `libunwind_sha256`), and the musl target of the other architecture
# contributes none. A macOS `una` carries none.
case $target in
    *-linux-*)
        musl_target=${target%%-*}-unknown-linux-musl
        pins=$(sed -n '/^\['"$musl_target"'\]$/,/^\[/s/^"[^"]*" *= *"\([0-9a-f]*\)"$/\1/p' supply-chain/musl.lock)
        [ "$(printf '%s\n' "$pins" | grep -c .)" -eq 7 ] \
            || { echo "error: supply-chain/musl.lock does not pin the 7 musl inputs of $musl_target" >&2; exit 1; }
        for pin in $pins; do
            grep -a -q "$pin" "$una" \
                || { echo "error: the packed una does not record the musl input $pin that supply-chain/musl.lock pins for $musl_target" >&2; exit 1; }
        done
        ;;
esac
# The runtime lost its debug sections (ADR 0124 rule 12, #1359), so that no line table or path of
# the machine that built it reaches an executable: no `__DWARF` segment or `__debug_*` section in
# a Mach-O member (`otool -l`), no `.debug_*` section in an ELF one (`objdump -h`). The runtime
# as cargo built it, before scripts/release.sh stripped it, is the canary: the same scan must
# find its debug sections, or the scan proves nothing.
debug_sections() {
    case $target in
        *-apple-darwin)
            otool -l "$1" > "$tmp/sections" || return 2
            grep -E '^[[:space:]]*(segname __DWARF|sectname __debug_)' "$tmp/sections" | LC_ALL=C sort -u | head -5
            ;;
        *)
            objdump -h "$1" > "$tmp/sections" || return 2
            grep -E '[[:space:]]\.debug_[a-z_]*' "$tmp/sections" | awk '{ print $2 }' | LC_ALL=C sort -u | head -5
            ;;
    esac
}
unstripped="$root/target/release-remapped/release/libuna_rt.a"
if [ ! -f "$unstripped" ]; then
    echo "error: $unstripped, the runtime before scripts/release.sh stripped it, is missing; the debug-section scan has no canary" >&2
    exit 1
fi
found=$(debug_sections "$unstripped") || { echo "error: cannot list the sections of $unstripped" >&2; exit 1; }
if [ -z "$found" ]; then
    echo "error: the debug-section scan finds no debug section in the unstripped $unstripped; it cannot be trusted" >&2
    exit 1
fi
found=$(debug_sections "$runtime") || { echo "error: cannot list the sections of $runtime" >&2; exit 1; }
if [ -n "$found" ]; then
    echo "error: $runtime, the runtime built into una, still has debug sections (ADR 0124 rule 12):" >&2
    echo "$found" >&2
    exit 1
fi
echo "ok: the runtime built into una has no debug section"
# Keyed hashing's test hooks (a fixed key, a probe counter, a constant hash) exist only in the
# toolchain's own test builds (ADR 0119 rule 9). Their module holds a marker that its key path
# keeps: neither the symbol, by `nm`, nor its bytes, which a stripped binary still holds, may be
# in the shipped `una` or `libuna_rt.a`. The same holds for the test hooks of the `Http`
# client's reach pool, a lookup and a connect function a test can make hang and a pool started
# on a first request (#1148), and the scripted serve engine of `tests/serve_runs/` (ADR 0123
# rule 21, una-rt's `test-hooks`), and the placement probe of the serve threads, which writes
# each connection's owner and can place by `port mod M` with no key (#1660), all of which only
# una-http's `test-hooks` feature compiles, and which can unify into the workspace's test builds.
for artifact in "$una" "$runtime"; do
    for marker in UNA_HASH_TEST_HOOKS_MARKER UNA_HTTP_TEST_HOOKS_MARKER; do
        if nm -a "$artifact" 2>/dev/null | grep -q "$marker" || LC_ALL=C grep -a -q "$marker" "$artifact"; then
            case $marker in
                UNA_HASH_*) what="keyed hashing's test hooks (ADR 0119 rule 9)" ;;
                *) what="una-http's test hooks: the Http client's (#1148), the scripted serve engine or the placement probe (#1660)" ;;
            esac
            echo "error: $artifact was built with $what" >&2
            exit 1
        fi
    done
done
# The solver child's stand-in and the stand-in for the embedded runtime (the `test-hooks` feature
# of una-cli, ADR 0124 rule 11 and step 2) exist only in the toolchain's own test builds; their
# marker may be in no shipped una.
marker=UNA_CLI_TEST_HOOKS_MARKER
if nm -a "$tmp/$name/una" 2>/dev/null | grep -q "$marker" || LC_ALL=C grep -a -q "$marker" "$tmp/$name/una"; then
    echo "error: $tmp/$name/una was built with the solver's test hooks (ADR 0124 rule 11)" >&2
    exit 1
fi
# AddressSanitizer's fiber interface and its unpoisoning are declared only in builds with
# `--cfg una_asan`, which only scripts/sanitize.sh and scripts/fuzz-smoke.sh set, the latter
# through the fuzz flags of scripts/fuzz-targets.sh (#1145, #1430): no shipped `una` or
# `libuna_rt.a` may refer to them, by symbol or by bytes.
asan_symbols="-e __sanitizer_start_switch_fiber -e __sanitizer_finish_switch_fiber -e __asan_unpoison_memory_region"
for artifact in "$una" "$runtime"; do
    # shellcheck disable=SC2086 # the patterns are split on purpose
    if nm -a "$artifact" 2>/dev/null | grep -q $asan_symbols \
        || LC_ALL=C grep -a -q $asan_symbols "$artifact"; then
        echo "error: $artifact refers to AddressSanitizer's interface (#1145)" >&2
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
limited 300 "$tmp/step" "$una" check examples/events || { cat "$tmp/step" "$tmp/step.err" >&2; exit 1; }
# The FMA disassembly gate (#1216; ADR 0114 Appendix C rules 7 and 8, as ADR 0120 Appendix A
# amends them): on x86-64, no FMA instruction in the shipped `una` or `libuna_rt.a` outside the
# dispatched kernel variants of the allowlist, which the packed `una` records in its build report
# (`native.kernels.fma_allowlist`); without a report, scripts/check-fma.sh uses its own copy of
# una_core::dense::variants::ALLOWLIST. On AArch64 the scan prints that it is not applicable.
allowlist=$(grep -o '"fma_allowlist":\[[^]]*\]' "$tmp/report" \
    | sed -e 's/^"fma_allowlist":\[//' -e 's/\]$//' \
    | tr ',' '\n' | sed -e 's/^"//' -e 's/"$//' || true)
set --
for prefix in $allowlist; do
    set -- "$@" --allow "$prefix"
done
if [ "$#" -eq 0 ]; then
    echo "note: no fma_allowlist in the report of the packed una; scripts/check-fma.sh uses its own"
fi
# The shipped `una` and `libuna_rt.a` also allow the four exact names of the toolchain's `fma`
# and `fmaf` (ADR 0120's amendment of 2026-11-27, #2106); the programs `una build` writes do not.
scripts/check-fma.sh "$@" --toolchain-fma "$una" "$runtime"
limited 300 "$tmp/step" "$una" test examples/events || { cat "$tmp/step" "$tmp/step.err" >&2; exit 1; }
limited 300 "$tmp/step" "$una" run examples/hello || { cat "$tmp/step.err" >&2; exit 1; }
greeting=$(cat "$tmp/step")
if [ "$greeting" != "Hello, Una" ]; then
    echo "error: the packed una printed \"$greeting\" for examples/hello" >&2
    exit 1
fi
# The packed una serves (spec/31 §1 rule 10), and so does the executable it builds:
# examples/health, on a free port, names the address on stderr, answers `GET /health` with 200
# and ends with 0 at SIGTERM, under `una run .` and as the executable of `una build`; its start
# and its stop are each bounded, and an EXIT trap kills it if the check ends first.
if ! command -v curl >/dev/null 2>&1; then
    echo "error: the serve check of examples/health needs curl, which is not on PATH" >&2
    exit 1
fi
dir="$tmp/serve-health"
cp -R examples/health "$dir"
serve_pid=
trap 'if [ -n "$serve_pid" ]; then kill -9 "$serve_pid" 2>/dev/null || true; fi; rm -rf "$tmp"' EXIT
# A port with no listener: one that `curl` cannot connect to, tried from a run-dependent start.
port=$((20000 + $$ % 20000))
tries=0
while curl -s -m 1 -o /dev/null "http://127.0.0.1:$port/" 2>/dev/null || [ "$?" != 7 ]; do
    tries=$((tries + 1))
    [ "$tries" -lt 50 ] || { echo "error: no free port for the serve check" >&2; exit 1; }
    port=$((20000 + ($port + 1) % 20000))
done
sed "s|127.0.0.1:8080|127.0.0.1:$port|" examples/health/ita.una > "$dir/ita.una"
limited 600 "$tmp/step" "$una" build "$dir" --out "$dir/native-exe" || { cat "$tmp/step.err" >&2; exit 1; }
# serve_health WHAT COMMAND...: COMMAND, run in $dir, serves examples/health as above.
serve_health() {
    what=$1
    shift
    (cd "$dir" && exec "$@") > "$tmp/serve.out" 2> "$tmp/serve.err" &
    serve_pid=$!
    ticks=0
    until grep -q "^serving on 127.0.0.1:$port (granted by the package)$" "$tmp/serve.err"; do
        if [ "$ticks" -ge 300 ] || ! kill -0 "$serve_pid" 2>/dev/null; then
            echo "error: $what did not serve examples/health within 60 s:" >&2
            cat "$tmp/serve.err" >&2
            exit 1
        fi
        sleep 0.2
        ticks=$((ticks + 1))
    done
    answer=$(curl -s -m 20 -o /dev/null -w '%{http_code}' "http://127.0.0.1:$port/health" || true)
    kill -TERM "$serve_pid"
    ticks=0
    while kill -0 "$serve_pid" 2>/dev/null; do
        if [ "$ticks" -ge 50 ]; then
            echo "error: $what still served examples/health 10 s after SIGTERM" >&2
            exit 1
        fi
        sleep 0.2
        ticks=$((ticks + 1))
    done
    code=0
    wait "$serve_pid" || code=$?
    serve_pid=
    if [ "$answer" != 200 ] || [ "$code" != 0 ]; then
        echo "error: $what answered \"$answer\" for /health of examples/health and exited $code" >&2
        exit 1
    fi
}
serve_health "una run of the packed una" "$una" run .
serve_health "the executable the packed una built" ./native-exe
# Every example, built natively with the runtime built into the packed una, prints what `una run` prints and exits
# with its status (spec/16 §5); each runs in its own directory, where its grants resolve. A package
# with a `serve` section serves until it is stopped: it is built, and served above. A package with
# a `drive` section is driven by the host until it is stopped (spec/26 §2): it runs, under `una run`
# and as the executable, with `--stop-after-iterations 5`, as the tests run it (#1835).
for example in examples/*/; do
    example=${example%/}
    dir="$tmp/native-$(basename "$example")"
    cp -R "$example" "$dir"
    if grep -qx serve "$example/ita.una" 2>/dev/null; then
        limited 600 "$tmp/step" "$una" build "$dir" --out "$dir/native-exe" || { cat "$tmp/step.err" >&2; exit 1; }
        continue
    fi
    if grep -qx drive "$example/ita.una" 2>/dev/null; then
        set -- --stop-after-iterations 5
    else
        set --
    fi
    expected=$(shown "$dir" 300 "$una" run . "$@") || true
    timed_out
    limited 600 "$tmp/step" "$una" build "$dir" --out "$dir/native-exe" || { cat "$tmp/step.err" >&2; exit 1; }
    native=$(shown "$dir" 120 ./native-exe "$@") || true
    timed_out
    if [ "$native" != "$expected" ]; then
        echo "error: the natively built $example printed \"$native\"; una run printed \"$expected\"" >&2
        exit 1
    fi
done
# No C math library symbol in what ships (#1092, #1953): on Linux, scripts/no-libm.sh reads the
# packed `una` by the trace of its own link (scripts/release.sh, scripts/link-trace.sh), which
# names each object that refers to a name of scripts/no-libm.txt, libstdc++.a's objects inside
# `una` included: only the objects of the Z3 library that supply-chain/z3.lock pins for this
# target that scripts/no-libm-una.txt names may. The runtime built into `una` and the examples'
# executables it built above are strict. The Z3 library is the cache entry whose digest is the
# lock's, as scripts/release.sh takes it, checked again here. The check's canaries run first.
case $target in
    *-linux-*)
        z3_digest=$(sed -n '/^\[library\]$/,$s/^"'"$target"'" *= *"\([0-9a-f]*\)"$/\1/p' supply-chain/z3.lock | head -n 1)
        [ -n "$z3_digest" ] || { echo "error: supply-chain/z3.lock records no library digest for $target" >&2; exit 1; }
        z3_lib=
        for recorded in "${UNA_Z3_CACHE:-$HOME/.cache/una/z3}"/*/library.sha256; do
            [ -f "$recorded" ] || continue
            if [ "$(cat "$recorded")" = "$z3_digest" ]; then
                z3_lib=$(dirname "$recorded")/lib/libz3shell.a
                break
            fi
        done
        [ -n "$z3_lib" ] && [ -f "$z3_lib" ] && [ "$(sha256sum "$z3_lib" | cut -d ' ' -f 1)" = "$z3_digest" ] \
            || { echo "error: no Z3 library of the cache has the digest $z3_digest that supply-chain/z3.lock records for $target" >&2; exit 1; }
        link_trace="$root/target/release-remapped/una.link-trace"
        [ -f "$link_trace" ] || { echo "error: $link_trace, the trace of the link of una that scripts/release.sh keeps, is missing" >&2; exit 1; }
        mkdir "$tmp/no-libm-executables"
        for exe in "$tmp"/native-*/native-exe; do
            [ -f "$exe" ] || continue
            cp "$exe" "$tmp/no-libm-executables/$(basename "$(dirname "$exe")")"
        done
        scripts/no-libm-test.sh
        UNA_Z3_LIB=$z3_lib scripts/no-libm.sh --link-trace "$link_trace" "$una" "$runtime" "$tmp/no-libm-executables"
        ;;
esac
# The release una reads no variable and no file for its runtime (ADR 0124 rules 14 and 15):
# UNA_RT_LIB naming a file that is no runtime, and such a file at each place a development build
# looks (lib/ beside una, beside una, ../lib), change nothing, so a runtime older than una cannot
# fail its link (#203).
cp -R examples/hello "$tmp/ignored"
mkdir -p "$tmp/ignored-install/bin/lib" "$tmp/ignored-install/lib"
cp "$una" "$tmp/ignored-install/bin/una"
for junk in "$tmp/ignored-install/bin/lib/libuna_rt.a" "$tmp/ignored-install/bin/libuna_rt.a" "$tmp/ignored-install/lib/libuna_rt.a" "$tmp/not-a-runtime.a"; do
    echo "not a runtime" > "$junk"
done
limited 600 "$tmp/step" env UNA_RT_LIB="$tmp/not-a-runtime.a" "$tmp/ignored-install/bin/una" build "$tmp/ignored" --out "$tmp/ignored/native-exe" \
    || { cat "$tmp/step.err" >&2; echo "error: the packed una read UNA_RT_LIB or a runtime file beside it (ADR 0124 rule 15)" >&2; exit 1; }
greeting=$(shown "$tmp/ignored" 120 ./native-exe) || true
timed_out
if [ "$greeting" != "Hello, Una" ]; then
    echo "error: examples/hello built with UNA_RT_LIB set printed \"$greeting\"" >&2
    exit 1
fi
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
# An executable that the packed `una` builds links the runtime built into it with this image's
# `cc` and glibc: it stays at or below the same glibc floor (#2105), so the runtime asks for
# nothing that the oldest supported system lacks.
if [ -n "$glibc_floor" ]; then
    if scripts/glibc-floor.sh 2.17 "$tmp/install-one/out/hello" > /dev/null 2>&1; then
        echo "error: the glibc floor scan accepts the executable of examples/hello at the floor 2.17; it cannot be trusted" >&2
        exit 1
    fi
    scripts/glibc-floor.sh "$glibc_floor" "$tmp/install-one/out/hello"
fi
# The builder's user name is refused too, in any form (`~name`, a home elsewhere), when it is long
# enough to mean something; inside Rust's own path, as a CI runner's user is, it is allowed.
user=$(id -un 2>/dev/null || true)
for file in "$una" "$runtime" "$tmp/install-one/out/hello"; do
    # The solver child's probes of an outer sandbox name the fixed places `/Users`, `/private/var`
    # and `/private/tmp` (crates/una-sys/src/solver/sandbox.rs), which the binary stores side by
    # side, so they read as `/Users/private/...`: a probe, not a home directory. Only a match made
    # of those places alone, whole, is skipped.
    found=$(grep -a -o -E '(/Users|/home)/[^[:space:][:cntrl:]"]*' "$file" | grep -v '^/Users/runner/work/rust/' | grep -v -x -E '/Users(/private/(var|tmp))+(\(temporary)?' | LC_ALL=C sort -u | head -5 || true)
    if [ "${#user}" -ge 4 ] && [ -z "$found" ]; then
        found=$(LC_ALL=C grep -a -o -E ".{0,16}$user.{0,16}" "$file" | grep -v -F '/Users/runner/work/rust' | LC_ALL=C sort -u | head -5 || true)
    fi
    if [ -n "$found" ] || grep -a -q -F "$root" "$file"; then
        echo "error: $file names a path of the machine that built it:" >&2
        echo "${found:-$root}" >&2
        exit 1
    fi
done
echo "ok: $name.tar.gz checks and runs the examples, and builds each of them natively with the runtime built into una"
