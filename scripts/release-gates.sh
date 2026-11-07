#!/bin/bash
# The release gates (#232, docs/release-gates.md): the local, recorded run that replaces
# hosted CI for merges, tags and releases. It runs each gate on exactly one commit, in a fresh
# clone made for this run, and checks before every gate that the clone is still that commit
# with no change. A failing gate does not stop the next one.
#
# Gates, in order (all of them when none is named):
#   clone        a fresh clone of the commit, from the repository's remote
#   build        cargo build --release -p una-cli -p una-rt, then cargo test --workspace, cold,
#                then scripts/no-libm-build.sh and scripts/no-libm.sh (#1092): the release
#                `una` links the runtime conformance programs, and no C math library symbol
#                in the release `libuna_rt.a` or those executables, nor in that `una` beyond
#                the solver's imports of scripts/no-libm-una.txt (#1277)
#                then scripts/load-commands.sh (#1103): that `una` and examples/hello it links
#                load only what scripts/load-commands.txt lists, and `una` also what
#                scripts/load-commands-una.txt lists (the solver's C++ runtime, #1277);
#                each check after its canaries (scripts/no-libm-test.sh,
#                scripts/load-commands-test.sh) fail as they must
#   deps         scripts/deps-audit.sh (#770): cargo-deny at its pinned version against the
#                advisory database fetched now (advisories, licenses, bans, crates.io only), and
#                the supply-chain review record; the database's commit goes into the record
#   mac          scripts/local-ci.sh
#   linux        scripts/local-ci.sh --linux (Docker, linux/amd64)
#   linux-arm64  scripts/local-ci.sh --linux-arm64 (Docker, linux/arm64, #241)
#   release      scripts/local-release.sh for the three archives, each checked with
#                scripts/check-release.sh (Docker for the Linux ones)
#   reproducible scripts/local-release.sh a second time, from cold, in a new checkout and new
#                Docker volumes, and each archive's digest compared with the release gate's (#679)
#   determinism  the conformance suite twice, and `una --json check` of every case and example
#                twice, compared
#   native       the interpreter against the native executable, on every runnable program of
#                the repository (spec/16 §5): the equivalence test of crates/una-cli/tests/native.rs
#   timing-channels
#                the security timing oracles (#1145): `cargo test --release --locked -p una-cli --test
#                timing_channels -- --ignored --test-threads=1 --nocapture`, the real-clock tests that a timing
#                channel stays closed, which the ordinary test run ignores; it runs right after
#                `native`, with no Docker gate running, so the machine is as quiet as the run gets
#   owner-replay the exhaustive models of the replay of an owner's chain (spec/30 §14 rule 5)
#                and of yanks against it (spec/30 §15 rule 6), both to depth 5: `cargo test
#                --release --locked -p una-project --lib -- --ignored --exact
#                tlog::owner::model::owner_replay_holds_its_contract_to_depth_five --nocapture`,
#                then the same with registry::yank_model::yanks_hold_their_invariants_to_depth_five;
#                the ordinary test run checks both to depth 3, and the gate passes only when both
#                pass
#   sanitize     scripts/sanitize.sh (#766): Miri on the unit tests of una-rt, and the native
#                equivalence test against una-rt built with AddressSanitizer, with the checking
#                toolchain of scripts/check-toolchain.toml; macOS only for now
#   site         scripts/doc-links.py: its self-test, then every link of the repository's
#                documents names a tracked file and every fragment a heading (#708). Then (#584)
#                scripts/site-bundle.sh writes the docs bundle into OUT/site-bundle with the
#                clone's release `una`, and scripts/site-website.sh compares the heading ids with
#                the website's and builds the website that site/website.lock pins from the
#                bundle, with its budget and its code samples checked. When that website pins
#                una-lang/una-by-example (learn.lock, una#733), every example, its Try it
#                changes and its download archives are checked with the clone's `una`, and the
#                website must build its Learn pages from them. A missing Python 3, Node.js or
#                npm, or no access to the website's or the examples' repository, is a FAIL
#   formal       scripts/formal.sh: the Lean development of the formal track builds with its pinned
#                toolchain, and no stated theorem depends on `sorry` (#46)
#   hunt         the security hunt's fixed, seeded run (scripts/security-hunt, `python3 -m hunt
#                gate`): its generators through the must-reject and must-keep drivers, the
#                two-run oracle and the comparison with the formal model's re-check, with five
#                canaries, failing closed (#767, #791). It runs after `formal`, and builds the
#                re-check's executable (`lake build una-core-check`) with the same pinned Lean.
#                On macOS every compiler run is under sandbox-exec. Its summary is
#                OUT/hunt/summary.json
#   hunt-linux-arm64
#                the same fixed run on linux/arm64 (scripts/hunt-linux.sh, #793): the container
#                is the gate's sandbox (no network, read-only root, no capability, not root) and
#                Landlock confines each run. Its summary is OUT/hunt-linux-arm64/summary.json
#
# Output, in OUT: results.tsv (gate, sha, PASS or FAIL, seconds, sha256 of the gate's log),
# logs/<gate>.log, record.md, the full validation record, and, when the record names its
# version, summary.md, its public summary (scripts/release-summary.py, #717), and
# release-notes.md, the summary followed by the notes that the release gate wrote. All are
# described in docs/release-gates.md. The clone is OUT/clone.
#
# UNA_GATES_SOURCE overrides where the clone comes from (default: the remote `origin`), and
# UNA_GATES_Z3 where the pinned solver is (default: target/z3 of this checkout or of the clone).
# The Docker gates run on Docker Desktop's context only: run them with
# DOCKER_CONTEXT=desktop-linux (#1082, below).
# Exit code: 0 when no gate failed, 1 when one did, 129 or 143 after a HUP or TERM once the
# gate in progress has ended, 2 for a usage error, a refused Docker
# context, an OUT that another run holds, or a release tag of an earlier run that cannot be
# undone in the clone (#1017).
#
# Usage: scripts/release-gates.sh SHA OUT [GATE...]
set -u
# The test profile of the per-PR gate is opt-level 1 for speed (#1697); the release evidence keeps
# the unoptimized build it always had. Crates with their own opt-level in Cargo.toml (una-rt and
# una-http at 0, una-link, object and sha2 at 3) keep it: the variable sets the profile-wide value only.
export CARGO_PROFILE_DEV_OPT_LEVEL=0
# A trace would print the patterns below, and a startup file could read them: refuse a trace,
# whether `bash -x` or an exported SHELLOPTS turned it on (SHELLOPTS itself is read-only, and
# holds no xtrace once this check passes), drop the startup files the shells of the gates would
# read, and write no core file that could hold the patterns.
case $- in
    *x*)
        echo "release gates: refusing to run with xtrace (set -x), which would print the patterns" >&2
        exit 2
        ;;
esac
unset BASH_ENV ENV
ulimit -c 0

ALL=(clone build deps mac linux linux-arm64 release reproducible determinism native timing-channels owner-replay sanitize site formal hunt hunt-linux-arm64 solver-identity limits toolchain-free)

usage() {
    echo "usage: scripts/release-gates.sh SHA OUT [GATE...]; SHA is a full 40-digit commit, GATE is one of: ${ALL[*]}" >&2
    exit 2
}

[ $# -ge 2 ] || usage
SHA=$1
OUT=$2
shift 2
[[ "$SHA" =~ ^[0-9a-f]{40}$ ]] || usage
GATES=("$@")
[ ${#GATES[@]} -eq 0 ] && GATES=("${ALL[@]}")
for gate in "${GATES[@]}"; do
    case " ${ALL[*]} " in *" $gate "*) ;; *) usage ;; esac
done

REPO=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
# The release tag of the clone, made and undone by the release and reproducible gates (#1017).
. "$REPO/scripts/release-gates-tag.sh"
# Git reads the repository it is run in, never one an exported variable names, and no
# configuration but the machine's own files: every git step below, the clone gate's clone and
# the clone's tag among them (#1017), acts on the repository its -C or --git-dir says. The list,
# RELEASE_GATES_GIT_VARIABLES of that file, is the one its own git steps unset, with
# GIT_CONFIG_PARAMETERS, GIT_CONFIG_COUNT and every GIT_CONFIG_KEY_<n> and GIT_CONFIG_VALUE_<n>.
git_environment_unset

# The Docker backend (#1082): the Docker gates run on Docker Desktop's context desktop-linux,
# never on whichever context happens to be active, since another runtime on the same machine can
# hold it. When a requested gate needs Docker, the run refuses before any gate unless
# DOCKER_CONTEXT, or else the active context, is desktop-linux, and DOCKER_HOST, which would
# override the context, is unset. The context is then exported, so that every gate and its
# children use it even if the active context changes during the run. The record names the
# context and the server's version.
DOCKER_GATES=" linux linux-arm64 release reproducible hunt-linux-arm64 toolchain-free "
DOCKER_REQUIRED=desktop-linux
NEEDS_DOCKER=0
for gate in "${GATES[@]}"; do
    case "$DOCKER_GATES" in *" $gate "*) NEEDS_DOCKER=1 ;; esac
done
docker_context() {
    if [ -n "${DOCKER_CONTEXT:-}" ]; then
        printf '%s' "$DOCKER_CONTEXT"
    else
        docker context show 2> /dev/null
    fi
}
if [ $NEEDS_DOCKER = 1 ]; then
    if [ -n "${DOCKER_HOST:-}" ]; then
        echo "release gates: DOCKER_HOST is set, and it would override the Docker context; unset it and run with DOCKER_CONTEXT=$DOCKER_REQUIRED (#1082)" >&2
        exit 2
    fi
    context=$(docker_context)
    if [ "$context" != "$DOCKER_REQUIRED" ]; then
        echo "release gates: the Docker gates run on the context $DOCKER_REQUIRED, and the context is ${context:-unknown}; run with DOCKER_CONTEXT=$DOCKER_REQUIRED (#1082)" >&2
        exit 2
    fi
    export DOCKER_CONTEXT=$DOCKER_REQUIRED
fi

# The two patterns of this run (docs/release-gates.md §3): the privacy pattern of the public
# summary (#717) and the neutrality pattern of the public pages (ADR 0088, #789). Each comes
# from its environment variable when the caller set it, else from its macOS Keychain item. Each
# is held in a shell variable that is never exported, the caller's variable unset at once, and
# reaches the one step that applies it on a pipe (file descriptor 3), never in an environment
# or an argument, which other processes of the machine could read: the summary step for the
# privacy pattern, the site gate for the neutrality pattern. Neither is ever printed, logged or
# written. Each must be a valid extended regular expression (grep -E, the one dialect every
# check uses) that matches the canary `una-pattern-canary`, so that a pattern that matches
# nothing cannot pass for a check. Without either, the run stops before any gate (fail
# closed), unless UNA_GATES_UNCHECKED=1 asks for a run whose summary says it is unchecked and
# that prints no publish command, such as a dry run.
unset PRIVACY NEUTRALITY

# The pattern named $1 (privacy or neutrality), from $2 (the value of the variable $3) or the
# Keychain item una-$1-pattern, checked, into the variable named $4.
read_pattern() {
    local name=$1 value=$2 variable=$3 into=$4 status
    if [ -z "$value" ] && command -v security > /dev/null 2>&1; then
        value=$(security find-generic-password -s "una-$name-pattern" -w 2> /dev/null) || value=""
    fi
    if [ -z "$value" ]; then
        if [ "${UNA_GATES_UNCHECKED:-}" = 1 ]; then
            echo "release gates: no $name pattern; UNA_GATES_UNCHECKED=1, so this run is unchecked and prints no publish command" >&2
            printf -v "$into" '%s' ""
            return 0
        fi
        echo "release gates: no $name pattern: set $variable, or create the Keychain item una-$name-pattern; UNA_GATES_UNCHECKED=1 runs the gates without it, for a run that is not published" >&2
        exit 2
    fi
    # A value that cannot be a pattern of this project: blank, all hexadecimal digits (what
    # `security -w` prints for a value it cannot print as text), or longer than a pipe holds.
    if [ -z "${value//[[:space:]]/}" ] || [[ $value =~ ^[0-9A-Fa-f]+$ ]] || [ ${#value} -gt 60000 ]; then
        echo "release gates: the $name pattern is blank, hexadecimal or too long; set it again" >&2
        exit 2
    fi
    case $value in
        *una-pattern-canary*) ;;
        *)
            echo "release gates: the $name pattern does not name the canary una-pattern-canary (docs/release-gates.md §3)" >&2
            exit 2
            ;;
    esac
    printf 'una-pattern-canary\n' | grep -Eiq -f <(printf '%s\n' "$value") 2> /dev/null
    status=$?
    if [ "$status" = 2 ]; then
        echo "release gates: the $name pattern is not a valid extended regular expression (grep -E)" >&2
        exit 2
    elif [ "$status" != 0 ]; then
        echo "release gates: the $name pattern does not match the canary una-pattern-canary" >&2
        exit 2
    fi
    printf -v "$into" '%s' "$value"
}
read_pattern privacy "${UNA_PRIVACY_PATTERN:-}" UNA_PRIVACY_PATTERN PRIVACY
unset UNA_PRIVACY_PATTERN
read_pattern neutrality "${UNA_NEUTRALITY_PATTERN:-}" UNA_NEUTRALITY_PATTERN NEUTRALITY
unset UNA_NEUTRALITY_PATTERN
SOURCE=${UNA_GATES_SOURCE:-$(git -C "$REPO" remote get-url origin)}
mkdir -p "$OUT/logs"
# OUT by its physical path, every link resolved, so that two spellings of one directory (such as
# /tmp and /private/tmp on macOS) are one OUT to the clone's marker (#1017).
OUT=$(CDPATH='' cd -P -- "$OUT" && pwd -P)
# One run at a time on an OUT (#1017): a second run would truncate the first's results, see its
# release tag as a release, and undo that live tag at its start. The lock holds this run's pid,
# is released at its exit, and a lock whose run is gone (killed with SIGKILL) is taken over.
gates_lock "$OUT" || exit 2
trap 'gates_unlock "$OUT"' EXIT
trap 'exit 143' TERM
trap 'exit 129' HUP
CLONE=$OUT/clone
RESULTS=$OUT/results.tsv
printf 'gate\tsha\tresult\tseconds\tlog_sha256\n' > "$RESULTS"
# A tag that an earlier run made in this clone and could not undo, killed in the middle of the
# release or reproducible gate, is undone before any gate (#1017): a gate that reads tags would
# otherwise see the version as released. A run that cannot undo it stops here, unless its first
# gate is the clone gate, which makes the clone again.
if [ "${GATES[0]}" != clone ] \
    && { [ -e "$CLONE/.git/$RELEASE_GATES_PENDING" ] || [ -L "$CLONE/.git/$RELEASE_GATES_PENDING" ]; }; then
    clone_untag "$CLONE" "$OUT" || {
        echo "release gates: a release tag of an earlier run is still in $CLONE; run the clone gate first" >&2
        exit 2
    }
fi
# The pinned solver: UNA_GATES_Z3, else this checkout's target/z3 when it is there, else the
# clone's target/z3, which the build gate installs with scripts/install-z3.sh, as local-ci does
# (target/ is ignored, so the clone stays exact).
if [ -n "${UNA_GATES_Z3:-}" ]; then
    Z3=$UNA_GATES_Z3
elif [ -x "$REPO/target/z3/bin/z3" ]; then
    Z3=$REPO/target/z3
else
    Z3=$CLONE/target/z3
fi
export PATH="$Z3/bin:$PATH"
# A gated commit may be a past release's, older than tags that exist now: its released sections
# of CHANGELOG.md are checked against the tags it contains only (#645).
export UNA_RELEASED_SECTIONS_SCOPE=ancestors
# A gate's evidence never skips that check, nor the advisories' releases (#715), nor the
# documents' links (#708), whatever the caller's shell exports.
unset UNA_SKIP_RELEASED_SECTIONS UNA_LOCAL_CI_RELEASED_SECTIONS_ON_HOST UNA_LOCAL_CI_ADVISORIES_ON_HOST \
    UNA_LOCAL_CI_DOC_LINKS_ON_HOST
# Nor does the site gate build or compare anything but what the locks pin: the website and Una by
# Example from their repositories, and the Learn release as published (#831).
unset UNA_SITE_WEBSITE_SOURCE UNA_SITE_LEARN_SOURCE UNA_SITE_LEARN_RELEASE_DIR UNA_SITE_LEARN_UNA_DIR

digest() { shasum -a 256 "$1" | cut -d' ' -f1; }

record() { # gate result seconds
    local log=$OUT/logs/$1.log
    printf '%s\t%s\t%s\t%s\t%s\n' "$1" "$SHA" "$2" "$3" "$(digest "$log")" >> "$RESULTS"
    echo "== $1: $2 (${3}s)"
}

# Whether the clone is exactly SHA, with no change and no untracked file.
exact() {
    [ -d "$CLONE/.git" ] \
        && [ "$(git -C "$CLONE" rev-parse HEAD 2>/dev/null)" = "$SHA" ] \
        && [ -z "$(git -C "$CLONE" status --porcelain --untracked-files=all 2>/dev/null)" ]
}

# Runs gate $1 as the command that follows, in the clone, once it is checked to be exact.
run() {
    local gate=$1 log=$OUT/logs/$1.log start result=FAIL
    shift
    start=$(date +%s)
    if ! exact; then
        echo "the clone is not exactly $SHA with no change; run the clone gate first" > "$log"
    elif (
        cd "$CLONE" || exit 1
        # Only the site gate's command gets the neutrality pattern, on file descriptor 3 (a
        # pipe), and UNA_NEUTRALITY_PATTERN_FD says so; no other gate has that descriptor.
        if [ "$gate" = site ] && [ -n "$NEUTRALITY" ]; then
            UNA_NEUTRALITY_PATTERN_FD=3 "$@" 3< <(printf '%s\n' "$NEUTRALITY")
        else
            "$@"
        fi
    ) > "$log" 2>&1; then
        result=PASS
    fi
    record "$gate" "$result" $(($(date +%s) - start))
}

gate_clone() {
    local log=$OUT/logs/clone.log start result=FAIL
    start=$(date +%s)
    [ -e "$CLONE" ] && rm -rf "${CLONE:?}"
    {
        echo "source: $SOURCE"
        git clone --quiet --no-checkout "$SOURCE" "$CLONE" \
            && git -C "$CLONE" checkout --quiet --detach "$SHA" \
            && clone_mark "$CLONE" "$OUT" \
            && git -C "$CLONE" log -1 --format='commit %H%n%s'
    } > "$log" 2>&1
    if exact; then
        result=PASS
    else
        echo "the clone is not exactly $SHA with no change" >> "$log"
    fi
    record clone "$result" $(($(date +%s) - start))
}

gate_build() {
    # The Z3 library built into una (ADR 0124): from the cache, verified, or built once; every
    # cargo step of the clone gets it by UNA_Z3_LIB. The standalone z3 of scripts/install-z3.sh
    # serves the hunt gate alone.
    # Then no C math library symbol (#1092), in what ships: the release `una` just built links
    # every runnable program of tests/conformance/runtime that the native backend compiles
    # (scripts/no-libm-build.sh, into target/no-libm; target/ is ignored, so the clone stays
    # exact), and scripts/no-libm.sh reads the release graph's features and the undefined symbols
    # of the release `libuna_rt.a` and of those executables, strictly, and of that `una`, which
    # holds the solver, against scripts/no-libm-una.txt too (#1277). Each symbol and load
    # command check first runs its canaries, which must fail it.
    run build sh -c '
        UNA_Z3_LIB=$(scripts/z3-library.sh) || exit 1
        export UNA_Z3_LIB
        no_libm=$(pwd)/target/no-libm
        rm -rf "${no_libm:?}"
        cargo build --locked --release -p una-cli -p una-rt \
            && cargo test --workspace --locked \
            && scripts/no-libm-build.sh target/release/una "$no_libm" \
            && scripts/no-libm-test.sh \
            && scripts/no-libm.sh target/release/una target/release/libuna_rt.a "$no_libm/executables" "$no_libm/una-without-solver" \
            && scripts/load-commands-test.sh \
            && scripts/load-commands.sh target/release/una'
}

gate_solver-identity() {
    # ADR 0124 rule 31: the solver child against the standalone z3 built from the same library,
    # byte for byte, over every query of the repository's obligations, each twice; against the
    # Z3 project's own release (UNA_GATES_Z3_UPSTREAM, else the one scripts/install-z3.sh
    # installs) by status, listed for review. The log keeps the counts.
    run solver-identity sh -c '
        UNA_Z3_LIB=$(scripts/z3-library.sh) || exit 1
        export UNA_Z3_LIB
        UNA_Z3_STANDALONE=$(dirname "$(dirname "$UNA_Z3_LIB")")/bin/z3
        export UNA_Z3_STANDALONE
        if [ ! -x "'"$Z3"'/bin/z3" ]; then scripts/install-z3.sh "'"$Z3"'" || exit 1; fi
        UNA_Z3_UPSTREAM=${UNA_GATES_Z3_UPSTREAM:-'"$Z3"'/bin/z3}
        export UNA_Z3_UPSTREAM
        cargo test --release --locked -p una-cli --test solver_identity -- --ignored --nocapture'
}

gate_limits() {
    # ADR 0124 rule 31: memout, cpu and wallclock each end as their reason with the parent intact.
    run limits sh -c '
        UNA_Z3_LIB=$(scripts/z3-library.sh) || exit 1
        export UNA_Z3_LIB
        cargo test --release --locked -p una-cli --test solver_limits -- --ignored --nocapture'
}

gate_toolchain-free() {
    # ADR 0124 rule 31, the build phase for `una check` and `una test`: the release una alone,
    # under a sandbox that execs nothing but una (macOS) and in a bare container (linux/arm64).
    # The una under test is the one each archive of the release gate holds, which
    # scripts/release.sh built with the solver and the runtime inside it (ADR 0124 step 2,
    # #1357): the aarch64-apple-darwin archive's on macOS, the aarch64-unknown-linux-gnu
    # archive's in the container. Each archive is checked against its .sha256 file and unpacked
    # in OUT/toolchain-free, outside the clone; without the release gate's archives the gate
    # fails and says so.
    local version
    version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$CLONE/Cargo.toml" 2>/dev/null | head -1)
    run toolchain-free sh -c '
        version=$1 unpacked=$2
        built=target/local-release/v$version
        rm -rf "${unpacked:?}"
        mkdir -p "$unpacked" || exit 1
        # unpacked_una TARGET: the una of that archive, checked and unpacked; its path on stdout.
        unpacked_una() {
            name=una-$version-$1
            [ -f "$built/$name.tar.gz" ] && [ -f "$built/$name.tar.gz.sha256" ] \
                || { echo "no $built/$name.tar.gz; run the release gate first" >&2; return 1; }
            (cd "$built" && shasum -a 256 --check "$name.tar.gz.sha256" >&2) || return 1
            mkdir -p "$unpacked/$1" && tar -xzf "$built/$name.tar.gz" -C "$unpacked/$1" || return 1
            echo "$unpacked/$1/$name/una"
        }
        una=$(unpacked_una aarch64-apple-darwin) \
            && scripts/toolchain-free.sh "$una" examples/events \
            && una=$(unpacked_una aarch64-unknown-linux-gnu) \
            && scripts/toolchain-free.sh "$una" examples/events --linux-arm64' \
        sh "$version" "$OUT/toolchain-free"
}

gate_deps() { run deps scripts/deps-audit.sh; }

gate_mac() { run mac scripts/local-ci.sh; }

gate_linux() { run linux scripts/local-ci.sh --linux; }

gate_linux-arm64() { run linux-arm64 scripts/local-ci.sh --linux-arm64; }

gate_release() {
    # The archives are named after the workspace version, so the run tags it here, in the
    # clone only; the tag is never pushed, and it is undone when the gate ends, however it ends
    # (#1017): a later gate that reads tags, such as linux-arm64 in a run of its own on the same
    # OUT, must not see the version as released.
    local version tag
    version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$CLONE/Cargo.toml" 2>/dev/null | head -1)
    tag=v$version
    run release with_clone_tag "$CLONE" "$OUT" "$tag" "$SHA" \
        sh -c "git status --porcelain && scripts/local-release.sh '$tag'"
}

gate_reproducible() {
    # The release gate's archives, kept aside, against a second build of the same commit: a new
    # checkout (local-release.sh makes one per run, at a new path) and new Docker volumes for
    # the Linux target directories, so every artifact is built again from cold (#679). Its cost
    # is one more release gate. The release gate's archives are put back at the end, so the
    # record lists the ones the release gate built. The tag is made and undone as in the
    # release gate (#1017).
    local version tag
    version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$CLONE/Cargo.toml" 2>/dev/null | head -1)
    tag=v$version
    run reproducible with_clone_tag "$CLONE" "$OUT" "$tag" "$SHA" bash -c '
        set -u
        tag='"'$tag'"'
        first='"'$OUT'"'/reproducible-first
        again=una-local-release-again
        built=target/local-release/$tag
        ls "$built"/*.tar.gz > /dev/null 2>&1 || { echo "no archives in $built; run the release gate first"; exit 1; }
        rm -rf "${first:?}"
        cp -R "$built" "$first" || exit 1
        docker volume rm "$again" "$again-arm64" > /dev/null 2>&1
        UNA_LOCAL_RELEASE_VOLUME=$again scripts/local-release.sh "$tag"
        status=$?
        for archive in "$first"/*.tar.gz; do
            name=$(basename "$archive")
            one=$(shasum -a 256 "$archive" | cut -d" " -f1)
            two=$(shasum -a 256 "$built/$name" 2> /dev/null | cut -d" " -f1)
            echo "$name: first $one, second ${two:-missing}"
            [ "$one" = "$two" ] || status=1
        done
        docker volume rm "$again" "$again-arm64" > /dev/null 2>&1
        rm -rf "${built:?}"
        cp -R "$first" "$built"
        rm -rf "${first:?}"
        [ $status -eq 0 ] && echo "the archives are reproducible"
        exit $status'
}

gate_determinism() {
    run determinism bash -c '
        set -e
        cargo build --quiet -p una-cli
        # One test per case (#1652): no failure is the exit status of cargo test; the summary
        # compared between the runs is the case count, the sorted pending lines and the libtest
        # result line without its time.
        suite() {
            log=$(mktemp)
            if ! cargo test --quiet -p una-cli --test conformance -- --nocapture > "$log" 2>&1; then
                cat "$log" >&2
                echo "the conformance suite failed" >&2
                exit 1
            fi
            # libtest --quiet prints its dots on the line before a line a test prints.
            grep -o "conformance: [0-9]* cases" "$log"
            { grep -o "  pending M[0-9]*  .*" "$log" || true; } | LC_ALL=C sort
            grep "^test result: " "$log" | sed -E "s/; finished in .*//"
            rm -f "$log"
        }
        first=$(suite)
        second=$(suite)
        echo "conformance, run 1:"; echo "$first"
        echo "conformance, run 2:"; echo "$second"
        [ -n "$first" ]
        [ "$first" = "$second" ]
        hashes() {
            {
                find tests/conformance examples -name una.toml -exec dirname {} \;
                find tests/conformance -mindepth 2 -maxdepth 2 -type f -name "*.una"
            } | LC_ALL=C sort | while read -r target; do
                printf "%s %s\n" "$(./target/debug/una --json check "$target" | shasum -a 256 | cut -c1-16)" "$target"
            done
        }
        scratch=$(mktemp -d)
        hashes > "$scratch/check-1.txt"
        hashes > "$scratch/check-2.txt"
        echo "outputs compared: $(wc -l < "$scratch/check-1.txt")"
        diff "$scratch/check-1.txt" "$scratch/check-2.txt"
        test -z "$(git status --porcelain --untracked-files=no)"'
}

gate_native() {
    # The equivalence test of spec/16 §5: every runnable program of the repository (the
    # examples, the conformance cases that run, the review corpora), in the interpreter and as
    # a native executable, with the rules of spec/19 §8 rule 4 for tasks and the clock read
    # aside. One test per program (`interpreter::<name>`, #1652), with the tests that hold the
    # generated list to the programs on disk and the lists of tests/native to the programs, so
    # the gate and `cargo test` cannot drift apart.
    run native sh -c 'cargo test -p una-cli --test native -- interpreter:: the_generated_programs_are_the_runnable_programs_on_disk the_native_lists_name_runnable_programs_only --nocapture'
}

gate_hunt() {
    # The security hunt's fixed run (#767, docs/release-gates.md): the clone's release `una`,
    # every compiler run under sandbox-exec, the evidence in OUT/hunt, outside the clone. Python
    # runs with -B, so the clone gets no bytecode files and stays exact.
    local hunt=$OUT/hunt
    rm -rf "${hunt:?}"
    # The output directory and the commit go in as positional arguments, never spliced into the
    # script's text.
    # The re-check's executable comes from the pinned Lean, as scripts/formal.sh finds it; a
    # missing executable fails the gate.
    run hunt sh -c '
        scripts/check-solver.sh || exit 1
        [ -x target/release/una ] || cargo build --locked --release -p una-cli || exit 1
        una=$(pwd)/target/release/una
        root=$(pwd)
        . scripts/lean-env.sh || exit 1
        (cd formal && lake build una-core-check) || exit 1
        core_check=$(pwd)/formal/.lake/build/bin/una-core-check
        [ -x "$core_check" ] || { echo "no una-core-check at $core_check"; exit 1; }
        cd scripts/security-hunt && PYTHONDONTWRITEBYTECODE=1 python3 -B -m hunt gate \
            --una "$una" --out "$1" --tip "$2" --core-check "$core_check" --z3 "$3/bin/z3"' \
        sh "$hunt" "$SHA" "$Z3"
}

gate_hunt-linux-arm64() {
    # The hunt gate on linux/arm64 (#793): scripts/hunt-linux.sh builds the release `una` and
    # the re-check in the image of scripts/local-ci.sh, then runs the gate in a hardened
    # container, its evidence in OUT/hunt-linux-arm64.
    local hunt=$OUT/hunt-linux-arm64
    rm -rf "${hunt:?}"
    run hunt-linux-arm64 scripts/hunt-linux.sh --linux-arm64 "$hunt" "$SHA"
}

gate_site() {
    # The neutrality pattern reaches this one command on file descriptor 3 (`run`), never in
    # its environment or arguments; the command reads it into a variable, closes the
    # descriptor, and hands it to the bundle and the website steps each on a pipe of its own.
    run site sh -c '
        unset neutrality
        neutrality=
        if [ -n "${UNA_NEUTRALITY_PATTERN_FD:-}" ]; then
            neutrality=$(cat <&3)
            exec 3<&-
        fi
        unset UNA_NEUTRALITY_PATTERN_FD
        # Each step reads the pattern from its own pipe on descriptor 3, when there is one.
        with_pattern() {
            if [ -n "$neutrality" ]; then
                printf "%s\n" "$neutrality" | { UNA_NEUTRALITY_PATTERN_FD=3 "$@" 3<&0 </dev/null; }
            else
                "$@"
            fi
        }
        version=$(sed -n "s/^version = \"\(.*\)\"$/\1/p" Cargo.toml | head -n 1)
        scripts/doc-links.py --self-test \
            && scripts/site-learn-release.sh --self-test \
            && scripts/site-learn-una.sh --self-test \
            && scripts/doc-links.py \
            && with_pattern env UNA_BIN=target/release/una scripts/site-bundle.sh "'"$OUT"'/site-bundle" \
            && with_pattern env UNA_SITE_LEARN_SAME_VERSION=1 UNA_BIN=target/release/una scripts/site-website.sh "'"$OUT"'/site-bundle/una-docs-$version" "'"$OUT"'/site-website"'
}
gate_formal() { run formal scripts/formal.sh; }

gate_timing-channels() {
    # The security timing oracles (#1145, crates/una-cli/tests/timing_channels.rs): each test is
    # ignored in the ordinary run and runs only here and in the step of the same name of
    # scripts/local-ci.sh, one at a time, on the real clock of a native host.
    run timing-channels cargo test --release --locked -p una-cli --test timing_channels -- --ignored --test-threads=1 --nocapture
}

gate_owner-replay() {
    # The exhaustive model of the replay of an owner's chain (spec/30 §14 rule 5), then the
    # model of yanks against it (spec/30 §15 rule 6): both ignored in the ordinary run, which
    # checks each to depth 3, and both run here to depth 5; the gate passes only when both
    # pass. They are unit tests of una-project, since the model of signatures that stands in
    # for Ed25519 exists only in that crate's test build; `--release`, for their millions of
    # replays; `--nocapture`, so the log holds their counts.
    run owner-replay sh -c '
        cargo test --release --locked -p una-project --lib -- --ignored --exact tlog::owner::model::owner_replay_holds_its_contract_to_depth_five --nocapture &&
        cargo test --release --locked -p una-project --lib -- --ignored --exact registry::yank_model::yanks_hold_their_invariants_to_depth_five --nocapture
    '
}

# #766: Miri on the runtime's unit tests, and the native equivalence test against the runtime
# built with AddressSanitizer, by the checking toolchain of scripts/check-toolchain.toml, which
# builds no release artifact.
gate_sanitize() { run sanitize scripts/sanitize.sh; }

for gate in "${GATES[@]}"; do
    "gate_$gate"
done

# The Docker volume that local-ci made for this clone's target directory (named from its
# path), so that runs do not pile volumes up. Only a run that used Docker removes them, on the
# context it checked.
if [ $NEEDS_DOCKER = 1 ] && command -v docker > /dev/null 2>&1; then
    checkout=$(printf '%s' "$CLONE" | shasum -a 256 | cut -c1-12)
    docker volume rm "una-local-ci-target-$checkout" "una-local-ci-target-$checkout-arm64" \
        > /dev/null 2>&1 || true
fi

# The record.
{
    echo "# Validation record"
    echo
    echo "- Commit: \`$SHA\`"
    subject=$(git -C "$CLONE" log -1 --format=%s "$SHA" 2>/dev/null || echo unknown)
    echo "- Subject: $subject"
    echo "- Run: $(date -u +%Y-%m-%dT%H:%M:%SZ), $(uname -sm), $(rustc --version 2>/dev/null || echo 'rustc unknown'), $(docker --version 2>/dev/null || echo 'Docker unavailable')"
    if command -v docker > /dev/null 2>&1; then
        server=$(docker version --format '{{.Server.Version}}' 2> /dev/null) || server=""
        line="context ${context:-$(docker_context)}, server ${server:-unavailable}"
    else
        line="unavailable"
    fi
    [ $NEEDS_DOCKER = 1 ] || line="$line (no Docker gate ran)"
    echo "- Docker: $line"
    echo "- Script: \`scripts/release-gates.sh\` at \`$(git -C "$REPO" rev-parse HEAD 2>/dev/null || echo unknown)\`"
    echo
    echo "| Gate | Result | Seconds | Log sha256 |"
    echo "| --- | --- | --- | --- |"
    tail -n +2 "$RESULTS" | while IFS=$'\t' read -r gate _ result seconds log; do
        echo "| $gate | $result | $seconds | \`$log\` |"
    done
    echo
    echo "## Archives"
    echo
    archives=$(ls "$CLONE"/target/local-release/*/*.tar.gz 2>/dev/null)
    if [ -z "$archives" ]; then
        echo "None: the release gate did not run or did not pack them."
    else
        echo "| Archive | sha256 |"
        echo "| --- | --- |"
        for archive in $archives; do
            echo "| \`$(basename "$archive")\` | \`$(digest "$archive")\` |"
        done
    fi
    echo
    # The strip that removed the debug sections of each runtime built into una (#1359), as
    # scripts/release.sh printed it in the release gate's log: its version and its binary's digest.
    echo "## Runtime strip"
    echo
    strips=$(sed -n 's/^strip: \([a-z0-9_-]*\): \(.*\)$/- \1: \2/p' "$OUT/logs/release.log" 2>/dev/null | LC_ALL=C sort -u)
    if [ -z "$strips" ]; then
        echo "None: the release gate did not run or did not record the strip tool."
    else
        printf '%s\n' "$strips"
    fi
    echo
    echo "## Dependency audit"
    echo
    tool=$(sed -n 's/^tool: \(cargo-deny [0-9.]*\), sha256 [0-9a-f]*$/\1/p' "$OUT/logs/deps.log" 2>/dev/null)
    database=$(sed -n 's/^advisory-db: \([0-9a-f]\{40\}\) \(.*\)$/`\1` (\2)/p' "$OUT/logs/deps.log" 2>/dev/null)
    if [ -z "$tool" ] || [ -z "$database" ]; then
        echo "None: the deps gate did not run or did not fetch the advisory database."
    else
        echo "- Tool: $tool"
        echo "- Advisory database: $database"
    fi
    echo
    echo "## Hunt"
    echo
    if [ -f "$OUT/hunt/summary.json" ]; then
        python3 -B -c '
import json, sys
s = json.load(open(sys.argv[1]))
print("- Verdict: %s; seed %s, %s programs, una tip `%s`" % (s["verdict"], s["seed"], s["sizes"]["programs"], s["tip"]))
for key in ("reject", "keep", "oracle", "differ"):
    print("- %s: %s of %s cases judged" % (key, s["sizes"][key]["judged"], s["sizes"][key]["expected"]))
d = s["sizes"]["differ"]
print("- Re-check comparison: %s programs read clean, %s pinned unknown, %s residual" % (d["clean"], d["unknown"], d["residual"]))
o = s["sizes"]["oracle"]
print("- Oracle: %s programs and %s variants compared, %s live, %s programs set aside" % (o["programs_compared"], o["variants_compared"], o["live"], o["set_aside"]))
print("- Termination pairs (outside the noninterference guarantee, recorded): %s" % s["termination_pairs"]["count"])
print("- Canaries: " + ", ".join("%s %s" % (k, "caught" if v else "missed") for k, v in sorted(s["canaries"].items())))
print("- Findings: %s; errors: %s" % (len(s["findings"]), len(s["errors"])))
' "$OUT/hunt/summary.json" || echo "The hunt summary could not be read."
    else
        echo "None: the hunt gate did not run or did not write its summary."
    fi
    echo
    echo "## Docs bundle"
    echo
    bundles=$(ls "$OUT"/site-bundle/una-docs-*.tar.gz 2>/dev/null)
    if [ -z "$bundles" ]; then
        echo "None: the site gate did not run or did not write it."
    else
        echo "| Bundle | sha256 |"
        echo "| --- | --- |"
        for bundle in $bundles; do
            echo "| \`$(basename "$bundle")\` | \`$(digest "$bundle")\` |"
        done
        echo
        echo "Built into the website at \`$(sed -n 's/^commit = "\(.*\)"$/\1/p' "$CLONE/site/website.lock" 2>/dev/null)\` (\`site/website.lock\`)."
    fi
    echo
    failed=$(tail -n +2 "$RESULTS" | awk -F'\t' '$3 == "FAIL" { print $1 }' | sort -u | paste -sd ' ' - | sed 's/ /, /g')
    missing=()
    for gate in "${ALL[@]}"; do
        awk -F'\t' -v g="$gate" 'NR > 1 && $1 == g { found = 1 } END { exit !found }' "$RESULTS" || missing+=("$gate")
    done
    if [ -n "$failed" ]; then
        echo "Verdict: FAIL ($failed)"
    elif [ ${#missing[@]} -gt 0 ]; then
        echo "Verdict: INCOMPLETE (not run: $(printf '%s, ' "${missing[@]}" | sed 's/, $//'))"
    else
        echo "Verdict: PASS (every gate)"
    fi
} > "$OUT/record.md"

echo "record: $OUT/record.md"

# The public summary and the release notes (#717): the summary, then the version's CHANGELOG
# section that scripts/local-release.sh wrote. The publish command is printed only for a PASS
# whose summary passed the privacy check (docs/release-gates.md §4).
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$CLONE/Cargo.toml" 2>/dev/null | head -1)
rm -f "$OUT/summary.md" "$OUT/release-notes.md" "$OUT"/una-*-validation.md
# The privacy pattern goes to the summary step on descriptor 3; a run without the neutrality
# pattern is unchecked too, and its summary says so.
summary() {
    local neutrality=checked
    [ -n "$NEUTRALITY" ] || neutrality=unchecked
    if [ -n "$PRIVACY" ]; then
        UNA_PRIVACY_PATTERN_FD=3 UNA_SUMMARY_NEUTRALITY=$neutrality python3 -B "$REPO/scripts/release-summary.py" "$@" 3< <(printf '%s\n' "$PRIVACY")
    else
        UNA_SUMMARY_NEUTRALITY=$neutrality python3 -B "$REPO/scripts/release-summary.py" "$@"
    fi
}
if [ -n "$version" ] && summary "$version" "$OUT/record.md" "$OUT/summary.md"; then
    built=$CLONE/target/local-release/v$version
    if [ -f "$built/notes.md" ]; then
        { cat "$OUT/summary.md"; echo; cat "$built/notes.md"; } > "$OUT/release-notes.md"
        # Published only when both patterns checked this run.
        if grep -qx 'Verdict: PASS (every gate)' "$OUT/summary.md" && [ -n "$PRIVACY" ] && [ -n "$NEUTRALITY" ]; then
            cp "$OUT/summary.md" "$OUT/una-$version-validation.md"
            sh "$REPO/scripts/release-publish.sh" "$version" "$OUT" "$built"
        fi
    fi
fi
grep -q $'\tFAIL\t' "$RESULTS" && exit 1
exit 0
