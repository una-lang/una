# The shipped targets (#1903): sourced by the release scripts, never run. POSIX sh.
#
# release_targets FILE prints the targets that FILE (scripts/release-targets.txt) lists, one per
# line, in its order. It fails closed, with status 2 and nothing on stdout, when FILE cannot be
# read, lists no target, has a line that is not a bare target triple (spaces and carriage returns
# included), lists one twice, or lists one that release_target_platform does not know.
#
# release_target_platform TARGET prints where a target is built and checked: `macos` (natively,
# on the gate machine), `linux/arm64` or `linux/amd64` (in the container of scripts/local-ci/);
# status 1 for any other target.

release_target_platform() {
    case $1 in
        aarch64-apple-darwin) echo macos ;;
        aarch64-unknown-linux-gnu) echo linux/arm64 ;;
        x86_64-unknown-linux-gnu) echo linux/amd64 ;;
        *) return 1 ;;
    esac
}

release_targets() {
    if [ -z "${1:-}" ] || [ ! -f "$1" ] || [ ! -r "$1" ]; then
        echo "release targets: cannot read the list ${1:-(none named)}" >&2
        return 2
    fi
    _release_targets=$(awk '
        /^#/ || /^$/ { next }
        !/^[a-z0-9_]+(-[a-z0-9_]+)+$/ {
            printf "release targets: line %d of %s is not a target triple\n", NR, FILENAME > "/dev/stderr"
            bad = 1
            next
        }
        seen[$0]++ {
            printf "release targets: %s lists %s twice\n", FILENAME, $0 > "/dev/stderr"
            bad = 1
            next
        }
        { listed[++n] = $0 }
        END {
            if (bad) exit 2
            if (n == 0) {
                printf "release targets: %s lists no target\n", FILENAME > "/dev/stderr"
                exit 2
            }
            for (i = 1; i <= n; i++) print listed[i]
        }
    ' "$1") || return 2
    printf '%s\n' "$_release_targets" | while read -r _release_target; do
        release_target_platform "$_release_target" > /dev/null || {
            echo "release targets: $1 lists $_release_target, which no release script builds" >&2
            exit 2
        }
    done || return 2
    printf '%s\n' "$_release_targets"
}
