#!/bin/sh
# Runs what .github/workflows/ci.yml runs, in the same order, on this machine: the solver check,
# the released sections of CHANGELOG.md against their tags, fmt, clippy with warnings denied,
# the tests, the C math library check of release artifacts (#1092), then packing the release
# archive of the host's target, the load commands of its `una` and of a program it links
# (#1103), and checking the archive. With --linux,
# it runs the same inside a linux/amd64 container of CI's Ubuntu image
# (scripts/local-ci/Dockerfile), emulated on an arm64 Mac, so it is slow and runs the tests one
# at a time (#286), unless the Docker engine is x86-64, where the container is native and runs
# as --linux-arm64 does (#1939); with --linux-arm64, inside a linux/arm64 one, native on such a Mac (#241),
# with the tests in parallel. The target directory (one volume per checkout), the cargo registry,
# Z3 and the cache of the Z3 library built into una (ADR 0124, one volume per architecture,
# kept across fresh target volumes) live in named volumes, so later runs are incremental. It ends with PASS, or FAIL and
# the step that failed, and a non-zero exit on any failure. While GitHub Actions is unavailable,
# this is the check before a merge (#156).
#
# With --limits after --linux or --linux-arm64 (#1903), the container runs scripts/limits.sh alone,
# the release gates' limits leg of that architecture, and none of the steps above; alone, --limits
# runs scripts/limits.sh here.
#
# Usage: scripts/local-ci.sh [--linux | --linux-arm64] [--limits]
set -u

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

fail() {
    echo "local CI: FAIL ($1)" >&2
    exit 1
}

if [ "$*" = --limits ]; then
    exec scripts/limits.sh
fi
if [ "${1:-}" = --linux ] || [ "${1:-}" = --linux-arm64 ]; then
    # --linux: x86_64, emulated on an arm64 Mac; --linux-arm64: aarch64, native on one (#241).
    case $1 in
        # Emulated unless the native-host check below says otherwise (#1939).
        --linux) arch=amd64 target=x86_64-unknown-linux-gnu suffix="" serial=1 retry=1 ;;
        *) arch=arm64 target=aarch64-unknown-linux-gnu suffix=-arm64 serial="" retry="" ;;
    esac
    leg=$1 mode=""
    case "${2:-}" in
        "") [ "$#" -eq 1 ] || mode=usage ;;
        --limits) [ "$#" -eq 2 ] && mode=--limits || mode=usage ;;
        *) mode=usage ;;
    esac
    if [ "$mode" = usage ]; then
        echo "usage: scripts/local-ci.sh [--linux | --linux-arm64] [--limits]" >&2
        exit 2
    fi
    command -v docker >/dev/null 2>&1 || fail "docker not found; $leg needs Docker"
    # The native-host mode of --linux (#1939): the amd64 container is emulated, and marked so
    # (UNA_LOCAL_CI_SERIAL, which makes scripts/limits.sh refuse and the tests run one at a
    # time), unless the Docker engine itself is x86-64 (`docker info`, its Architecture exactly
    # x86_64) and this shell is not translated by Rosetta; a failed or empty answer keeps the
    # mark. Inside the container scripts/limits.sh checks again, on its own (scripts/emulation.sh),
    # so both must say native before the limits gate runs.
    if [ "$arch" = amd64 ]; then
        engine=$(docker info --format '{{.Architecture}}' 2>/dev/null) || engine=""
        translated=$( (sysctl -n sysctl.proc_translated 2>/dev/null) || echo 0)
        if [ "$engine" = x86_64 ] && [ "$translated" != 1 ]; then
            serial="" retry=""
            echo "== linux/amd64 runs natively here (Docker engine $engine)"
        else
            echo "== linux/amd64 is emulated here (Docker engine ${engine:-unknown}${translated:+, sysctl.proc_translated $translated}): tests one at a time, and the limits leg refuses"
        fi
    fi
    # The released sections of CHANGELOG.md need the repository's tags, which a worktree
    # mounted alone into the container cannot reach: the check runs here, on the host, and the
    # container's run skips it, visibly (#645).
    # The limits leg runs none of the steps, so none of their host parts either.
    if [ -z "$mode" ]; then
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
    fi
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
    # The container runs as the image's unprivileged user, uid 1000, never as root: the solver
    # child refuses root (ADR 0124 rule 7, #1394). Its volumes are handed to that user first, by a
    # step that runs as root and changes only the owner of entries another owner holds (a volume
    # made by a root container before). The checkout's bind mount needs no such step: Docker
    # Desktop lets the container's user write it.
    echo "== handing the volumes to the container's user (uid 1000)"
    scripts/container-volumes.sh "linux/$arch" "$image" \
        "$volume:/work/target" \
        una-local-ci-registry:/opt/cargo/registry \
        "una-local-ci-z3$suffix:/opt/z3" \
        "una-local-ci-z3-cache$suffix:/opt/z3-cache" \
        "una-local-ci-lean$suffix:/opt/lean" \
        || fail "handing the container's volumes to its user"
    # The pinned Z3 tarball for a cold cache volume (#1340, #1902): UNA_Z3_SOURCE on the host,
    # when set, names a local copy, which is mounted read-only and named to the container's
    # scripts/z3-library.sh the same way; build-z3.sh still checks it against supply-chain/z3.lock.
    # Set to anything but a readable file, the run fails here. Unset, the container takes the
    # library from the cache volume or downloads the tarball from the lock's mirror, and fails
    # when neither works: there is no build without the solver.
    z3_source=$(scripts/z3-source.sh) || fail "the Z3 source that UNA_Z3_SOURCE names"
    set --
    if [ -n "$z3_source" ]; then
        set -- --volume "$z3_source:/z3-src/z3-source.tar.gz:ro" \
            --env UNA_Z3_SOURCE=/z3-src/z3-source.tar.gz
    fi
    echo "== running scripts/local-ci.sh in $image, with the target directory in $volume"
    # The checkout is mounted read-write, as CI checks it out; the target directory, the cargo
    # registry and Z3 are volumes, which keep the container's builds apart from the host's. The
    # release archive's entries take the commit's time (#679), which the container cannot read
    # when the checkout is a worktree, whose .git links outside the mount. The Linux legs build the
    # development profile unoptimized, as the release gates do (#1697), unless the caller sets
    # CARGO_PROFILE_DEV_OPT_LEVEL.
    docker run --rm --platform "linux/$arch" \
        --user 1000:1000 \
        --volume "$root:/work" \
        --volume "$volume:/work/target" \
        --volume una-local-ci-registry:/opt/cargo/registry \
        --volume "una-local-ci-z3$suffix:/opt/z3" \
        --env UNA_LOCAL_CI_Z3=/opt/z3 \
        --volume "una-local-ci-z3-cache$suffix:/opt/z3-cache" \
        --env UNA_Z3_CACHE=/opt/z3-cache \
        "$@" \
        --volume "una-local-ci-lean$suffix:/opt/lean" \
        --env UNA_FORMAL_LEAN=/opt/lean \
        --env "UNA_LOCAL_CI_SERIAL=$serial" \
        --env "UNA_RELEASE_CHECK_RETRY=$retry" \
        --env "CARGO_BUILD_JOBS=$jobs" \
        --env "CARGO_PROFILE_DEV_OPT_LEVEL=${CARGO_PROFILE_DEV_OPT_LEVEL:-0}" \
        --env RUST_TEST_THREADS \
        --env UNA_LOCAL_CI_RELEASED_SECTIONS_ON_HOST=1 \
        --env UNA_LOCAL_CI_ADVISORIES_ON_HOST=1 \
        --env UNA_LOCAL_CI_DOC_LINKS_ON_HOST=1 \
        --env UNA_LOCAL_CI_PACK_IN_RELEASE_IMAGE=1 \
        --env "SOURCE_DATE_EPOCH=$(git -C "$root" log -1 --format=%ct HEAD)" \
        "$image" scripts/local-ci.sh $mode || fail "the Linux run failed; its output is above"
    if [ -z "$mode" ]; then
        # The release archive (#2105): packed and checked in the release image of
        # scripts/local-ci/Dockerfile.release (AlmaLinux 8, glibc 2.28), whose toolchain
        # supply-chain/z3.lock records, with the same target volume, registry and Z3 library
        # cache; the library of the lock is built there once into the cache when it is missing.
        release_image="una-local-ci-release:$toolchain$suffix"
        echo "== building the release image $release_image (linux/$arch)"
        docker build --platform "linux/$arch" --build-arg "TOOLCHAIN=$toolchain" \
            --file scripts/local-ci/Dockerfile.release --tag "$release_image" scripts/local-ci \
            || fail "the release image did not build"
        scripts/container-volumes.sh "linux/$arch" "$release_image" \
            "$volume:/work/target" \
            una-local-ci-registry:/opt/cargo/registry \
            "una-local-ci-z3-cache$suffix:/opt/z3-cache" \
            || fail "handing the release container's volumes to its user"
        echo "== packing and checking the release archive of $target in $release_image"
        docker run --rm --platform "linux/$arch" \
            --user 1000:1000 \
            --volume "$root:/work" \
            --volume "$volume:/work/target" \
            --volume una-local-ci-registry:/opt/cargo/registry \
            --volume "una-local-ci-z3-cache$suffix:/opt/z3-cache" \
            --env UNA_Z3_CACHE=/opt/z3-cache \
            "$@" \
            --env "UNA_LOCAL_CI_SERIAL=$serial" \
            --env "UNA_RELEASE_CHECK_RETRY=$retry" \
            --env "CARGO_BUILD_JOBS=$jobs" \
            --env "SOURCE_DATE_EPOCH=$(git -C "$root" log -1 --format=%ct HEAD)" \
            "$release_image" scripts/local-ci.sh --release-pack \
            || fail "the release archive in the release image; its output is above"
    fi
    echo "local CI: PASS ($target${mode:+, $mode}, in the container)"
    exit 0
fi
release_pack=""
if [ "$*" = --release-pack ] && [ -f /.dockerenv ]; then
    # The second container of a Linux leg (#2105), in the release image: the archive's steps
    # below and nothing else.
    release_pack=1
    shift
fi
if [ "$#" -ne 0 ]; then
    echo "usage: scripts/local-ci.sh [--linux | --linux-arm64] [--limits]" >&2
    exit 2
fi

# The Z3 built into una (ADR 0124 rules 1 and 19): scripts/z3-library.sh gives the library of
# supply-chain/z3.lock from the cache (UNA_Z3_CACHE; inside the container a named volume),
# verified against the digest its build recorded, or builds it once; every cargo step gets it
# by UNA_Z3_LIB. No z3 on PATH is needed or used by una any more.
UNA_Z3_LIB=$(scripts/z3-library.sh) || fail "the Z3 library of supply-chain/z3.lock"
export UNA_Z3_LIB

target=$(rustc -vV | sed -n 's/^host: //p')
step() {
    name=$1
    shift
    echo "== $name"
    "$@" || fail "$name"
}
pack() {
    step "Pack the release archive of $target" scripts/release.sh "$target"
    # The release `una` just packed, and examples/hello that it links, ask the dynamic loader
    # only for what scripts/load-commands.txt lists (#1103).
    step "Load commands" scripts/load-commands.sh "$root/target/release-remapped/release/una"
    # Under emulation the check runs the packed `una` with one verifier thread, as the serial
    # tests do (#286): a `una run` that starts several solver processes from many threads at once
    # can deadlock inside the translator (#1133). The check's own time limits and single retry
    # are a second guard.
    if [ -n "${UNA_LOCAL_CI_SERIAL:-}" ]; then
        step "Check the release archive of $target" env RAYON_NUM_THREADS=1 scripts/check-release.sh "$target"
    else
        step "Check the release archive of $target" scripts/check-release.sh "$target"
    fi
}
if [ -n "$release_pack" ]; then
    # The release `una` is built by scripts/release.sh, which unsets UNA_Z3_LIB and takes the
    # library by the lock's digest from the same cache, where the step above put it.
    pack
    echo "local CI: PASS ($target, the release archive, in the release image)"
    exit 0
fi
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
# native runs, --linux-arm64 among them, keep full parallelism.
if [ -n "${UNA_LOCAL_CI_SERIAL:-}" ]; then
    step "Test (serial)" env RAYON_NUM_THREADS=1 cargo test --workspace --locked -- --test-threads=1
else
    step "Test" cargo test --workspace --locked
fi
# The security timing oracles (#1145): every test of crates/una-cli/tests/timing_channels.rs is
# a real-clock oracle of a timing channel, ignored in the ordinary test run and run only here and
# in the release gate of the same name, one at a time. Under emulation (UNA_LOCAL_CI_SERIAL, the
# --linux container through Rosetta on an arm64 Mac, or a shell translated by Rosetta) the real
# clock measures the emulator, not the runtime, so the step is skipped there, visibly.
# With them, the cryptography's own timing evidence (#2119), in the same release build: the
# statistical timing tests of crates/una-crypto/tests/timing.rs (ADR 0133 rule 9), ignored in the
# ordinary run, and the scaling tests of crates/una-crypto/tests/perf.rs (ADR 0133 rule 10), which
# return at once in the debug build of the step "Test". They are skipped with the oracles.
translated=$( (sysctl -n sysctl.proc_translated 2>/dev/null) || echo 0)
if [ -n "${UNA_LOCAL_CI_SERIAL:-}" ] || [ "$translated" = 1 ]; then
    echo "== Timing channels (security oracles)"
    echo "skipped: under emulation the real clock is not the runtime's; run on a native host (macOS, --linux-arm64)"
    echo "== Timing channels (cryptography: timing and scaling)"
    echo "skipped: under emulation the real clock is not the runtime's; run on a native host (macOS, --linux-arm64)"
else
    step "Timing channels (security oracles)" \
        cargo test --release --locked -p una-cli --test timing_channels -- --ignored --test-threads=1 --nocapture
    step "Timing channels (cryptography: timing)" \
        cargo test --release --locked -p una-crypto --test timing -- --ignored --test-threads=1 --nocapture
    step "Timing channels (cryptography: scaling)" \
        cargo test --release --locked -p una-crypto --test perf -- --test-threads=1 --nocapture
fi
# No C math library symbol (#1092), in what ships: a test build unifies the features of
# dev-dependencies in, so the check builds its own release `una` and `libuna_rt.a`, links with
# that `una` every runnable program of tests/conformance/runtime that the native backend
# compiles (scripts/no-libm-build.sh, into a fresh target/no-libm), and reads the release
# graph's features and the undefined symbols of all of them (scripts/no-libm.sh), strictly.
step "Build (release) for the C math library check" \
    env CARGO_TARGET_DIR="$root/target" cargo build --locked --release -p una-cli -p una-rt
no_libm="$root/target/no-libm"
rm -rf "${no_libm:?}"
step "Native executables of the release una" \
    scripts/no-libm-build.sh "$root/target/release/una" "$no_libm"
step "No C math library symbol (canaries)" scripts/no-libm-test.sh
step "No C math library symbol" \
    scripts/no-libm.sh "$root/target/release/una" "$root/target/release/libuna_rt.a" \
    "$no_libm/executables" "$no_libm/una-without-solver"
# The x86-64 contraction canary (#1228; ADR 0114 Appendix C rule 7, ADR 0120 Appendix A): the
# float corpus's contraction-sensitive case, built with the shipped flags, passes the FMA gate of
# scripts/check-fma.sh, and built with contraction forced on it makes the gate fail. It runs on
# the --linux leg; on an AArch64 host it prints that it is not applicable.
step "FMA gate (contraction canary)" scripts/fma-canary.sh
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
step "Load commands (canaries)" scripts/load-commands-test.sh
# On Linux the archive is packed and checked in the release image (scripts/local-ci/
# Dockerfile.release, glibc 2.28, #2105), whose toolchain supply-chain/z3.lock records, in a
# second container that the host starts after this one, with the same target volume; this
# container, in the Ubuntu image of the tests, says so and packs nothing.
if [ -n "${UNA_LOCAL_CI_PACK_IN_RELEASE_IMAGE:-}" ] && [ -f /.dockerenv ]; then
    echo "== Pack and check the release archive of $target"
    echo "in the release image, in the next container"
else
    pack
fi
# In a container, the most memory the run used, which sizes the job count (#701).
if [ -n "${UNA_LOCAL_CI_Z3:-}" ] && [ -r /sys/fs/cgroup/memory.peak ]; then
    echo "== peak memory of the container: $(($(cat /sys/fs/cgroup/memory.peak) / 1048576)) MiB, with CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-unset}"
fi
echo "local CI: PASS ($target)"
