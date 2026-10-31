#!/bin/sh
# The toolchain-free gate of ADR 0124 rule 31, build phase for `una check` and `una test`
# (step 1): the release `una` alone, with no Z3, no C compiler, no linker and no Rust reachable,
# must check and test a package. On macOS it runs under `sandbox-exec` with a profile that
# denies starting any program but the `una` under test, the network and writes outside the
# run's own directory, and a PATH that holds only an empty directory, so the solver child takes
# the outer sandbox as its confinement (Question 6); each command runs on a fresh copy of the
# package, so that each starts the solver rather than reading the proof cache; on
# Linux it runs in a container of the bare Ubuntu image of scripts/local-ci/ with none of those
# tools installed, with `una` and the package mounted read-only and the same empty PATH. Any
# attempt to start another program fails the gate. The run phase (`una build`) comes with the
# linker steps of that record.
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
        profile="(version 1)(allow default)(deny network*)(deny file-write*)(allow file-write* (subpath \"$own\"))(allow file-write* (literal \"/dev/null\") (literal \"/dev/tty\") (literal \"/dev/dtracehelper\"))(deny process-exec*)(allow process-exec* (literal \"$una\"))"
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
            --env PATH=/nonexistent \
            --network none \
            "$image" sh -c '
                for tool in cc gcc clang ld z3 cargo rustc; do
                    if [ -x "/usr/bin/$tool" ] || [ -x "/usr/local/bin/$tool" ]; then
                        echo "error: the image has $tool"; exit 1
                    fi
                done
                cp -R /package /work && rm -rf /work/.una && cp -R /work /report && cd /work \
                    && /una --json check . > /tmp/check.json \
                    && /una --json test . > /tmp/test.json \
                    && cd /report && /una --json report . > /tmp/report.json \
                    && grep -q "\"solver_confinement\":\"own\"" /tmp/report.json' \
            || { echo "error: the toolchain-free run failed on linux/$arch" >&2; exit 1; }
        ;;
    *)
        echo "usage: scripts/toolchain-free.sh UNA PACKAGE [--linux-arm64 | --linux]" >&2
        exit 2
        ;;
esac
echo "toolchain-free: PASS"
