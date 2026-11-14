#!/bin/sh
# The toolchain-free gate of ADR 0124 rule 31, build phase for `una check` and `una test`
# (step 1): the release `una` alone, with no Z3, no C compiler, no linker and no Rust reachable,
# must check and test a package. On macOS it runs under `sandbox-exec` with a profile that
# denies starting any program but the `una` under test, the network and writes outside the
# run's own directory, and a PATH that holds only an empty directory, so the solver child takes
# the outer sandbox as its confinement (Question 6); each command runs on a fresh copy of the
# package, so that each starts the solver rather than reading the proof cache; on
# Linux it runs in a container of the bare Ubuntu image of scripts/local-ci/ with none of those
# tools installed, with `una` and the package mounted read-only and a PATH of /usr/bin:/bin,
# which holds the shell and no tool of the toolchain, as a user that is not root (the solver
# refuses to run as root, U5003) with a scratch tmpfs. Any
# attempt to start another program fails the gate. On macOS it then builds examples/hello with
# `una build` under the same profile, which links with the linker inside `una` (ADR 0124 step
# 3), and runs the executable under a profile that lets only it start; on Linux, the build phase
# comes with step 4.
#
# Usage: scripts/toolchain-free.sh UNA PACKAGE [--linux-arm64 | --linux]
#   UNA       the release `una` to test (a development build fails: it has no solver)
#   PACKAGE   a package to check and test, such as examples/events
set -eu

una=${1:?the una to test}
package=${2:?a package}
leg=${3:-}
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
una=$(CDPATH='' cd -- "$(dirname -- "$una")" && pwd)/$(basename -- "$una")
package=$(CDPATH='' cd -- "$package" && pwd)
empty=$(mktemp -d)
trap 'rm -rf "${empty:?}"' EXIT

# A development build has no solver: the gate must see the release `una`.
if "$una" --version | grep -q '^development build$'; then
    echo "error: $una is a development build; the gate needs the release una" >&2
    exit 1
fi

case $leg in
    "")
        case "$(uname -s)" in
            Darwin) ;;
            *) echo "error: without a --linux leg this runs on macOS only" >&2; exit 2 ;;
        esac
        # The profile: everything allowed but starting a program, which only `una` itself may
        # do (for its solver child, which is `una` again); the hunt gate's profile is the model.
        # It also denies what ADR 0124's Question 6 requires of an outer sandbox before the
        # solver child takes it as its confinement: the network, and writing anywhere but this
        # run's own directory, which holds the package copies, HOME (the proof cache key) and
        # TMPDIR (#1357); without that minimum the child fails with U5003.
        own=$(CDPATH='' cd -- "$empty" && pwd -P)
        mkdir -p "$own/home" "$own/tmp"
        profile="(version 1)(allow default)(deny network*)(deny signal (target others))(deny file-write*)(allow file-write* (subpath \"$own\"))(allow file-write* (literal \"/dev/null\") (literal \"/dev/tty\") (literal \"/dev/dtracehelper\"))(deny process-exec*)(allow process-exec* (literal \"$una\"))"
        # By its absolute path: with PATH holding only the empty directory, the shell would not
        # find it (#1357).
        sandbox=/usr/bin/sandbox-exec
        [ -x "$sandbox" ] || { echo "error: no $sandbox" >&2; exit 1; }
        echo "== toolchain-free (macOS): una check and una test under a sandbox that execs only una"
        # Each command on its own copy of the package with no proof cache, so that each starts
        # the solver: a report read from the cache of the check before it records no
        # confinement (#1357). `una` writes its cache beside the package, so the copies also
        # keep the tree as it was.
        for step in check test report; do
            cp -R "$package" "$empty/$step" && rm -rf "${empty:?}/$step/.una" || exit 1
        done
        HOME="$own/home" TMPDIR="$own/tmp" PATH="$empty" "$sandbox" -p "$profile" "$una" --json check "$empty/check" > "$empty/check.json" \
            || { echo "error: una check failed under the sandbox" >&2; cat "$empty/check.json" >&2; exit 1; }
        HOME="$own/home" TMPDIR="$own/tmp" PATH="$empty" "$sandbox" -p "$profile" "$una" --json test "$empty/test" > "$empty/test.json" \
            || { echo "error: una test failed under the sandbox" >&2; cat "$empty/test.json" >&2; exit 1; }
        # The solver ran under the outer sandbox, and the report says so (rule 28).
        HOME="$own/home" TMPDIR="$own/tmp" PATH="$empty" "$sandbox" -p "$profile" "$una" --json report "$empty/report" > "$empty/report.json" \
            || { echo "error: una report failed under the sandbox" >&2; exit 1; }
        grep -q '"solver_confinement":"outer"' "$empty/report.json" \
            || { echo "error: the report does not record the outer confinement" >&2; cat "$empty/report.json" >&2; exit 1; }
        # The build phase of `una build` (ADR 0124 step 3): `una` links examples/hello by
        # itself, under the same profile, which lets it start no linker and no compiler.
        echo "== toolchain-free (macOS): una build under the same sandbox"
        cp -R "$root/examples/hello" "$empty/hello" && rm -rf "${empty:?}/hello/.una" "${empty:?}/hello/target" || exit 1
        HOME="$own/home" TMPDIR="$own/tmp" PATH="$empty" "$sandbox" -p "$profile" "$una" build "$empty/hello" --out "$own/hello-exe" > "$empty/build.log" 2>&1 \
            || { echo "error: una build failed under the sandbox" >&2; cat "$empty/build.log" >&2; exit 1; }
        # The run phase: the executable alone may start, under a second profile.
        echo "== toolchain-free (macOS): the executable under a sandbox that execs only it"
        run_profile="(version 1)(allow default)(deny network*)(deny signal (target others))(deny file-write*)(allow file-write* (subpath \"$own\"))(allow file-write* (literal \"/dev/null\") (literal \"/dev/tty\"))(deny process-exec*)(allow process-exec* (literal \"$own/hello-exe\"))"
        output=$(HOME="$own/home" TMPDIR="$own/tmp" PATH="$empty" "$sandbox" -p "$run_profile" "$own/hello-exe") \
            || { echo "error: the executable failed under the sandbox" >&2; exit 1; }
        [ "$output" = "Hello, Una" ] || { echo "error: the executable printed: $output" >&2; exit 1; }
        ;;
    --linux | --linux-arm64)
        arch=amd64
        [ "$leg" = --linux-arm64 ] && arch=arm64
        image=$(sed -n 's/^FROM \(ubuntu:[^ ]*\).*/\1/p' "$root/scripts/local-ci/Dockerfile" | head -n 1)
        [ -n "$image" ] || { echo "error: no base image in scripts/local-ci/Dockerfile" >&2; exit 1; }
        echo "== toolchain-free (linux/$arch): una check and una test in $image with no tools"
        # Nothing but the image's own packages: no compiler, no linker, no Z3, no Rust. The
        # package is copied in, since `una` writes its cache beside it.
        docker run --rm --platform "linux/$arch" \
            --volume "$una:/una:ro" \
            --volume "$package:/package:ro" \
            --user 1000:1000 \
            --tmpfs /w:rw,exec,uid=1000,gid=1000,mode=700 \
            --env HOME=/w/home \
            --env TMPDIR=/w/tmp \
            --env PATH=/usr/bin:/bin \
            --network none \
            "$image" sh -c '
                # The PATH holds the shell and the OS basics, since the container runtime must
                # find `sh` on it (a PATH of /nonexistent fails the start, #2011), and the solver
                # refuses to run as root, so the container runs as user 1000; the checks
                # below make sure that it holds no compiler, linker, solver or Rust.
                for tool in cc c++ gcc g++ cpp clang ld ld.bfd ld.gold ld.lld lld as z3 cargo rustc rustup; do
                    if command -v "$tool" >/dev/null 2>&1; then
                        echo "error: $tool is on the PATH"; exit 1
                    fi
                    for dir in /usr/bin /bin /usr/local/bin /usr/sbin /sbin /usr/local/sbin /root/.cargo/bin /opt; do
                        if [ -e "$dir/$tool" ]; then
                            echo "error: the image has $dir/$tool"; exit 1
                        fi
                    done
                done
                mkdir /w/home /w/tmp && cp -R /package /w/work && rm -rf /w/work/.una && cp -R /w/work /w/report && cd /w/work \
                    && /una --json check . > /w/check.json \
                    && /una --json test . > /w/test.json \
                    && cd /w/report && /una --json report . > /w/report.json \
                    && grep -q "\"solver_confinement\":\"own\"" /w/report.json' \
            || { echo "error: the toolchain-free run failed on linux/$arch" >&2; exit 1; }
        ;;
    *)
        echo "usage: scripts/toolchain-free.sh UNA PACKAGE [--linux-arm64 | --linux]" >&2
        exit 2
        ;;
esac
echo "toolchain-free: PASS"
