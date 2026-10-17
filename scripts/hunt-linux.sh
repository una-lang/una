#!/bin/sh
# The security hunt's gate on Linux (#793, docs/release-gates.md "The hunt gate"): the same
# fixed run as `python3 -m hunt gate` on macOS, in the image of scripts/local-ci.sh, where the
# container is the sandbox of the gate as a whole and Landlock confines each of its runs
# (scripts/security-hunt/hunt/landlock.py).
#
# Two containers, one after the other:
# 1. The build, as scripts/local-ci.sh runs its own: the release `una`, the pinned Z3 and Lean in
#    their volumes, the native runtime that `una build` links (`libuna_rt.a`, next to `una`), and
#    the re-check's executable (`lake build una-core-check`).
# 2. The gate, hardened: no network (`--network none`, loopback only), a read-only root with a
#    `tmpfs` for /tmp, the checkout and the tools mounted read-only, OUT the only writable mount,
#    the caller's user (never root), no capability (`--cap-drop ALL`), no-new-privileges, an init
#    process that reaps orphans (`--init`), limits
#    on processes and memory (`PIDS_LIMIT`, `MEMORY`) and on time (`TIMEOUT` seconds), and a
#    clean environment (`env -i`) with the marker that the gate's sandbox checks, among the
#    container's own conditions, before any run (`sandbox.container_problems`).
#
# OUT must be missing or empty; the gate writes its evidence there, summary.json included. The
# container sees the host's paths of its mounts (`/proc/self/mountinfo`), so the evidence is
# checked afterwards for this checkout's path and the caller's home, which fail the gate: no
# personal path is published with it.
# Docker is reached through the caller's environment (DOCKER_CONTEXT=desktop-linux on a Mac).
#
# Usage: scripts/hunt-linux.sh --linux-arm64|--linux OUT TIP
set -eu

usage() {
    echo "usage: scripts/hunt-linux.sh --linux-arm64|--linux OUT TIP" >&2
    exit 2
}
fail() {
    echo "hunt (Linux): FAIL ($1)" >&2
    exit 1
}

case ${1:-} in
    --linux-arm64) arch=arm64 suffix=-arm64 ;;
    --linux) arch=amd64 suffix="" ;;
    *) usage ;;
esac
[ "$#" -eq 3 ] || usage
out=$2
tip=$3
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
# The gate container's limits: the full gate takes about 4 minutes on macOS.
PIDS_LIMIT=1024
MEMORY=6g
TIMEOUT=1800

command -v docker >/dev/null 2>&1 || fail "docker not found"
if [ -e "$out" ] && [ -n "$(ls -A "$out")" ]; then
    fail "$out is not empty; the gate writes a fresh directory"
fi
mkdir -p "$out"
out=$(CDPATH='' cd -- "$out" && pwd)

toolchain=$(sed -n 's/^channel *= *"\([^"]*\)"$/\1/p' rust-toolchain.toml)
image="una-local-ci:$toolchain$suffix"
echo "== building the image $image (linux/$arch)"
docker build --platform "linux/$arch" --build-arg "TOOLCHAIN=$toolchain" \
    --tag "$image" scripts/local-ci || fail "the container image did not build"
memory=$(docker info --format '{{.MemTotal}}' 2>/dev/null)
cpus=$(docker info --format '{{.NCPU}}' 2>/dev/null)
jobs=$(scripts/container-jobs.sh "$memory" "$cpus" "${CARGO_BUILD_JOBS:-}" | sed -n 1p) \
    || fail "the container's job count"
# The volumes of scripts/local-ci.sh for this checkout and architecture, so that the two share
# their builds.
checkout=$(printf '%s' "$root" | (shasum -a 256 2>/dev/null || sha256sum) | cut -c1-12)
volume="una-local-ci-target-$checkout$suffix"

echo "== building the release una and the re-check in $image, with the target directory in $volume"
docker run --rm --platform "linux/$arch" \
    --volume "$root:/work" \
    --volume "$volume:/work/target" \
    --volume una-local-ci-registry:/opt/cargo/registry \
    --volume "una-local-ci-z3$suffix:/opt/z3" \
    --volume "una-local-ci-lean$suffix:/opt/lean" \
    --env UNA_FORMAL_LEAN=/opt/lean \
    --env "CARGO_BUILD_JOBS=$jobs" \
    "$image" sh -c '
        set -e
        [ -x /opt/z3/bin/z3 ] || scripts/install-z3.sh /opt/z3
        PATH=/opt/z3/bin:$PATH scripts/check-solver.sh
        cargo build --locked --release -p una-cli -p una-rt
        # The oracle runner (ADR 0115), in a build of its own, so that its `oracle` features
        # never reach the `una` the gate tests.
        cargo build --locked --release -p una-oracle
        root=/work
        . scripts/lean-env.sh
        (cd formal && lake build una-core-check)
        [ -x formal/.lake/build/bin/una-core-check ]' \
    || fail "the build of the release una or of the re-check"

echo "== running the hunt gate in $image, hardened (no network, read-only, user $(id -u))"
docker run --rm --platform "linux/$arch" \
    --init \
    --network none \
    --read-only \
    --tmpfs /tmp:rw,nosuid,nodev,noexec,size=64m \
    --cap-drop ALL \
    --security-opt no-new-privileges \
    --pids-limit "$PIDS_LIMIT" \
    --memory "$MEMORY" \
    --user "$(id -u):$(id -g)" \
    --volume "$root:/work:ro" \
    --volume "$volume:/work/target:ro" \
    --volume "una-local-ci-z3$suffix:/opt/z3:ro" \
    --volume "una-local-ci-lean$suffix:/opt/lean:ro" \
    --volume "$out:/out" \
    --workdir /work/scripts/security-hunt \
    "$image" timeout -k 30 "$TIMEOUT" env -i PATH=/usr/bin:/bin HOME=/tmp LANG=C.UTF-8 \
        UNA_HUNT_SANDBOX=container UNA_HUNT_READ=/opt/z3:/opt/lean \
        python3 -B -E -s -m hunt gate \
        --una /work/target/release/una --out /out --tip "$tip" \
        --core-check /work/formal/.lake/build/bin/una-core-check --z3 /opt/z3/bin/z3 \
        --runner /work/target/release/una-oracle \
    || fail "the gate failed; its output is above, and its evidence is in $out"
for path in "$root" "$HOME"; do
    status=0
    grep -rqF -- "$path" "$out" || status=$?
    case $status in
        0) fail "the evidence in $out holds the host path $path" ;;
        1) ;;
        *) fail "the scan of the evidence in $out for host paths failed" ;;
    esac
done
echo "hunt (Linux): PASS (linux/$arch)"
