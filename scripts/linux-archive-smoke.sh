#!/bin/sh
# The Linux archive smoke (#1906, ops/coordination/releases/PLAN-LINUX-R0.4.md L7): what a user
# does with the aarch64-unknown-linux-gnu release archive, on clean linux/arm64 containers that
# hold no Rust, no Z3 and no checkout, with no network, as an ordinary user (uid 1000), each
# item of ops/coordination/releases/MUR-R0.4.md that applies to an archive:
#
#   install   install.sh installs the archive from a local mirror (file:// URLs): the archive,
#             and a stand-in validation summary that lists its digest (no signing key is read or
#             needed: the installer checks the digest the summary lists, nothing else)
#   refuse    install.sh refuses, before any download, a musl system (Alpine) and a glibc older
#             than the floor (Debian 9, glibc 2.24, below 2.28, and below 2.39 for 0.4.0 to
#             0.5.1), and installs nothing
#   loop      una --version; una check --tier 1, una test and una run on every package under
#             examples/; examples/health served under una run (GET /health is 200, exit 0 at
#             SIGTERM)
#   no-cc     una build without a C compiler stops with the documented "cannot run the system
#             linker `cc`", and una run still works afterwards
#   cc        with gcc as cc, una build of every package; each executable prints what una run
#             prints and exits with its status; the health executable serves
#   root      as root, una check refuses (U5003), as getting-started says (on examples/events,
#             which has obligations: a package with none, such as hello, starts no solver)
#   seccomp   the solver child's reads that the confinement answers with EACCES (#1455), seen
#             with strace on una check of examples/events when the container allows it; no
#             process may die of SIGSYS; the EACCES list is reported, not required
#   record    glibc of the container, the GLIBC_ symbol versions una needs, readelf -d of una
#             and of an executable it built, and the SHA-256 of the archive and of una
#
# The archive is the one scripts/release.sh packed (scripts/local-ci.sh --linux-arm64 leaves it
# in its target volume, target/release-archives/). The installer installs 0.4.0 and later only,
# so an archive of an earlier version is packed again under UNA_SMOKE_VERSION (default 0.4.0):
# only the name of its top directory changes, and the smoke checks that the installed una is
# byte for byte the archive's. UNA_SMOKE_VERSION, when set, also renames an archive of a later
# version: an archive of `main` carries the last release's version and is served as the next
# one, for the installer to apply that release's glibc floor (#2105).
#
# UNA_SMOKE_DISTRO picks the system of the install, loop, no-cc, cc, root and seccomp cases
# (#2105): ubuntu-24.04 (the default), ubuntu-22.04 (glibc 2.35), debian-12 (glibc 2.36) or
# almalinux-8 (glibc 2.28, the floor and the build image's base), each pinned by digest below.
#
# Usage: scripts/linux-archive-smoke.sh ARCHIVE OUT
#   ARCHIVE  una-<version>-aarch64-unknown-linux-gnu.tar.gz, with its .sha256 beside it
#   OUT      a directory that does not exist yet: logs, results.txt and the mirror go there
# Prints one line per item, `<case> PASS|FAIL|NOTE <what>`, also in OUT/results.txt, and exits
# 1 when any item failed. Needs Docker with linux/arm64 (native on an arm64 Mac: set
# DOCKER_CONTEXT as the other container scripts do). It builds two images from the pinned
# system of UNA_SMOKE_DISTRO (curl; and curl, gcc, the C library's development files, binutils,
# strace), which needs the network once; every smoke container runs with --network none. It
# creates no Docker volume.
#
# Inside a container the same file runs as: linux-archive-smoke.sh --inside CASE
set -eu

ubuntu=ubuntu:24.04@sha256:534baea6a22c03a63003dbc8dbe78fe34bc0d7e595d9a9dc9834884ff530eb55
ubuntu_old=ubuntu:22.04@sha256:076e7bf82319619f28dd0aa9c02ab34b055ad37100395b4f49a2ffba04fb5e11
debian12=debian:12@sha256:1d04200c2ece1d4d75a2a6f8e11ebce7d86a2ae28b34bb2c9cbb3ee9f4953e75
almalinux8=almalinux:8@sha256:768bedc30fe0a536adb0a65d2eddb7caad364e514fbddd0ae9e026975f590342
# Below every floor: glibc 2.24.
glibc_old=debian:9@sha256:c5c5200ff1e9c73ffbf188b4a67eb1c91531b644856b4aefe86a58d2f0cb05be
alpine=alpine:latest@sha256:be2656061b536c1883c4034790e21d0a09c5aaaf51968b89368a3c0371ea2c9a
target=aarch64-unknown-linux-gnu

# ---------------------------------------------------------------- inside a container
if [ "${1:-}" = --inside ]; then
    case=${2:?case}
    out=/out/$case
    mkdir -p "$out"
    res() { echo "$case $1 $2" | tee -a "$out/results.txt"; }
    W=$(mktemp -d /tmp/smoke.XXXXXX)
    export HOME="$W/home"
    mkdir -p "$HOME"
    version=$(cat /mirror/version)
    install_una() {
        code=0
        env UNA_HOME="$HOME/.una" UNA_VERSION="$version" \
            UNA_RELEASES=file:///mirror/releases UNA_RECORDS=file:///mirror/records \
            sh /smoke/install.sh > "$out/install.log" 2>&1 || code=$?
        echo "exit $code" >> "$out/install.log"
        return "$code"
    }
    environment() {
        {
            echo "id: $(id)"
            echo "uname: $(uname -srm)"
            echo "os: $(sed -n 's/^PRETTY_NAME=//p' /etc/os-release)"
            echo "getconf GNU_LIBC_VERSION: $(getconf GNU_LIBC_VERSION 2>&1 || true)"
            echo "ldd --version: $(ldd --version 2>&1 | head -n 1)"
            echo "cc: $(command -v cc || echo none)"
            echo "seccomp of this process: $(sed -n 's/^Seccomp:\t*//p' /proc/self/status)"
        } > "$out/environment.txt"
    }
    environment
    case $case in
    refuse-*)
        code=0
        env UNA_HOME="$HOME/.una" UNA_VERSION="$version" \
            UNA_RELEASES=file:///mirror/releases UNA_RECORDS=file:///mirror/records \
            sh /smoke/install.sh > "$out/install.log" 2>&1 || code=$?
        echo "exit $code" >> "$out/install.log"
        why=$(grep 'error:' "$out/install.log" | head -n 1)
        expected=$(cat /smoke-expect)
        if [ "$code" = 1 ] && printf '%s' "$why" | grep -qF -- "$expected" && [ ! -e "$HOME/.una" ]; then
            res PASS "install.sh refused with exit 1 and installed nothing: $why"
        else
            res FAIL "install.sh exit $code, expected 1 with \"$expected\" and no UNA_HOME: $why"
        fi
        exit 0
        ;;
    esac
    if ! install_una; then
        res FAIL "install.sh from the local mirror: $(tail -n 3 "$out/install.log" | tr '\n' ' ')"
        exit 0
    fi
    una=$HOME/.una/bin/una
    if cmp -s "$una" /mirror/una; then
        res PASS "install.sh installed una from the local mirror; it is the archive's una byte for byte ($(sha256sum "$una" | cut -d ' ' -f 1))"
    else
        res FAIL "the installed una differs from the archive's"
    fi
    # Bounded runs: STEP SECONDS COMMAND..., stdout in $out/STEP.out, stderr in $out/STEP.err.
    step() {
        name=$1
        secs=$2
        shift 2
        code=0
        timeout -k 5 "$secs" "$@" > "$out/$name.out" 2> "$out/$name.err" || code=$?
        return "$code"
    }
    # step_in DIR STEP SECONDS COMMAND...: the same, run in DIR.
    step_in() {
        dir=$1
        name=$2
        secs=$3
        shift 3
        code=0
        (cd "$dir" && exec timeout -k 5 "$secs" "$@") > "$out/$name.out" 2> "$out/$name.err" || code=$?
        return "$code"
    }
    first() { head -c 240 "$1" | tr '\n' ' '; }
    # serve_health WHAT DIR COMMAND...: serves the copy of examples/health in DIR, asks for
    # /health, stops it with SIGTERM; PASS when the answer is 200 and the exit 0.
    serve_health() {
        what=$1
        dir=$2
        shift 2
        (cd "$dir" && exec "$@") > "$out/serve.out" 2> "$out/serve.err" &
        pid=$!
        ticks=0
        until grep -q '^serving on 127.0.0.1:8080' "$out/serve.err"; do
            if [ "$ticks" -ge 300 ] || ! kill -0 "$pid" 2> /dev/null; then
                kill -9 "$pid" 2> /dev/null || true
                res FAIL "$what did not serve within 60 s: $(first "$out/serve.err")"
                return 0
            fi
            sleep 0.2
            ticks=$((ticks + 1))
        done
        answer=$(curl -s -m 20 -o /dev/null -w '%{http_code}' http://127.0.0.1:8080/health || true)
        kill -TERM "$pid"
        code=0
        wait "$pid" || code=$?
        if [ "$answer" = 200 ] && [ "$code" = 0 ]; then
            res PASS "$what served examples/health: GET /health 200, exit 0 at SIGTERM"
        else
            res FAIL "$what answered '$answer' for /health and exited $code"
        fi
    }
    kind() {
        if grep -qx serve "$1/ita.una" 2> /dev/null; then
            echo serve
        elif grep -qx drive "$1/ita.una" 2> /dev/null; then
            echo drive
        else
            echo plain
        fi
    }
    case $case in
    loop)
        if step version 30 env -i PATH=/usr/bin:/bin HOME="$HOME" "$una" --version \
            && grep -q '^editions:' "$out/version.out" && grep -q '^preview:' "$out/version.out" \
            && grep -q '^z3:' "$out/version.out"; then
            res PASS "una --version in a clean environment: $(tr '\n' ' ' < "$out/version.out")"
        else
            res FAIL "una --version: $(first "$out/version.out") $(first "$out/version.err")"
        fi
        [ -z "$(command -v cc || true)" ] || res FAIL "this container has a cc ($(command -v cc)); the no-cc case needs none"
        for example in /examples/*/; do
            p=$(basename "$example")
            d=$W/$p
            cp -R "$example" "$d"
            if step "check-$p" 600 "$una" check --tier 1 "$d"; then
                res PASS "una check --tier 1 $p"
            else
                res FAIL "una check --tier 1 $p (exit $code): $(first "$out/check-$p.out") $(first "$out/check-$p.err")"
            fi
            if step "test-$p" 600 "$una" test "$d"; then
                res PASS "una test $p: $(tail -n 1 "$out/test-$p.out")"
            else
                res FAIL "una test $p (exit $code): $(first "$out/test-$p.out") $(first "$out/test-$p.err")"
            fi
            case $(kind "$d") in
            serve) serve_health "una run $p" "$d" "$una" run . ;;
            drive)
                if step_in "$d" "run-$p" 300 "$una" run . --stop-after-iterations 5; then
                    res PASS "una run $p --stop-after-iterations 5"
                else
                    res FAIL "una run $p (exit $code): $(first "$out/run-$p.err")"
                fi
                ;;
            *)
                if step_in "$d" "run-$p" 300 "$una" run .; then
                    res PASS "una run $p: $(head -n 1 "$out/run-$p.out")"
                else
                    res FAIL "una run $p (exit $code): $(first "$out/run-$p.err")"
                fi
                ;;
            esac
            # Without cc, una build stops with the documented message (getting-started §6,
            # known-limits) and writes no executable.
            code=0
            step "build-$p" 600 "$una" build "$d" --out "$d/native-exe" || true
            if [ "$code" != 0 ] && grep -qF 'cannot run the system linker `cc`' "$out/build-$p.err" \
                && [ ! -e "$d/native-exe" ]; then
                res PASS "una build $p without cc fails (exit $code) with: $(grep -F 'cannot run the system linker' "$out/build-$p.err" | head -n 1)"
            else
                res FAIL "una build $p without cc: exit $code, expected the message 'cannot run the system linker \`cc\`': $(first "$out/build-$p.err")"
            fi
        done
        # The interpreter path still works after the failed builds.
        if step_in "$W/hello" run-after-build 300 "$una" run . \
            && [ "$(cat "$out/run-after-build.out")" = "Hello, Una" ]; then
            res PASS "una run hello after the failed builds prints \"Hello, Una\""
        else
            res FAIL "una run hello after the failed builds: $(first "$out/run-after-build.out") $(first "$out/run-after-build.err")"
        fi
        ;;
    cc)
        [ -n "$(command -v cc || true)" ] || { res FAIL "this container has no cc"; exit 0; }
        cc --version | head -n 1 > "$out/cc-version.txt"
        for example in /examples/*/; do
            p=$(basename "$example")
            d=$W/$p
            cp -R "$example" "$d"
            if ! step "build-$p" 600 "$una" build "$d" --out "$d/native-exe"; then
                res FAIL "una build $p with cc (exit $code): $(first "$out/build-$p.err")"
                continue
            fi
            case $(kind "$d") in
            serve)
                res PASS "una build $p with cc"
                serve_health "the executable una built for $p" "$d" ./native-exe
                continue
                ;;
            drive) set -- --stop-after-iterations 5 ;;
            *) set -- ;;
            esac
            code=0
            step_in "$d" "run-$p" 300 "$una" run . "$@" || true
            expected_code=$code
            code=0
            step_in "$d" "exe-$p" 120 ./native-exe "$@" || true
            if [ "$code" = "$expected_code" ] && cmp -s "$out/run-$p.out" "$out/exe-$p.out"; then
                res PASS "una build $p with cc: the executable prints what una run prints and exits $code"
            else
                res FAIL "una build $p with cc: the executable exited $code (una run $expected_code), output $(first "$out/exe-$p.out")"
            fi
        done
        {
            echo "== readelf -d una"
            readelf -d "$una"
            echo "== GLIBC_ versions una needs (readelf -V)"
            readelf -V "$una" | grep -o 'GLIBC_[0-9.]*' | sort -u -V
            echo "== readelf -d of the executable una built for hello"
            readelf -d "$W/hello/native-exe"
            echo "== GLIBC_ versions that executable needs"
            readelf -V "$W/hello/native-exe" | grep -o 'GLIBC_[0-9.]*' | sort -u -V
            echo "== file sizes"
            ls -l "$una" "$W/hello/native-exe"
        } > "$out/elf.txt" 2>&1
        needed=$(readelf -d "$una" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' | tr '\n' ' ')
        glibc_max=$(readelf -V "$una" | grep -o 'GLIBC_[0-9.]*' | sort -u -V | tail -n 1)
        exe_needed=$(readelf -d "$W/hello/native-exe" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' | tr '\n' ' ')
        res NOTE "una needs: $needed; highest symbol version $glibc_max; the hello executable needs: $exe_needed; $(cat "$out/cc-version.txt")"
        ;;
    root)
        cp -R /examples/events "$W/events"
        code=0
        step check-root 300 "$una" check --tier 1 "$W/events" || true
        if [ "$code" != 0 ] && grep -q U5003 "$out/check-root.out" "$out/check-root.err"; then
            res PASS "as root, una check refuses (exit $code, U5003): $(grep -h U5003 "$out/check-root.out" "$out/check-root.err" | head -n 1)"
        else
            res FAIL "as root, una check: exit $code, expected U5003: $(first "$out/check-root.out") $(first "$out/check-root.err")"
        fi
        if grep -q 'refuse to run as root' "$out/install.log"; then
            res PASS "install.sh says that una check refuses to run as root"
        else
            res FAIL "install.sh did not say that una check refuses to run as root"
        fi
        ;;
    seccomp)
        cp -R /examples/events "$W/events"
        code=0
        step strace-check 600 strace -f -q -e trace=open,openat,openat2 -e signal=none \
            -o "$out/strace.txt" "$una" check --tier 1 "$W/events" || true
        if [ "$code" != 0 ] && grep -q 'PTRACE\|ptrace' "$out/strace-check.err"; then
            res NOTE "strace cannot trace in this container (exit $code): $(first "$out/strace-check.err")"
            exit 0
        fi
        [ "$code" = 0 ] && res PASS "una check --tier 1 events under strace, as uid $(id -u)" \
            || res FAIL "una check --tier 1 events under strace: exit $code: $(first "$out/strace-check.out") $(first "$out/strace-check.err")"
        denied=$(grep 'EACCES' "$out/strace.txt" | sed -n 's/.*open[a-z0-9]*([^"]*"\([^"]*\)".*/\1/p' | sort -u | tr '\n' ' ')
        if [ -n "$denied" ]; then
            res NOTE "files the confinement answered with EACCES (#1455): $denied"
        else
            res NOTE "no open answered with EACCES under strace (the solver child's reads may differ under tracing)"
        fi
        if grep -q 'killed by SIGSYS\|SIGSYS' "$out/strace.txt"; then
            res FAIL "a process was killed by SIGSYS: $(grep SIGSYS "$out/strace.txt" | head -n 1)"
        fi
        ;;
    *)
        res FAIL "no case $case"
        ;;
    esac
    exit 0
fi

# ---------------------------------------------------------------- on the host
[ "$#" -eq 2 ] || { echo "usage: scripts/linux-archive-smoke.sh ARCHIVE OUT" >&2; exit 2; }
archive=$1
out=$2
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
self=$root/scripts/linux-archive-smoke.sh
fail() { echo "linux-archive-smoke: error: $*" >&2; exit 2; }
command -v docker > /dev/null 2>&1 || fail "docker not found"
[ -f "$archive" ] && [ -f "$archive.sha256" ] || fail "$archive and $archive.sha256 must both exist"
[ ! -e "$out" ] || fail "$out exists; name a new directory"
base=$(basename "$archive")
case $base in
    una-*-$target.tar.gz) ;;
    *) fail "$base is not una-<version>-$target.tar.gz" ;;
esac
name=${base%.tar.gz}
archive_version=${name#una-}
archive_version=${archive_version%-"$target"}
if command -v sha256sum > /dev/null 2>&1; then
    digest() { sha256sum "$1" | cut -d ' ' -f 1; }
else
    digest() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
fi
archive_sha=$(digest "$archive")
[ "$archive_sha" = "$(cut -d ' ' -f 1 "$archive.sha256")" ] || fail "$base does not match $base.sha256"
version=${UNA_SMOKE_VERSION:-0.4.0}
case $archive_version in
    0.[0-3].*) ;;
    *) [ -n "${UNA_SMOKE_VERSION:-}" ] || version=$archive_version ;;
esac
distro=${UNA_SMOKE_DISTRO:-ubuntu-24.04}
case $distro in
    ubuntu-24.04) system=$ubuntu packages=apt ;;
    ubuntu-22.04) system=$ubuntu_old packages=apt ;;
    debian-12) system=$debian12 packages=apt ;;
    almalinux-8) system=$almalinux8 packages=dnf ;;
    *) fail "UNA_SMOKE_DISTRO is ubuntu-24.04, ubuntu-22.04, debian-12 or almalinux-8, not $distro" ;;
esac

mkdir -p "$out/mirror/releases/v$version" "$out/mirror/records/v$version/docs/validation"
out=$(CDPATH='' cd -- "$out" && pwd)
mirror=$out/mirror
results=$out/results.txt
: > "$results"
echo "$version" > "$mirror/version"
cp "$archive" "$archive.sha256" "$out/"

# The images: the pinned system of UNA_SMOKE_DISTRO, with curl (no C compiler), and with gcc.
build_image() {
    tag=$1
    file=$2
    docker build --quiet --platform linux/arm64 --tag "$tag" - < "$file" > /dev/null \
        || fail "the image $tag did not build"
    docker image inspect --format "$tag {{.Id}}" "$tag" >> "$out/images.txt"
}
if [ "$packages" = apt ]; then
    cat > "$out/Dockerfile.base" << EOF
FROM $system
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends curl \
    && rm -rf /var/lib/apt/lists/*
EOF
else
    # AlmaLinux's base image has curl; the cases compare files with cmp (diffutils).
    cat > "$out/Dockerfile.base" << EOF
FROM $system
RUN dnf install --assumeyes --setopt=install_weak_deps=False diffutils tar gzip && dnf clean all
EOF
fi
base_tag=una-linux-smoke-base:$(digest "$out/Dockerfile.base" | cut -c1-12)
: > "$out/images.txt"
build_image "$base_tag" "$out/Dockerfile.base"
if [ "$packages" = apt ]; then
    cat > "$out/Dockerfile.cc" << EOF
FROM $base_tag
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    gcc libc6-dev binutils strace && rm -rf /var/lib/apt/lists/*
EOF
else
    cat > "$out/Dockerfile.cc" << EOF
FROM $base_tag
RUN dnf install --assumeyes --setopt=install_weak_deps=False gcc glibc-devel binutils strace \
    && dnf clean all
EOF
fi
cc_tag=una-linux-smoke-cc:$(digest "$out/Dockerfile.cc" | cut -c1-12)
build_image "$cc_tag" "$out/Dockerfile.cc"
for image in "$glibc_old" "$alpine"; do
    docker image inspect "$image" > /dev/null 2>&1 || docker pull --platform linux/arm64 "$image" > /dev/null \
        || fail "cannot pull $image"
    echo "$image" >> "$out/images.txt"
done
echo "distro $distro $system" >> "$out/images.txt"
if [ "$packages" = apt ]; then
    docker run --rm --network none --platform linux/arm64 "$cc_tag" \
        dpkg-query -W libc6 curl gcc libc6-dev binutils strace > "$out/packages.txt"
else
    docker run --rm --network none --platform linux/arm64 "$cc_tag" \
        rpm -q glibc curl gcc glibc-devel binutils strace > "$out/packages.txt"
fi

# The archive the mirror serves: the archive itself, or, for a version the installer refuses,
# the same files under the top directory una-$version-$target, packed in the base image.
published=una-$version-$target.tar.gz
docker run --rm --network none --platform linux/arm64 --user 1000:1000 \
    --volume "$out:/in:ro" --volume "$mirror:/mirror" "$base_tag" sh -c '
        set -eu
        cd "$(mktemp -d)"
        tar -xzf "/in/$1"
        cp "$2/una" /mirror/una
        if [ "$2" != "$3" ]; then
            mv "$2" "$3"
            tar --sort=name --owner=0 --group=0 --numeric-owner --mtime=@0 -czf "/mirror/releases/v$4/$3.tar.gz" "$3"
        else
            cp "/in/$1" "/mirror/releases/v$4/$3.tar.gz"
        fi' sh "$base" "$name" "${published%.tar.gz}" "$version" \
    || fail "preparing the mirror's archive"
published_sha=$(digest "$mirror/releases/v$version/$published")
una_sha=$(digest "$mirror/una")
# A stand-in for docs/validation/<version>.md: the two lines install.sh reads, the verdict and the
# archive's digest. It is no validation record and is never published.
cat > "$mirror/records/v$version/docs/validation/$version.md" << EOF
# Stand-in summary for the Linux archive smoke (scripts/linux-archive-smoke.sh), not a validation record

Verdict: PASS (every gate)

| Archive | SHA-256 |
|---|---|
| \`$published\` | \`$published_sha\` |
EOF
{
    echo "archive $base sha256 $archive_sha"
    echo "served as $published sha256 $published_sha (installer version $version)"
    echo "una sha256 $una_sha"
} > "$out/digests.txt"
echo "record PASS archive $base sha256 $archive_sha; una sha256 $una_sha; served to install.sh as $published ($published_sha)" | tee -a "$results"

# run_case CASE IMAGE USER [EXPECT]: the case in a fresh container, with no network.
run_case() {
    case=$1
    image=$2
    user=$3
    mkdir -p "$out/$case"
    printf '%s\n' "${4:-}" > "$out/$case/expect"
    code=0
    docker run --rm --network none --platform linux/arm64 --user "$user" \
        --volume "$self:/smoke/smoke.sh:ro" \
        --volume "$root/install.sh:/smoke/install.sh:ro" \
        --volume "$out/$case/expect:/smoke-expect:ro" \
        --volume "$mirror:/mirror:ro" \
        --volume "$root/examples:/examples:ro" \
        --volume "$out:/out" \
        "$image" sh /smoke/smoke.sh --inside "$case" > "$out/$case/console.log" 2>&1 || code=$?
    if [ -s "$out/$case/results.txt" ]; then
        cat "$out/$case/results.txt" | tee -a "$results"
    else
        echo "$case FAIL the container gave no result (exit $code): $(tail -n 3 "$out/$case/console.log" | tr '\n' ' ')" | tee -a "$results"
    fi
    [ "$code" = 0 ] || echo "$case FAIL the container exited $code" | tee -a "$results"
}
run_case refuse-musl "$alpine" 1000:1000 "this system's C library is musl"
run_case refuse-glibc "$glibc_old" 1000:1000 "this system has glibc 2.24"
run_case loop "$base_tag" 1000:1000
run_case cc "$cc_tag" 1000:1000
run_case root "$base_tag" 0:0
run_case seccomp "$cc_tag" 1000:1000
for case in loop cc seccomp; do
    [ -f "$out/$case/environment.txt" ] && sed "s/^/$case: /" "$out/$case/environment.txt"
done > "$out/environments.txt"
if grep -q '^[a-z-]* FAIL ' "$results"; then
    echo "linux-archive-smoke: FAIL ($(grep -c '^[a-z-]* FAIL ' "$results") items); $out/results.txt"
    exit 1
fi
echo "linux-archive-smoke: PASS; $out/results.txt"
