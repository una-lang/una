#!/bin/sh
# The native legs of x86_64-unknown-linux-gnu on a GitHub-hosted x86-64 Linux runner (#2106,
# ADR 0124 rule 31 as its amendment of 2026-11-27 reads it, docs/release-gates.md §2 "The
# hosted x86-64 evidence"). The workflow .github/workflows/linux-x86-64.yml runs it, one leg per
# job, on `ubuntu-24.04`; it runs nowhere else that matters, and refuses a machine that is not
# x86-64 Linux, an emulated one, or one whose Docker engine is not x86-64.
#
# Legs:
#   z3-library    the Z3 library of supply-chain/z3.lock, built twice from cold in the release
#                 image (scripts/local-ci/Dockerfile.release, linux/amd64) by
#                 scripts/build-z3.sh --reproducible, which fails unless both builds have one
#                 digest; then the image's packages (`rpm -q`) and the digests of the files its
#                 linker puts into `una`, for the lock's tables of the target
#   linux         scripts/local-ci.sh --linux natively (the tests, the timing oracles, the archive
#                 packed and checked in the release image)
#   archive       the release archive as scripts/local-release.sh builds it, packed by
#                 scripts/release.sh and checked by scripts/check-release.sh in the release image,
#                 then run through scripts/toolchain-free.sh --linux
#   limits-linux  scripts/local-ci.sh --linux --limits natively: the solver limits and the solver
#                 child's confinement cases (scripts/limits.sh) against the shipped library
#
# Output, in OUT: LEG.log (everything the leg printed), LEG.manifest (below), for `archive` the
# archive and its .sha256 file, and SHA256SUMS over every other file. The manifest is lines
# `key=value`: leg, commit (the checkout's HEAD), result (PASS or FAIL), the runner's image
# (ImageOS, ImageVersion), the kernel (uname -srvm), the processor (its model name and count),
# the verdict of scripts/emulation.sh, the Docker engine's architecture, the run's URL and
# attempt, log_sha256, and per leg library_sha256 or archive and archive_sha256.
# scripts/hosted-evidence.sh reads it on the gate machine.
#
# Usage: scripts/hosted-x86-64.sh LEG OUT
set -u

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root" || exit 2

[ "$#" -eq 2 ] || { echo "usage: scripts/hosted-x86-64.sh z3-library|linux|limits-linux|archive OUT" >&2; exit 2; }
leg=$1 out=$2
case $leg in
    z3-library | linux | limits-linux | archive) ;;
    *) echo "usage: scripts/hosted-x86-64.sh z3-library|linux|limits-linux|archive OUT" >&2; exit 2 ;;
esac
mkdir -p "$out" || exit 2
out=$(CDPATH='' cd -- "$out" && pwd)
log=$out/$leg.log
manifest=$out/$leg.manifest
: > "$log"

say() { echo "$*" | tee -a "$log"; }
digest() { sha256sum "$1" | cut -d ' ' -f 1; }

# The machine, before anything runs: x86-64 Linux, natively, with an x86-64 Docker engine.
machine_fail=""
[ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ] || machine_fail="this is $(uname -sm), not x86_64 Linux"
emulation=native
scripts/emulation.sh >> "$log" 2>&1 || { emulation=emulated; machine_fail="scripts/emulation.sh does not find this machine native"; }
engine=$(docker info --format '{{.Architecture}}' 2>/dev/null) || engine=""
[ "$engine" = x86_64 ] || machine_fail="the Docker engine is ${engine:-unknown}, not x86_64"
cpu=$(sed -n 's/^model name[[:space:]]*: //p' /proc/cpuinfo 2>/dev/null | head -n 1)
commit=$(git rev-parse HEAD 2>/dev/null) || commit=""

say "== hosted x86-64 leg $leg on $commit"
say "== runner image ${ImageOS:-unknown} ${ImageVersion:-unknown}, kernel $(uname -srvm)"
say "== processor ${cpu:-unknown}, $(nproc) online, emulation $emulation, Docker engine ${engine:-unknown}"

result=FAIL
extra=""
toolchain=$(sed -n 's/^channel *= *"\([^"]*\)"$/\1/p' rust-toolchain.toml)
image="una-local-ci-release:$toolchain"

z3_library() {
    docker build --platform linux/amd64 --build-arg "TOOLCHAIN=$toolchain" \
        --file scripts/local-ci/Dockerfile.release --tag "$image" scripts/local-ci || return 1
    # The lock's package names and toolchain files, those of the aarch64 table (one list for
    # both architectures: the Dockerfile pins the same versions for each).
    packages=$(sed -n '/^\[toolchain\.aarch64-unknown-linux-gnu\]$/,/^\[/s/^"\([^"]*\)" *= *"[^"]*"$/\1/p' supply-chain/z3.lock)
    files=$(sed -n '/^\[toolchain-files\.aarch64-unknown-linux-gnu\]$/,/^\[/s/^"\([^"]*\)" *= *"[0-9a-f]*"$/\1/p' supply-chain/z3.lock)
    [ -n "$packages" ] && [ -n "$files" ] || { echo "supply-chain/z3.lock lists no toolchain"; return 1; }
    # A fresh cache in the container, as uid 1000 (the image's user), two cold builds.
    docker run --rm --platform linux/amd64 --user 1000:1000 --network bridge \
        --volume "$root:/work:ro" --env "PACKAGES=$packages" --env "FILES=$files" \
        "$image" sh -c '
            set -e
            cache=$(mktemp -d)
            scripts/build-z3.sh --reproducible --out "$cache"
            key=$(scripts/build-z3.sh --key)
            echo "library: x86_64-unknown-linux-gnu $(sha256sum "$cache/$key/lib/libz3shell.a" | cut -d " " -f 1)"
            sed -n "1,200p" "$cache/$key/build-record.txt"
            for package in $PACKAGES; do
                echo "package: \"$package\" = \"$(rpm -q --queryformat "%{VERSION}-%{RELEASE}" "$package")\""
            done
            for file in $FILES; do
                path=$(cc -print-file-name="$file")
                echo "file: \"$file\" = \"$(sha256sum "$path" | cut -d " " -f 1)\""
            done
            echo "glibc: $(getconf GNU_LIBC_VERSION)"' || return 1
}

# The volume of the checkout's target directory, named as scripts/local-ci.sh names it.
target_volume() {
    printf 'una-local-ci-target-%s' "$(printf '%s' "$root" | sha256sum | cut -c1-12)"
}

# After a failed test step of the linux leg, each failing test once more, alone, in the same
# container image and volumes, for the log only: it tells a test that fails on this machine from
# one that failed beside the others. The leg stays FAIL whatever these runs give.
rerun_alone() {
    failing=$(sed -n 's/^test \([^ ]*\) \.\.\. FAILED$/\1/p' "$log" | LC_ALL=C sort -u | head -n 12)
    suites=$(sed -n 's/^error: test failed, to rerun pass `\(.*\)`$/\1/p' "$log" | LC_ALL=C sort -u | head -n 4)
    [ -n "$failing" ] && [ -n "$suites" ] || return 0
    echo "== diagnostic only: the failed tests once more, alone (the leg stays FAIL)"
    printf '%s\n' "$suites" | while read -r suite; do
        for name in $failing; do
            echo "== diagnostic: $suite $name"
            # shellcheck disable=SC2086
            docker run --rm --platform linux/amd64 --user 1000:1000 \
                --volume "$root:/work" --volume "$(target_volume):/work/target" \
                --volume una-local-ci-registry:/opt/cargo/registry \
                --volume una-local-ci-z3:/opt/z3 --env UNA_LOCAL_CI_Z3=/opt/z3 \
                --volume una-local-ci-z3-cache:/opt/z3-cache --env UNA_Z3_CACHE=/opt/z3-cache \
                --env CARGO_PROFILE_DEV_OPT_LEVEL=0 \
                "una-local-ci:$toolchain" sh -c 'UNA_Z3_LIB=$(scripts/z3-library.sh) && export UNA_Z3_LIB && cargo test --locked '"$suite"' -- --exact "$1" --nocapture' sh "$name" \
                2>&1 | grep -v '^ *Compiling\|^ *Finished\|^ *Running' | tail -n 30
        done
    done
}

linux() {
    scripts/local-ci.sh --linux || { rerun_alone; return 1; }
}

# The release archive of this commit, as scripts/local-release.sh builds it on the gate machine
# (the release image, the checkout at /work, the target directory at /work/target, the commit's
# time), packed by scripts/release.sh, checked by scripts/check-release.sh natively, and then
# run by scripts/toolchain-free.sh --linux; volumes of its own, from cold. The packed archive's
# digest is printed before the check, so that a failed check still shows which bytes it judged;
# only a leg that passes names the archive in its manifest. The trace of the link of `una`
# (scripts/link-trace.sh, GNU ld's) is kept beside it, for the record (#2353).
archive() {
    docker build --platform linux/amd64 --build-arg "TOOLCHAIN=$toolchain" \
        --file scripts/local-ci/Dockerfile.release --tag "$image" scripts/local-ci || return 1
    version=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([^"]*\)"$/\1/p' Cargo.toml)
    name=una-$version-x86_64-unknown-linux-gnu.tar.gz
    # The container's user, uid 1000, writes the archive into OUT, which is the runner's.
    chmod a+rwx "$out" || return 1
    scripts/container-volumes.sh linux/amd64 "$image" una-hosted-target:/work/target \
        una-hosted-registry:/opt/cargo/registry una-hosted-z3-cache:/opt/z3-cache || return 1
    docker run --rm --platform linux/amd64 --user 1000:1000 \
        --volume "$root:/work" \
        --volume una-hosted-target:/work/target \
        --env "SOURCE_DATE_EPOCH=$(git log -1 --format=%ct HEAD)" \
        --volume una-hosted-registry:/opt/cargo/registry \
        --volume una-hosted-z3-cache:/opt/z3-cache --env UNA_Z3_CACHE=/opt/z3-cache \
        --volume "$out:/out" \
        "$image" sh -c "set -e
            scripts/z3-library.sh > /dev/null
            scripts/release.sh x86_64-unknown-linux-gnu
            cp target/release-archives/$name target/release-archives/$name.sha256 /out/
            cp target/release-remapped/una.link-trace /out/una.link-trace
            chmod 644 /out/$name /out/$name.sha256 /out/una.link-trace
            echo \"packed: \$(cat target/release-archives/$name.sha256)\"
            scripts/check-release.sh x86_64-unknown-linux-gnu" || return 1
    (cd "$out" && sha256sum --check "$name.sha256") || return 1
    unpacked=$(mktemp -d)
    tar -xzf "$out/$name" -C "$unpacked" || return 1
    scripts/toolchain-free.sh "$unpacked/una-$version-x86_64-unknown-linux-gnu/una" examples/events --linux \
        || return 1
}

limits_linux() {
    scripts/local-ci.sh --linux --limits || return 1
}

if [ -n "$machine_fail" ]; then
    say "hosted x86-64: FAIL ($machine_fail)"
else
    case $leg in
        z3-library) step=z3_library ;;
        linux) step=linux ;;
        limits-linux) step=limits_linux ;;
        archive) step=archive ;;
    esac
    # The leg's output goes to the log and to the job's own output as it comes.
    rm -f "$out/status"
    { "$step" 2>&1; echo "$?" > "$out/status"; } | tee -a "$log"
    if [ "$(cat "$out/status" 2>/dev/null)" = 0 ]; then
        result=PASS
    fi
    # What the leg made, read here: the step ran in the pipeline's subshell.
    if [ "$result" = PASS ] && [ "$leg" = z3-library ]; then
        library=$(sed -n 's/^library: x86_64-unknown-linux-gnu \([0-9a-f]\{64\}\)$/\1/p' "$log" | head -n 1)
        [ -n "$library" ] && extra="library_sha256=$library" || result=FAIL
    fi
    if [ "$result" = PASS ] && [ "$leg" = archive ]; then
        version=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([^"]*\)"$/\1/p' Cargo.toml)
        name=una-$version-x86_64-unknown-linux-gnu.tar.gz
        # The archive, and the trace of the link of its `una`, both readable by the runner (#2363).
        if [ -f "$out/$name" ] && [ -r "$out/$name" ] && [ -r "$out/una.link-trace" ]; then
            extra="archive=$name
archive_sha256=$(digest "$out/$name")"
        else
            result=FAIL
        fi
    fi
    rm -f "$out/status"
    say "hosted x86-64: $result ($leg on $commit)"
fi

{
    echo "leg=$leg"
    echo "commit=$commit"
    echo "result=$result"
    echo "image=${ImageOS:-unknown} ${ImageVersion:-unknown}"
    echo "kernel=$(uname -srvm)"
    echo "cpu=${cpu:-unknown}"
    echo "cpus=$(nproc)"
    echo "emulation=$emulation"
    echo "docker_architecture=${engine:-unknown}"
    echo "run=${GITHUB_SERVER_URL:-}/${GITHUB_REPOSITORY:-}/actions/runs/${GITHUB_RUN_ID:-} attempt ${GITHUB_RUN_ATTEMPT:-}"
    echo "log_sha256=$(digest "$log")"
    [ -n "$extra" ] && printf '%s\n' "$extra"
} > "$manifest"
# Every file's digest, or the leg fails (#2363): a file that sha256sum cannot read would be left
# out of SHA256SUMS, and its status is checked, not the pipeline's last command's.
rm -f "$out/sums.status"
(cd "$out" && find . -maxdepth 1 -type f ! -name SHA256SUMS ! -name sums.status ! -name '*.tar.gz' ! -name '*.tar.gz.sha256' \
    | sed 's|^\./||' | LC_ALL=C sort | { xargs sha256sum > SHA256SUMS; echo "$?" > sums.status; })
sums=$(cat "$out/sums.status" 2>/dev/null)
rm -f "$out/sums.status"
cat "$manifest"
if [ "$sums" != 0 ]; then
    echo "hosted x86-64: FAIL (the digests of $out: sha256sum exited ${sums:-without a status})"
    exit 1
fi
[ "$result" = PASS ]
