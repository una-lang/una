#!/bin/sh
# Runs what .github/workflows/ci.yml runs, in the same order, on this machine: the solver check,
# fmt, clippy with warnings denied, the tests, then packing and checking the release archive of
# the host's target. With --linux, it runs the same inside a linux/amd64 container of CI's
# Ubuntu image (scripts/local-ci/Dockerfile), emulated on an arm64 Mac, so it is slow; the
# target directory, the cargo registry and Z3 live in named volumes, so later runs are
# incremental. It ends with PASS, or FAIL and the step that failed, and a non-zero exit on any
# failure. While GitHub Actions is unavailable, this is the check before a merge (#156).
#
# Usage: scripts/local-ci.sh [--linux]
set -u

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

fail() {
    echo "local CI: FAIL ($1)" >&2
    exit 1
}

if [ "${1:-}" = --linux ]; then
    command -v docker >/dev/null 2>&1 || fail "docker not found; --linux needs Docker"
    toolchain=$(sed -n 's/^channel *= *"\([^"]*\)"$/\1/p' rust-toolchain.toml)
    image="una-local-ci:$toolchain"
    echo "== building the image $image (linux/amd64)"
    docker build --platform linux/amd64 --build-arg "TOOLCHAIN=$toolchain" \
        --tag "$image" scripts/local-ci || fail "the container image did not build"
    echo "== running scripts/local-ci.sh in $image"
    # The checkout is mounted read-write, as CI checks it out; the target directory, the cargo
    # registry and Z3 are volumes, which keep the container's builds apart from the host's.
    docker run --rm --platform linux/amd64 \
        --volume "$root:/work" \
        --volume una-local-ci-target:/work/target \
        --volume una-local-ci-registry:/opt/cargo/registry \
        --volume una-local-ci-z3:/opt/z3 \
        --env UNA_LOCAL_CI_Z3=/opt/z3 \
        "$image" scripts/local-ci.sh || fail "the Linux run failed; its output is above"
    echo "local CI: PASS (x86_64-unknown-linux-gnu, in the container)"
    exit 0
fi
if [ "$#" -ne 0 ]; then
    echo "usage: scripts/local-ci.sh [--linux]" >&2
    exit 2
fi

# The pinned Z3: in UNA_LOCAL_CI_Z3 inside the container, else target/z3, where
# scripts/install-z3.sh installs it for development (compiler/testing.md §3).
z3=${UNA_LOCAL_CI_Z3:-$root/target/z3}
if [ ! -x "$z3/bin/z3" ]; then
    echo "== installing the Z3 release pinned in verification.lock in $z3"
    scripts/install-z3.sh "$z3" || fail "installing Z3"
fi
PATH="$z3/bin:$PATH"
export PATH

target=$(rustc -vV | sed -n 's/^host: //p')
step() {
    name=$1
    shift
    echo "== $name"
    "$@" || fail "$name"
}
step "Check the solver version" scripts/check-solver.sh
step "Format" cargo fmt --all --check
step "Clippy" cargo clippy --workspace --all-targets --locked -- -D warnings
step "Test" cargo test --workspace --locked
step "Pack the release archive of $target" scripts/release.sh "$target"
step "Check the release archive of $target" scripts/check-release.sh "$target"
echo "local CI: PASS ($target)"
