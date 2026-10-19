#!/bin/sh
# Runs what .github/workflows/ci.yml runs, in the same order, on this machine: the solver check,
# the released sections of CHANGELOG.md against their tags, fmt, clippy with warnings denied,
# the tests, then packing and checking the release archive of the host's target. With --linux,
# it runs the same inside a linux/amd64 container of CI's Ubuntu image
# (scripts/local-ci/Dockerfile), emulated on an arm64 Mac, so it is slow and runs the tests one
# at a time (#286); with --linux-arm64, inside a linux/arm64 one, native on such a Mac (#241),
# with the tests in parallel. The target directory (one volume per checkout), the cargo registry
# and Z3 live in named volumes, so later runs are incremental. It ends with PASS, or FAIL and
# the step that failed, and a non-zero exit on any failure. While GitHub Actions is unavailable,
# this is the check before a merge (#156).
#
# Usage: scripts/local-ci.sh [--linux | --linux-arm64]
set -u

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

fail() {
    echo "local CI: FAIL ($1)" >&2
    exit 1
}

if [ "${1:-}" = --linux ] || [ "${1:-}" = --linux-arm64 ]; then
    # --linux: x86_64, emulated on an arm64 Mac; --linux-arm64: aarch64, native on one (#241).
    case $1 in
        --linux) arch=amd64 target=x86_64-unknown-linux-gnu suffix="" serial=1 ;;
        *) arch=arm64 target=aarch64-unknown-linux-gnu suffix=-arm64 serial="" ;;
    esac
    command -v docker >/dev/null 2>&1 || fail "docker not found; $1 needs Docker"
    # The released sections of CHANGELOG.md need the repository's tags, which a worktree
    # mounted alone into the container cannot reach: the check runs here, on the host, and the
    # container's run skips it, visibly (#645).
    echo "== Released sections of CHANGELOG.md (on the host)"
    env -u UNA_SKIP_RELEASED_SECTIONS scripts/check-released-sections.sh \
        || fail "Released sections of CHANGELOG.md"
    # The security advisories' releases (#649) need the same tags and history: their test runs
    # here, on the host, and only once it passes is the container told so, in
    # UNA_LOCAL_CI_ADVISORIES_ON_HOST; the container's run then checks the page alone (#715).
    echo "== Security advisories' releases (on the host)"
    cargo test --locked -p una-cli --test advisories the_advisories_match_the_releases -- --exact \
        || fail "Security advisories' releases (on the host)"
    # The documents' links (#708) need the files git tracks, which git cannot list there either:
    # they are checked here too, and only once they pass is the container told so, in
    # UNA_LOCAL_CI_DOC_LINKS_ON_HOST.
    echo "== Document links (on the host)"
    scripts/doc-links.py --self-test && scripts/doc-links.py || fail "Document links (on the host)"
    toolchain=$(sed -n 's/^channel *= *"\([^"]*\)"$/\1/p' rust-toolchain.toml)
    image="una-local-ci:$toolchain$suffix"
    echo "== building the image $image (linux/$arch)"
    docker build --platform "linux/$arch" --build-arg "TOOLCHAIN=$toolchain" \
        --tag "$image" scripts/local-ci || fail "the container image did not build"
    # The job count of the container's build (#701). Cargo uses every CPU of the Docker VM by
    # default, and that many rustc and linker processes at once can run the VM out of memory:
    # its kernel kills the linker (signal 9) and no test runs. The build runs one job per 2 GiB
    # of the VM's memory, at most one per CPU and at least one, and a CARGO_BUILD_JOBS set here
    # can lower that and never raise it (scripts/container-jobs.sh,
    # docs/release-gates.md). RUST_TEST_THREADS, when set here, is passed in too. Neither
    # changes what is built or tested, only the peak memory and the time.
    memory=$(docker info --format '{{.MemTotal}}' 2>/dev/null)
    cpus=$(docker info --format '{{.NCPU}}' 2>/dev/null)
    sized=$(scripts/container-jobs.sh "$memory" "$cpus" "${CARGO_BUILD_JOBS:-}") \
        || fail "the container's job count, from docker info and CARGO_BUILD_JOBS"
    jobs=$(printf '%s\n' "$sized" | sed -n 1p)
    why=$(printf '%s\n' "$sized" | sed -n 2p)
    echo "== the container's build runs $jobs jobs ($why)${RUST_TEST_THREADS:+, RUST_TEST_THREADS=$RUST_TEST_THREADS}"
    # The target directory is a volume of this checkout and architecture, named from its path,
    # so runs from two checkouts never share or overwrite one; Z3 has one volume per
    # architecture, and the cargo registry, which holds sources, one for both.
    checkout=$(printf '%s' "$root" | (shasum -a 256 2>/dev/null || sha256sum) | cut -c1-12)
    volume="una-local-ci-target-$checkout$suffix"
    echo "== running scripts/local-ci.sh in $image, with the target directory in $volume"
    # The checkout is mounted read-write, as CI checks it out; the target directory, the cargo
    # registry and Z3 are volumes, which keep the container's builds apart from the host's. The
    # release archive's entries take the commit's time (#679), which the container cannot read
    # when the checkout is a worktree, whose .git links outside the mount.
    docker run --rm --platform "linux/$arch" \
        --volume "$root:/work" \
        --volume "$volume:/work/target" \
        --volume una-local-ci-registry:/opt/cargo/registry \
        --volume "una-local-ci-z3$suffix:/opt/z3" \
        --env UNA_LOCAL_CI_Z3=/opt/z3 \
        --volume "una-local-ci-lean$suffix:/opt/lean" \
        --env UNA_FORMAL_LEAN=/opt/lean \
        --env "UNA_LOCAL_CI_SERIAL=$serial" \
        --env "CARGO_BUILD_JOBS=$jobs" \
        --env RUST_TEST_THREADS \
        --env UNA_LOCAL_CI_RELEASED_SECTIONS_ON_HOST=1 \
        --env UNA_LOCAL_CI_ADVISORIES_ON_HOST=1 \
        --env UNA_LOCAL_CI_DOC_LINKS_ON_HOST=1 \
        --env "SOURCE_DATE_EPOCH=$(git -C "$root" log -1 --format=%ct HEAD)" \
        "$image" scripts/local-ci.sh || fail "the Linux run failed; its output is above"
    echo "local CI: PASS ($target, in the container)"
    exit 0
fi
if [ "$#" -ne 0 ]; then
    echo "usage: scripts/local-ci.sh [--linux | --linux-arm64]" >&2
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
# Every released section of CHANGELOG.md is the one its tag published (#645): an entry that a
# rebase carried under a released heading fails here, before it merges. The run's evidence never
# skips it: UNA_SKIP_RELEASED_SECTIONS from the caller is dropped, and only the container, whose
# host checked the same tree before starting it, skips it, visibly.
if [ -n "${UNA_LOCAL_CI_RELEASED_SECTIONS_ON_HOST:-}" ] && [ -f /.dockerenv ]; then
    echo "== Released sections of CHANGELOG.md"
    echo "skipped: checked on the host before the container"
else
    step "Released sections of CHANGELOG.md" \
        env -u UNA_SKIP_RELEASED_SECTIONS scripts/check-released-sections.sh
fi
# Likewise the security advisories' releases (#715): the host ran their test with the tags
# before starting the container, which checks the page alone. Said here, since a passing test's
# own output is not shown.
if [ -n "${UNA_LOCAL_CI_ADVISORIES_ON_HOST:-}" ] && [ -f /.dockerenv ]; then
    echo "== Security advisories' releases"
    echo "skipped where git reads no tags: checked on the host before the container"
fi
step "Format" cargo fmt --all --check
step "Clippy" cargo clippy --workspace --all-targets --locked -- -D warnings
# Under emulation (--linux on an arm64 Mac, through Rosetta), the tests run one at a time, on
# one thread each: processes spawned from many threads at once can hang there (#286). The
# native runs, --linux-arm64 among them, keep full parallelism. The native differential
# (crates/una-cli/tests/native.rs) keeps a copy of every executable it links in
# UNA_NATIVE_KEEP, a directory it requires empty, for the check that follows (#1092).
keep="$root/target/no-libm-executables"
rm -rf "${keep:?}"
if [ -n "${UNA_LOCAL_CI_SERIAL:-}" ]; then
    step "Test (serial)" env RAYON_NUM_THREADS=1 UNA_NATIVE_KEEP="$keep" \
        cargo test --workspace --locked -- --test-threads=1
else
    step "Test" env UNA_NATIVE_KEEP="$keep" cargo test --workspace --locked
fi
# No C math library symbol (#1092): the `una` and `libuna_rt.a` that the tests built in
# target/debug, the runtime against scripts/no-libm-allow.txt, and every executable the native
# differential linked with them, strict.
step "No C math library symbol" \
    scripts/no-libm.sh "$root/target/debug/una" "$root/target/debug/libuna_rt.a" "$keep"
# The documents' links (#708): every link of the documents names a tracked file or directory,
# and every fragment a heading of its document; the check's own test refuses each kind of
# broken link first. It needs only Python 3 and git, so it runs everywhere, the containers
# included. In a container whose checkout git cannot read, a worktree's, only the self-test runs,
# and the documents are the host's check, said here.
if [ -n "${UNA_LOCAL_CI_DOC_LINKS_ON_HOST:-}" ] && [ -f /.dockerenv ] \
    && ! git ls-files --error-unmatch README.md >/dev/null 2>&1; then
    step "Document links (self-test)" scripts/doc-links.py --self-test
    echo "skipped: the documents, which git cannot list here; checked on the host before the container"
else
    step "Document links" sh -c 'scripts/doc-links.py --self-test && scripts/doc-links.py'
fi

# The security hunt's tests that need no compiler (#767): its generators, drivers, two-run oracle
# and release gate. Python runs with -B, so the checkout gets no bytecode files.
step "Security hunt (unit tests)" sh -c 'cd scripts/security-hunt && PYTHONDONTWRITEBYTECODE=1 python3 -B -m unittest discover -s tests'

# The formal track (docs/plan/formal-track.md §6, #46): the Lean development builds with the
# pinned toolchain, and no stated theorem depends on `sorry`.
step "Formal (Lean)" scripts/formal.sh

# The replay of the findings through the re-check (#528): S-01 to S-12, S-04 and S-05 keep their
# verdicts, compared byte for byte with the recorded output.
step "Formal replay" docs/reviews/formal-m2-replay/check.sh

# A release build never trusts the test certificate authority of una-http (ADR 0074, N-28).
step "Test (release, TLS trust)" cargo test --release --locked -p una-http --test tls
step "Pack the release archive of $target" scripts/release.sh "$target"
step "Check the release archive of $target" scripts/check-release.sh "$target"
# In a container, the most memory the run used, which sizes the job count (#701).
if [ -n "${UNA_LOCAL_CI_Z3:-}" ] && [ -r /sys/fs/cgroup/memory.peak ]; then
    echo "== peak memory of the container: $(($(cat /sys/fs/cgroup/memory.peak) / 1048576)) MiB, with CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-unset}"
fi
echo "local CI: PASS ($target)"
