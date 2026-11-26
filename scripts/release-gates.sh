#!/bin/bash
# The release gates (#232, docs/release-gates.md): the local, recorded run that replaces
# hosted CI for merges, tags and releases. It runs each gate on exactly one commit, in a fresh
# clone made for this run, and checks before every gate that the clone is still that commit
# with no change. A failing gate does not stop the next one.
#
# Gates, in order (the required ones when none is named: every gate of the gate machine, and the
# gates of each target that scripts/release-targets.txt lists, below):
#   clone        a fresh clone of the commit, from the repository's remote
#   build        cargo build --release -p una-cli -p una-rt, copied as the product into
#                OUT/product (#2254, `una` and `libuna_rt.a` with their SHA256SUMS), which then
#                checks every package of examples/ at tier 1 (so it holds the solver), then
#                cargo test --workspace, cold, then scripts/no-libm-build.sh and scripts/no-libm.sh (#1092): the release
#                `una` (the product) links the runtime conformance programs, and no C math library symbol
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
#   linux        scripts/local-ci.sh --linux (Docker, linux/amd64, emulated unless the Docker
#                engine is x86-64); required only when
#                x86_64-unknown-linux-gnu is shipped (#1903)
#   linux-arm64  scripts/local-ci.sh --linux-arm64 (Docker, linux/arm64, #241)
#   release      scripts/local-release.sh for the archive of each shipped target
#                (scripts/release-targets.txt, #1903), each checked with scripts/check-release.sh
#                (Docker for the Linux ones)
#   reproducible scripts/local-release.sh a second time, from cold, in a new checkout and new
#                Docker volumes, and each archive's digest compared with the release gate's (#679)
#   determinism  the conformance suite twice, and `una --json check` of every case and example
#                twice with the product's `una` (#2254), compared
#   native       the interpreter against the native executable, on every runnable program of
#                the repository (spec/16 §5): the equivalence test of crates/una-cli/tests/native.rs
#   timing-channels
#                the security timing oracles (#1145): `cargo test --release --locked -p una-cli --test
#                timing_channels -- --ignored --test-threads=1 --nocapture`, the real-clock tests that a timing
#                channel stays closed, which the ordinary test run ignores; then the statistical
#                timing tests of the cryptography (#2119, ADR 0133 rule 9): `cargo test --release
#                --locked -p una-crypto --test timing -- --ignored --test-threads=1 --nocapture`,
#                and its scaling tests, which do nothing in a debug build (ADR 0133 rule 10):
#                `cargo test --release --locked -p una-crypto --test perf -- --test-threads=1
#                --nocapture`; the three always run, and one that fails fails the gate; it runs
#                right after `native`, with no Docker gate running, so the machine is as quiet
#                as the run gets
#   owner-replay scripts/owner-replay-gate.sh (#1997, ADR 0122 Amendment 14): the replay of an
#                owner's chain (spec/30 §14 rule 5) against the oracle of its contract and
#                against the literal reference of the amendment, at every question, on every
#                chain of the bounded model to depth 4, to depth 5 on a narrow alphabet, on
#                seeded walks of depths 6 to 9 and on the named rows; the
#                release test binary run directly under a supervisor (a death by signal and
#                the 60-minute watchdog fail), one test each, with exact recorded counts; the
#                ordinary test run checks the same to depth 3
#   sanitize     scripts/sanitize.sh (#766): Miri on the unit tests of una-rt, and the native
#                equivalence test against una-rt built with AddressSanitizer, with the checking
#                toolchain of scripts/check-toolchain.toml; macOS only for now
#   docs         scripts/doc-links.py: its self-test, then every link of the repository's
#                documents names a tracked file and every fragment a heading (#708). Then (#584)
#                scripts/site-bundle.sh writes the docs bundle into OUT/site-bundle with the
#                product's `una` (#2254), its files checked against the neutrality pattern. The
#                gate was named `site` before #2081, when it also built the website from that
#                bundle; the website is now validated by its own repository's flow against a
#                published bundle, and no step of this run clones, installs or builds it. A
#                missing Python 3 is a FAIL
#   formal       scripts/formal.sh: the Lean development of the formal track builds with its pinned
#                toolchain, and no stated theorem depends on `sorry` (#46)
#   hunt         the security hunt's fixed, seeded run (scripts/security-hunt, `python3 -m hunt
#                gate`), with the product's `una` (#2254): its generators through the must-reject and must-keep drivers, the
#                two-run oracle and the comparison with the formal model's re-check, with five
#                canaries, failing closed (#767, #791). It runs after `formal`, and builds the
#                re-check's executable (`lake build una-core-check`) with the same pinned Lean.
#                On macOS every compiler run is under sandbox-exec. Its summary is
#                OUT/hunt/summary.json
#   hunt-linux-arm64
#                the same fixed run on linux/arm64 (scripts/hunt-linux.sh, #793): the container
#                is the gate's sandbox (no network, read-only root, no capability, not root) and
#                Landlock confines each run. Its summary is OUT/hunt-linux-arm64/summary.json
#   solver-identity, limits, toolchain-free
#                ADR 0124 rule 31 (docs/release-gates.md); limits-linux-arm64 and limits-linux are
#                the limits gate natively in the Linux container of that architecture (Docker)
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
# Exit code: the record's verdict (#1933): 0 for PASS (every gate), 1 for FAIL (a gate failed, or
# the archives are not exactly one per shipped target), 3 for INCOMPLETE (a required gate did not
# run); 129 or 143 after a HUP or TERM once the gate in progress has ended, 2 for a usage error,
# a gate in neither or both of the solver lists (SOLVER_GATES, #1996), a list of shipped targets that cannot be read, a refused Docker
# context, an OUT that another run holds, or a release tag of an earlier run that cannot be
# undone in the clone (#1017).
#
# The schedule (#2181, docs/release-gates.md §2 "The schedule"): with UNA_GATES_PARALLEL=1, the
# default, the sanitize gate runs in a clone of its own, OUT/clone-sanitize (its own target
# directory), beside the build and deps gates, and every other gate runs alone, in order, once
# sanitize has ended; UNA_GATES_PARALLEL=0 runs every gate in sequence, in one clone, as before.
# results.tsv lists the gates in the order they were named whatever the order they end in, and
# the record and the verdict are written once every gate has ended.
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

ALL=(clone build deps mac linux linux-arm64 release reproducible determinism native timing-channels owner-replay sanitize docs formal hunt hunt-linux-arm64 solver-identity limits limits-linux-arm64 limits-linux toolchain-free)

# The Z3 library built into una (ADR 0124, #1996). A development `una` that cargo builds without
# UNA_Z3_LIB has no solver, and every check of it is U5003: each gate that runs cargo or a
# cargo-built una on this machine is a solver gate, and `run` gives it UNA_Z3_LIB from
# scripts/z3-library.sh in the clone (the cache entry, verified against its recorded digest, or
# built once) before its command, or fails the gate, loudly, before any cargo step. The other
# gates build in a container that finds its own library (the Linux legs and hunt-linux-arm64),
# build with the embedded library alone (release and reproducible: scripts/release.sh unsets
# UNA_Z3_LIB), run the release archives' una (toolchain-free) or the build gate's (docs), or run
# no cargo at all (clone, formal). Every gate is in exactly one of the two lists, or the run
# stops before any gate: a new gate cannot run cargo without the library by omission.
SOLVER_GATES=" build deps mac determinism native timing-channels owner-replay sanitize hunt solver-identity limits "
NO_SOLVER_GATES=" clone linux linux-arm64 release reproducible docs formal hunt-linux-arm64 limits-linux-arm64 limits-linux toolchain-free "
for gate in "${ALL[@]}"; do
    in_solver=0 in_other=0
    case "$SOLVER_GATES" in *" $gate "*) in_solver=1 ;; esac
    case "$NO_SOLVER_GATES" in *" $gate "*) in_other=1 ;; esac
    if [ $((in_solver + in_other)) != 1 ]; then
        echo "release gates: the gate $gate must be in exactly one of SOLVER_GATES and NO_SOLVER_GATES (#1996)" >&2
        exit 2
    fi
done
for gate in $SOLVER_GATES $NO_SOLVER_GATES; do
    case " ${ALL[*]} " in
        *" $gate "*) ;;
        *)
            echo "release gates: $gate of SOLVER_GATES or NO_SOLVER_GATES is not a gate (#1996)" >&2
            exit 2
            ;;
    esac
done
# No solver library of the caller's shell reaches a gate: a solver gate gets the verified one, and
# the others none (a release build refuses UNA_Z3_LIB beside UNA_EMBED_Z3).
unset UNA_Z3_LIB UNA_Z3_STANDALONE UNA_EMBED_Z3 UNA_EMBED_Z3_SHA256

usage() {
    echo "usage: scripts/release-gates.sh SHA OUT [GATE...]; SHA is a full 40-digit commit, GATE is one of: ${ALL[*]}" >&2
    exit 2
}

REPO=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
# The shipped targets (scripts/release-targets.txt, #1903), whose archives the reproducible and
# toolchain-free gates take; a list that cannot be read or lists no target stops the run here.
. "$REPO/scripts/release-targets.sh"
TARGETS=$(release_targets "$REPO/scripts/release-targets.txt") || exit 2
# The required gates (#1903): those of the gate machine, an arm64 Mac, and those of each shipped
# target; a target with no gates here stops the run before any gate. Adding
# x86_64-unknown-linux-gnu to the list makes `linux` and `limits-linux` required, and the latter
# refuses to run under emulation (ADR 0124 rule 31).
HOST_GATES="clone build deps mac release reproducible determinism native timing-channels owner-replay sanitize docs formal hunt solver-identity limits toolchain-free"
TARGET_GATES=""
while read -r target; do
    case $target in
        aarch64-apple-darwin) ;;
        aarch64-unknown-linux-gnu) TARGET_GATES="$TARGET_GATES linux-arm64 hunt-linux-arm64 limits-linux-arm64" ;;
        x86_64-unknown-linux-gnu) TARGET_GATES="$TARGET_GATES linux limits-linux" ;;
        *)
            echo "release gates: no gates for the shipped target $target" >&2
            exit 2
            ;;
    esac
done <<< "$TARGETS"
REQUIRED=()
for gate in "${ALL[@]}"; do
    case " $HOST_GATES $TARGET_GATES " in *" $gate "*) REQUIRED+=("$gate") ;; esac
done

[ $# -ge 2 ] || usage
SHA=$1
OUT=$2
shift 2
[[ "$SHA" =~ ^[0-9a-f]{40}$ ]] || usage
GATES=("$@")
[ ${#GATES[@]} -eq 0 ] && GATES=("${REQUIRED[@]}")
for gate in "${GATES[@]}"; do
    case " ${ALL[*]} " in *" $gate "*) ;; *) usage ;; esac
done

# The schedule (#2181). Only a gate whose result does not depend on the machine's load, and that
# can have a clone and a target directory of its own, runs beside others: sanitize (Miri, a
# fixed count of fuzz inputs, the native equivalence test under AddressSanitizer, one test at a
# time), in OUT/clone-sanitize, beside build and deps, which read nothing it writes. It reads no
# tag, so the release tag that only release and reproducible make (after it ends) cannot reach
# it. Every other gate runs alone: mac and linux-arm64 run the timing oracles of
# timing_channels.rs and the cryptography's timing tests (local-ci.sh), as timing-channels does;
# the hunt gates judge each compiler run against a timeout; limits and its legs measure memory,
# CPU and wall-clock limits; owner-replay runs under a watchdog; solver-identity compares the
# solver's answers, which a time limit can change; and the Docker gates share one target volume
# per architecture. Nothing else changes: each gate runs the same command on the same commit.
case ${UNA_GATES_PARALLEL-1} in
    1) PARALLEL=1 ;;
    0) PARALLEL=0 ;;
    *)
        echo "release gates: UNA_GATES_PARALLEL is 1 (the default: sanitize beside build and deps) or 0 (every gate in sequence), not ${UNA_GATES_PARALLEL}" >&2
        exit 2
        ;;
esac
ISOLATED_GATE=sanitize
BESIDE_ISOLATED=" build deps "
# ISOLATED=1 when this run runs the isolated gate beside another: in parallel mode, with that
# gate and at least one of the gates it runs beside named. ISOLATED_INDEX is its first position.
ISOLATED=0 ISOLATED_INDEX="" beside=0 index=0
for gate in "${GATES[@]}"; do
    [ "$gate" = "$ISOLATED_GATE" ] && [ -z "$ISOLATED_INDEX" ] && ISOLATED_INDEX=$index
    case "$BESIDE_ISOLATED" in *" $gate "*) beside=1 ;; esac
    index=$((index + 1))
done
[ $PARALLEL = 1 ] && [ -n "$ISOLATED_INDEX" ] && [ $beside = 1 ] && ISOLATED=1

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
DOCKER_GATES=" linux linux-arm64 release reproducible hunt-linux-arm64 limits-linux-arm64 limits-linux toolchain-free "
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
# privacy pattern, the docs gate for the neutrality pattern. Neither is ever printed, logged or
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
# A run stopped by a signal waits for the isolated gate as for the gate in progress (#2181), and
# keeps the rows of every gate that ended, in their order.
ROWS=$OUT/rows ISOLATED_PID="" ISOLATED_WAITED=0 FLUSHED=0
gates_finish() {
    if [ $ISOLATED = 1 ]; then
        [ -z "$ISOLATED_PID" ] || wait_isolated
        ! declare -F flush_rows > /dev/null || flush_rows all
        rm -rf "${ROWS:?}"
    fi
    gates_unlock "$OUT"
}
trap gates_finish EXIT
trap 'exit 143' TERM
trap 'exit 129' HUP
CLONE=$OUT/clone
RESULTS=$OUT/results.tsv
printf 'gate\tsha\tresult\tseconds\tlog_sha256\n' > "$RESULTS"
if [ $ISOLATED = 1 ]; then
    rm -rf "${ROWS:?}"
    mkdir -p "$ROWS" || exit 2
fi
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

digest() { shasum -a 256 "$1" | cut -d' ' -f1; }

# The product (#2254, docs/release-gates.md §2 "The product"): the release build of the gated
# commit exactly as it ships its code, `cargo build --locked --release -p una-cli -p una-rt`
# with the solver library of `run`, which the build gate copies, right after that build and
# before any test build, into OUT/product: `una` and the `libuna_rt.a` it links native programs
# with (beside it, as a Cargo build leaves it), their SHA256SUMS and the commit. A later
# `cargo test --release` relinks target/release/una with the features of the dev-dependencies
# (the `test-hooks` of una-cli, una-http and una-hash, the `counters` of una-core), so no gate
# that runs `una` as the product reads target/release: each runs the copy through
# `with_product`, which first checks it (`product_verify`) and fails the gate when the copy is
# missing, is another commit's, does not match its digests or holds a test hook's name.
PRODUCT=$OUT/product
PRODUCT_FILES="una libuna_rt.a"
# A test build's names: the markers that the `test-hooks` features of una-cli
# (crates/una-cli/src/solver_child.rs), una-hash and una-http keep in the binary (`#[used]`),
# which scripts/check-release.sh refuses in an archive, and the environment variables of the
# test hooks, `UNA_TEST_<NAME>`, which only code under `test-hooks` reads (the `counters` of
# una-core unify in only with those dev-dependencies). Read with grep -a under LC_ALL=C.
PRODUCT_HOOKS='UNA_(CLI|HASH|HTTP)_TEST_HOOKS_MARKER|UNA_TEST_[A-Z0-9_]+'

# Whether the file $1 holds none of the test build's names; prints those it holds.
product_clean() {
    local found status
    LC_ALL=C grep -a -q -E "$PRODUCT_HOOKS" "$1"
    status=$?
    case $status in
        1) return 0 ;;
        0)
            found=$(LC_ALL=C grep -a -o -E "$PRODUCT_HOOKS" "$1" | LC_ALL=C sort -u | head -n 20)
            echo "release gates: FAIL: $1 was built with test features, it holds: $(printf '%s ' $found)(#2254)"
            ;;
        *) echo "release gates: FAIL: cannot read $1 (#2254)" ;;
    esac
    return 1
}

# The build gate's copy of the product, from the clone's target/release (the working directory):
# any earlier copy removed first, and none left when a file is missing or holds a test name.
product_save() {
    local file
    rm -rf "${PRODUCT:?}"
    mkdir -p "$PRODUCT" || return 1
    for file in $PRODUCT_FILES; do
        if [ ! -f "target/release/$file" ] || [ -L "target/release/$file" ]; then
            echo "release gates: FAIL: no target/release/$file to save as the product (#2254)"
            rm -rf "${PRODUCT:?}"
            return 1
        fi
        cp "target/release/$file" "$PRODUCT/$file" && product_clean "$PRODUCT/$file" \
            || { rm -rf "${PRODUCT:?}"; return 1; }
    done
    # shellcheck disable=SC2086
    (cd "$PRODUCT" && shasum -a 256 $PRODUCT_FILES > SHA256SUMS) && printf '%s\n' "$SHA" > "$PRODUCT/commit" \
        || { rm -rf "${PRODUCT:?}"; return 1; }
    echo "release gates: the product of $SHA, in $PRODUCT (#2254):"
    cat "$PRODUCT/SHA256SUMS"
}

# Whether OUT/product is the build gate's copy for this commit: its files regular, their digests
# those SHA256SUMS recorded (and it lists exactly them), and no test name in either.
product_verify() {
    local file listed
    if [ ! -d "$PRODUCT" ] || [ -L "$PRODUCT" ]; then
        echo "release gates: FAIL: no product at $PRODUCT; the build gate saves it (#2254)"
        return 1
    fi
    if [ "$(cat "$PRODUCT/commit" 2> /dev/null)" != "$SHA" ]; then
        echo "release gates: FAIL: the product at $PRODUCT is not the build of $SHA; run the build gate (#2254)"
        return 1
    fi
    for file in $PRODUCT_FILES; do
        if [ ! -f "$PRODUCT/$file" ] || [ -L "$PRODUCT/$file" ]; then
            echo "release gates: FAIL: no $PRODUCT/$file (#2254)"
            return 1
        fi
    done
    listed=$(awk '{ print $2 }' "$PRODUCT/SHA256SUMS" 2> /dev/null | paste -sd ' ' -)
    if [ "$listed" != "$PRODUCT_FILES" ]; then
        echo "release gates: FAIL: $PRODUCT/SHA256SUMS does not list exactly $PRODUCT_FILES (#2254)"
        return 1
    fi
    (cd "$PRODUCT" && shasum -a 256 -c SHA256SUMS) || {
        echo "release gates: FAIL: the product at $PRODUCT does not match the digests the build gate recorded (#2254)"
        return 1
    }
    for file in $PRODUCT_FILES; do
        product_clean "$PRODUCT/$file" || return 1
    done
    echo "release gates: the product: $PRODUCT/una, sha256 $(digest "$PRODUCT/una") (#2254)"
}

# Runs the command that follows with UNA_PRODUCT naming the product's `una`, once it is checked;
# a product that fails the check fails the gate before its command. Called inside `run`.
with_product() {
    # The docs gate's descriptor 3 (the neutrality pattern) is its command's alone.
    product_verify 3<&- || return 1
    UNA_PRODUCT=$PRODUCT/una
    export UNA_PRODUCT
    "$@"
}

record() { # gate result seconds
    local log=$OUT/logs/$1.log
    if [ $ISOLATED = 1 ]; then
        # The row of the gate at position CURRENT_INDEX, which flush_rows copies into
        # results.tsv in the order of the gates (#2181); written whole, then renamed.
        printf '%s\t%s\t%s\t%s\t%s\n' "$1" "$SHA" "$2" "$3" "$(digest "$log")" > "$ROWS/$CURRENT_INDEX.tmp" \
            && mv "$ROWS/$CURRENT_INDEX.tmp" "$ROWS/$CURRENT_INDEX"
    else
        printf '%s\t%s\t%s\t%s\t%s\n' "$1" "$SHA" "$2" "$3" "$(digest "$log")" >> "$RESULTS"
    fi
    echo "== $1: $2 (${3}s)"
}

# The rows of the gates, in their order, into results.tsv (#2181): every row from the first not
# yet copied up to the first gate that has not ended. With `all`, at the end of the run, every
# row there is, in order, skipping a gate that never ended (a run stopped by a signal).
flush_rows() {
    while [ "$FLUSHED" -lt ${#GATES[@]} ]; do
        if [ -f "$ROWS/$FLUSHED" ]; then
            cat "$ROWS/$FLUSHED" >> "$RESULTS"
        elif [ "${1:-}" != all ]; then
            return 0
        fi
        FLUSHED=$((FLUSHED + 1))
    done
}

# The isolated gate (#2181), started in the background in its own clone, OUT/clone-sanitize,
# made from OUT/clone once that is exact (a local clone: the same objects, the same tags), and
# checked to be exact by `run`, as any clone. Its log is OUT/logs/sanitize.log; what making the
# clone printed is OUT/logs/clone-sanitize.log. The Z3 library's cache is filled first, so that
# the two gates that start at once never both build it. When the gate passes, its clone is
# deleted; when it fails, all but target/debug is kept (the fuzzer's inputs, the logs of target/sanitize).
start_isolated() {
    local isolated=$OUT/clone-$ISOLATED_GATE prepared=$OUT/logs/clone-$ISOLATED_GATE.log
    echo "== $ISOLATED_GATE: started in $isolated, beside${BESIDE_ISOLATED% } (#2181)"
    # The cache first, here, before either gate starts: a failure is the gates' to report.
    { exact && (cd "$CLONE" && scripts/z3-library.sh); } > "$prepared" 2>&1
    (
        trap - EXIT TERM HUP
        CURRENT_INDEX=$ISOLATED_INDEX
        {
            rm -rf "${isolated:?}"
            if exact; then
                git clone --quiet --no-checkout "$CLONE" "$isolated" \
                    && git -C "$isolated" checkout --quiet --detach "$SHA" \
                    && git -C "$isolated" log -1 --format='commit %H%n%s'
            else
                echo "the clone is not exactly $SHA with no change; run the clone gate first"
            fi
        } >> "$prepared" 2>&1
        CLONE=$isolated
        "gate_$ISOLATED_GATE"
        if [ "$(cut -f3 "$ROWS/$CURRENT_INDEX" 2> /dev/null)" = PASS ]; then
            rm -rf "${isolated:?}"
        else
            rm -rf "${isolated:?}/target/debug"
        fi
    ) &
    ISOLATED_PID=$!
}

# Waits for the isolated gate, once; a gate that ended without its row (killed) is a FAIL.
wait_isolated() {
    [ -n "$ISOLATED_PID" ] && [ $ISOLATED_WAITED = 0 ] || return 0
    wait "$ISOLATED_PID"
    ISOLATED_WAITED=1
    if [ ! -f "$ROWS/$ISOLATED_INDEX" ]; then
        echo "release gates: the $ISOLATED_GATE gate ended without a result" >> "$OUT/logs/$ISOLATED_GATE.log"
        CURRENT_INDEX=$ISOLATED_INDEX record "$ISOLATED_GATE" FAIL 0
    fi
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
        # A solver gate's library (#1996, above): a path to a regular, non-empty file, or the
        # gate fails here, before its command.
        case "$SOLVER_GATES" in
            *" $gate "*)
                UNA_Z3_LIB=$(scripts/z3-library.sh) && [ -f "$UNA_Z3_LIB" ] && [ -s "$UNA_Z3_LIB" ] || {
                    echo "release gates: FAIL: the $gate gate runs cargo, and scripts/z3-library.sh gave no Z3 library (${UNA_Z3_LIB:-none}); without UNA_Z3_LIB its una would have no solver and fail with U5003 (ADR 0124, #1996)"
                    exit 1
                }
                export UNA_Z3_LIB
                echo "release gates: UNA_Z3_LIB=$UNA_Z3_LIB"
                ;;
        esac
        # Only the docs gate's command gets the neutrality pattern, on file descriptor 3 (a
        # pipe), and UNA_NEUTRALITY_PATTERN_FD says so; no other gate has that descriptor.
        if [ "$gate" = docs ] && [ -n "$NEUTRALITY" ]; then
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
    # A new clone has no product until its build gate saves one (#2254).
    rm -rf "${PRODUCT:?}"
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
    # The Z3 library built into una (ADR 0124) comes from `run`, as for every solver gate: every
    # cargo step of the clone gets it by UNA_Z3_LIB. The standalone z3 of scripts/install-z3.sh
    # serves the hunt and solver-identity gates alone.
    # Then no C math library symbol (#1092), in what ships: the release `una` just built links
    # every runnable program of tests/conformance/runtime that the native backend compiles
    # (scripts/no-libm-build.sh, into target/no-libm; target/ is ignored, so the clone stays
    # exact), and scripts/no-libm.sh reads the release graph's features and the undefined symbols
    # of the release `libuna_rt.a` and of those executables, strictly, and of that `una`, which
    # holds the solver, against scripts/no-libm-una.txt too (#1277). Each symbol and load
    # command check first runs its canaries, which must fail it.
    # No earlier product survives a build gate that fails before saving its own (#2254).
    rm -rf "${PRODUCT:?}"
    run build build_steps
}

# The build gate's steps (#2254): the release build, then its copy as the product, before the
# test build, then the checks of the product's files themselves.
build_steps() {
    cargo build --locked --release -p una-cli -p una-rt || return 1
    product_save || return 1
    # The product holds the solver (ADR 0124, #1996): it checks every package of examples/ at
    # tier 1, whose contracts are proof obligations; a `una` without the solver fails them with
    # U5003.
    sh -c '
        una=$1 checked=0
        for manifest in examples/*/una.toml; do
            [ -f "$manifest" ] || continue
            "$una" check --tier 1 "${manifest%/una.toml}" || { echo "release gates: FAIL: the product does not check ${manifest%/una.toml} (#2254)"; exit 1; }
            checked=$((checked + 1))
        done
        [ "$checked" -gt 0 ] || { echo "release gates: FAIL: no package of examples/ for the product to check (#2254)"; exit 1; }
        echo "release gates: the product checked $checked packages of examples/ at tier 1"' sh "$PRODUCT/una" || return 1
    sh -c '
        una=$1 runtime=$2
        no_libm=$(pwd)/target/no-libm
        rm -rf "${no_libm:?}"
        cargo test --workspace --locked \
            && scripts/no-libm-build.sh "$una" "$no_libm" \
            && scripts/no-libm-test.sh \
            && scripts/no-libm.sh "$una" "$runtime" "$no_libm/executables" "$no_libm/una-without-solver" \
            && scripts/load-commands-test.sh \
            && scripts/load-commands.sh "$una"' sh "$PRODUCT/una" "$PRODUCT/libuna_rt.a" || return 1
    # What the checks read is still what the build gate saved.
    product_verify
}

gate_solver-identity() {
    # ADR 0124 rule 31: the solver child against the standalone z3 built from the same library,
    # byte for byte, over every query of the repository's obligations, each twice; against the
    # Z3 project's own release (UNA_GATES_Z3_UPSTREAM, else the one scripts/install-z3.sh
    # installs) by status, listed for review. The log keeps the counts.
    run solver-identity sh -c '
        UNA_Z3_STANDALONE=$(dirname "$(dirname "$UNA_Z3_LIB")")/bin/z3
        export UNA_Z3_STANDALONE
        if [ ! -x "'"$Z3"'/bin/z3" ]; then scripts/install-z3.sh "'"$Z3"'" || exit 1; fi
        UNA_Z3_UPSTREAM=${UNA_GATES_Z3_UPSTREAM:-'"$Z3"'/bin/z3}
        export UNA_Z3_UPSTREAM
        cargo test --release --locked -p una-cli --test solver_identity -- --ignored --nocapture'
}

gate_limits() {
    # ADR 0124 rule 31, natively on each target: memout, cpu and wallclock each end as their
    # reason with the parent intact, and the confinement cases (scripts/limits.sh); here on macOS,
    # and in the Linux container of each architecture below.
    run limits scripts/limits.sh
}

gate_limits-linux-arm64() { run limits-linux-arm64 scripts/local-ci.sh --linux-arm64 --limits; }

# Emulated on the gate machine, where scripts/limits.sh refuses to run: it can pass only where the
# Docker engine is x86-64 (the native-host mode of scripts/local-ci.sh --linux, #1939) and
# scripts/emulation.sh finds the container native (ADR 0124 rule 31, #1442).
gate_limits-linux() { run limits-linux scripts/local-ci.sh --linux --limits; }

gate_toolchain-free() {
    # ADR 0124 rule 31, the build phase for `una check` and `una test`: the release una alone,
    # under a sandbox that execs nothing but una (macOS) and in a bare container (Linux).
    # The una under test is the one each archive of the release gate holds, which
    # scripts/release.sh built with the solver and the runtime inside it (ADR 0124 step 2,
    # #1357): for each shipped target, in its order, the aarch64-apple-darwin archive's on macOS
    # and a Linux archive's in the container of its architecture. Each archive is checked against
    # its .sha256 file and unpacked in OUT/toolchain-free, outside the clone; without the release
    # gate's archives the gate fails and says so.
    local version
    version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$CLONE/Cargo.toml" 2>/dev/null | head -1)
    run toolchain-free sh -c '
        version=$1 unpacked=$2
        shift 2
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
        [ $# -gt 0 ] || { echo "no shipped target" >&2; exit 1; }
        . scripts/release-targets.sh || exit 1
        # The archives must be exactly one per shipped target: an archive of another target would
        # go unchecked here.
        expected=$(for target in "$@"; do echo "una-$version-$target.tar.gz"; done | LC_ALL=C sort)
        present=$(cd "$built" 2> /dev/null && ls -- *.tar.gz 2> /dev/null | LC_ALL=C sort)
        [ -z "$present" ] || [ "$present" = "$expected" ] \
            || { echo "the archives of $built are not exactly one per shipped target" >&2; exit 1; }
        for target in "$@"; do
            # The leg of each platform of scripts/release-targets.sh.
            case $(release_target_platform "$target") in
                macos) leg="" ;;
                linux/arm64) leg=--linux-arm64 ;;
                linux/amd64) leg=--linux ;;
                *) echo "no toolchain-free leg for $target" >&2; exit 1 ;;
            esac
            una=$(unpacked_una "$target") || exit 1
            scripts/toolchain-free.sh "$una" examples/events $leg || exit 1
        done' \
        sh "$version" "$OUT/toolchain-free" $TARGETS
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
    # Every archive of the first build is compared, as is every shipped target's: one missing
    # from either build fails the gate, and so does an archive of the first build that is not a
    # shipped target's, which no other gate checks (#1903).
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
        version=${tag#v}
        [ $# -gt 0 ] || { echo "no shipped target"; status=1; }
        shipped=$(for target in "$@"; do echo "una-$version-$target.tar.gz"; done)
        for name in $( { printf "%s\n" "$shipped"; (cd "$first" && ls -- *.tar.gz 2> /dev/null); } | LC_ALL=C sort -u); do
            one=$(shasum -a 256 "$first/$name" 2> /dev/null | cut -d" " -f1)
            two=$(shasum -a 256 "$built/$name" 2> /dev/null | cut -d" " -f1)
            echo "$name: first ${one:-missing}, second ${two:-missing}"
            [ -n "$one" ] && [ "$one" = "$two" ] || status=1
            printf "%s\n" "$shipped" | grep -qxF -- "$name" \
                || { echo "$name is not the archive of a shipped target"; status=1; }
        done
        docker volume rm "$again" "$again-arm64" > /dev/null 2>&1
        rm -rf "${built:?}"
        cp -R "$first" "$built"
        rm -rf "${first:?}"
        [ $status -eq 0 ] && echo "the archives are reproducible"
        exit $status' bash $TARGETS
}

gate_determinism() {
    # The `una --json check` outputs are the product's (#2254): the conformance suite's test
    # build relinks target/debug/una with the test features.
    run determinism with_product bash -c '
        set -e
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
                printf "%s %s\n" "$("$UNA_PRODUCT" --json check "$target" | shasum -a 256 | cut -c1-16)" "$target"
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
    # The security hunt's fixed run (#767, docs/release-gates.md): the product's `una` (#2254),
    # every compiler run under sandbox-exec, the evidence in OUT/hunt, outside the clone. Python
    # runs with -B, so the clone gets no bytecode files and stays exact.
    local hunt=$OUT/hunt
    rm -rf "${hunt:?}"
    # The output directory and the commit go in as positional arguments, never spliced into the
    # script's text.
    # The re-check's executable comes from the pinned Lean, as scripts/formal.sh finds it; a
    # missing executable fails the gate. The pinned standalone z3 that the hunt compares with is
    # installed as the solver-identity gate installs it, when it is not there yet (#1996); the
    # hunt builds no `una`, and a missing product fails the gate (#2254).
    run hunt with_product sh -c '
        if [ ! -x "$3/bin/z3" ]; then scripts/install-z3.sh "$3" || exit 1; fi
        scripts/check-solver.sh || exit 1
        una=$UNA_PRODUCT
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

gate_docs() {
    # The documents and the docs bundle, the two products of this repository that the website
    # reads (#2081): the link check, then the bundle, built with the product's `una` (#2254).
    # The website itself is built and checked by its own repository's flow against a published
    # bundle, never here: this gate clones, installs and builds nothing of it.
    # The neutrality pattern reaches this one command on file descriptor 3 (`run`), never in
    # its environment or arguments; the command reads it into a variable, closes the
    # descriptor, and hands it to the bundle step on a pipe of its own.
    run docs with_product sh -c '
        unset neutrality
        neutrality=
        if [ -n "${UNA_NEUTRALITY_PATTERN_FD:-}" ]; then
            neutrality=$(cat <&3)
            exec 3<&-
        fi
        unset UNA_NEUTRALITY_PATTERN_FD
        # The step reads the pattern from its own pipe on descriptor 3, when there is one.
        with_pattern() {
            if [ -n "$neutrality" ]; then
                printf "%s\n" "$neutrality" | { UNA_NEUTRALITY_PATTERN_FD=3 "$@" 3<&0 </dev/null; }
            else
                "$@"
            fi
        }
        scripts/doc-links.py --self-test \
            && scripts/doc-links.py \
            && with_pattern env UNA_BIN="$UNA_PRODUCT" scripts/site-bundle.sh "'"$OUT"'/site-bundle"'
}
gate_formal() { run formal scripts/formal.sh; }

gate_timing-channels() {
    # The security timing oracles (#1145, crates/una-cli/tests/timing_channels.rs): each test is
    # ignored in the ordinary run and runs only here and in the step of the same name of
    # scripts/local-ci.sh, one at a time, on the real clock of a native host.
    # Then the cryptography (#2119), which no other gate times: the statistical timing tests of
    # crates/una-crypto/tests/timing.rs (ADR 0133 rule 9), ignored in the ordinary run, and the
    # scaling tests of crates/una-crypto/tests/perf.rs (ADR 0133 rule 10), which return at once
    # in a debug build. Each of the three runs whatever the others did, so the log holds every
    # statistic, and one that fails fails the gate.
    run timing-channels sh -c '
        status=0
        cargo test --release --locked -p una-cli --test timing_channels -- --ignored --test-threads=1 --nocapture || status=1
        cargo test --release --locked -p una-crypto --test timing -- --ignored --test-threads=1 --nocapture || status=1
        cargo test --release --locked -p una-crypto --test perf -- --test-threads=1 --nocapture || status=1
        exit "$status"'
}

gate_owner-replay() {
    # The replay of an owner's chain against its oracle and the literal reference of ADR 0122
    # Amendment 14 (#1997): scripts/owner-replay-gate.sh, which builds the release test binary
    # of una-project, runs each test directly under its supervisor, and checks the guards and
    # the recorded counts. Exhaustive depth 5 on the whole alphabet is a soak (card #2018), not
    # here.
    run owner-replay scripts/owner-replay-gate.sh 3600
}

# #766: Miri on the runtime's unit tests, and the native equivalence test against the runtime
# built with AddressSanitizer, by the checking toolchain of scripts/check-toolchain.toml, which
# builds no release artifact.
gate_sanitize() { run sanitize scripts/sanitize.sh; }

# The gates, in order; with ISOLATED (#2181), the isolated gate starts at its own position or at
# the first gate it runs beside, whichever comes first, and any other gate waits for it to end.
if [ $ISOLATED = 1 ]; then
    echo "release gates: the parallel plan (#2181): $ISOLATED_GATE beside${BESIDE_ISOLATED% }, every other gate alone; UNA_GATES_PARALLEL=0 runs them in sequence"
fi
CURRENT_INDEX=0
for gate in "${GATES[@]}"; do
    if [ $ISOLATED = 1 ]; then
        if [ "$CURRENT_INDEX" = "$ISOLATED_INDEX" ]; then
            [ -n "$ISOLATED_PID" ] || start_isolated
            CURRENT_INDEX=$((CURRENT_INDEX + 1))
            continue
        fi
        case "$BESIDE_ISOLATED" in
            *" $gate "*) [ -n "$ISOLATED_PID" ] || start_isolated ;;
            *) wait_isolated ;;
        esac
    fi
    "gate_$gate"
    [ $ISOLATED = 1 ] && flush_rows
    CURRENT_INDEX=$((CURRENT_INDEX + 1))
done
if [ $ISOLATED = 1 ]; then
    wait_isolated
    flush_rows all
    rm -rf "${ROWS:?}"
fi

# The Docker volume that local-ci made for this clone's target directory (named from its
# path), so that runs do not pile volumes up. Only a run that used Docker removes them, on the
# context it checked.
if [ $NEEDS_DOCKER = 1 ] && command -v docker > /dev/null 2>&1; then
    checkout=$(printf '%s' "$CLONE" | shasum -a 256 | cut -c1-12)
    docker volume rm "una-local-ci-target-$checkout" "una-local-ci-target-$checkout-arm64" \
        > /dev/null 2>&1 || true
fi

# The lines of the release gate's log that start with `PREFIX: <target>: `, for the record
# (#1359, #1971): each one whose value has the exact shape of the extended regular expression
# SHAPE, rewritten as `- <target>: <value>`, sorted, once; then, when any line with the prefix
# has another shape (a control character such as a carriage return included, which could pose
# as a heading of the record to a reader that splits lines on it), a line that says how many
# were not copied, which the summary's shape check of the runtime strip refuses. Read under
# LC_ALL=C, so that an invalid byte elsewhere in the log does not stop sed or grep. Prints
# nothing when no line has the prefix. O(lines of the log).
release_log_lines() {
    local prefix=$1 shape=$2 log=$OUT/logs/release.log all copied
    all=$(LC_ALL=C grep -a -c "^$prefix: " "$log" 2>/dev/null) || all=0
    copied=$(LC_ALL=C grep -a -c -E "^$prefix: [a-z0-9_-]+: ($shape)\$" "$log" 2>/dev/null) || copied=0
    LC_ALL=C sed -n -E "s/^$prefix: ([a-z0-9_-]+): ($shape)\$/- \1: \2/p" "$log" 2>/dev/null | LC_ALL=C sort -u
    if [ "$all" != "$copied" ]; then
        echo "- Not copied: $((all - copied)) of the $all lines of logs/release.log that start with \`$prefix: \` are out of shape"
    fi
}

# The record.
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$CLONE/Cargo.toml" 2>/dev/null | head -1)
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
    echo "- Shipped targets (\`scripts/release-targets.txt\`): $(printf '`%s`, ' $TARGETS | sed 's/, $//')"
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
    # The archives must be exactly one per shipped target (#1903): once there is any archive, or
    # the release gate passed, a missing or an extra one fails the verdict (`archives`).
    archives_wrong=0
    if [ -n "$archives" ] || awk -F'\t' 'NR > 1 && $1 == "release" && $3 == "PASS" { found = 1 } END { exit !found }' "$RESULTS"; then
        expected=$(while read -r target; do echo "una-$version-$target.tar.gz"; done <<< "$TARGETS" | LC_ALL=C sort)
        packed=$(for archive in $archives; do basename "$archive"; done | LC_ALL=C sort)
        if [ -z "$version" ] || [ "$packed" != "$expected" ]; then
            archives_wrong=1
            echo
            echo "The archives are not exactly one per shipped target: expected $(printf '%s ' $expected)"
        fi
    fi
    echo
    # The strip that removed the debug sections of each runtime built into una (#1359), as
    # scripts/release.sh printed it in the release gate's log: its version and its binary's digest.
    # GNU strip prints `GNU strip (GNU Binutils for Ubuntu) 2.42` on Ubuntu and
    # `GNU strip version 2.40-21.el8` from AlmaLinux 8's gcc-toolset-13 (#2105); each form is
    # matched exactly, and scripts/release-summary.py's STRIP_ROW takes the same two.
    echo "## Runtime strip"
    echo
    strips=$(release_log_lines strip '(xcrun strip -S \((Xcode [0-9.]+ build [0-9A-Za-z]+|Command Line Tools [0-9.]+)\)|GNU strip \([A-Za-z0-9 .]+\) [0-9][0-9.]* --strip-debug -D|GNU strip version [0-9]+\.[0-9]+(\.[0-9]+)?-[0-9]+(\.[0-9]+)*\.el[0-9]+(_[0-9]+)* --strip-debug -D), sha256 [0-9a-f]{64}')
    if [ -z "$strips" ]; then
        echo "None: the release gate did not run or did not record the strip tool."
    else
        printf '%s\n' "$strips"
    fi
    echo
    # The Linux toolchain that linked each Linux `una` (#1926, #1958, #1971), as
    # scripts/check-toolchain.sh printed it in the release gate's log, after it checked each
    # package and file against supply-chain/z3.lock: `toolchain: <target>: <package> <version>`
    # and `toolchain: <target>: <file> sha256 <digest>`. Only the record holds them; the summary
    # copies none (docs/release-gates.md §3).
    echo "## Linux toolchain"
    echo
    toolchains=$(release_log_lines toolchain '[A-Za-z0-9._+-]+ (sha256 [0-9a-f]{64}|[A-Za-z0-9.+~:_-]+)')
    if [ -z "$toolchains" ]; then
        echo "None: the release gate did not run or did not record a Linux toolchain."
    else
        printf '%s\n' "$toolchains"
    fi
    echo
    # The musl inputs embedded into each Linux `una` (ADR 0124 rules 12 and 20, #1971), as
    # scripts/release.sh printed them in the release gate's log, after it checked each digest
    # against supply-chain/musl.lock: `musl: <target>: <file> sha256 <digest>`. Only the record
    # holds them; the summary copies none (docs/release-gates.md §3).
    echo "## musl inputs"
    echo
    musls=$(release_log_lines musl '[A-Za-z0-9._+-]+ sha256 [0-9a-f]{64}')
    if [ -z "$musls" ]; then
        echo "None: the release gate did not run or did not record the musl inputs."
    else
        printf '%s\n' "$musls"
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
        echo "None: the docs gate did not run or did not write it."
    else
        echo "| Bundle | sha256 |"
        echo "| --- | --- |"
        for bundle in $bundles; do
            echo "| \`$(basename "$bundle")\` | \`$(digest "$bundle")\` |"
        done
    fi
    echo
    failed=$(tail -n +2 "$RESULTS" | awk -F'\t' '$3 == "FAIL" { print $1 }' | sort -u | paste -sd ' ' - | sed 's/ /, /g')
    [ $archives_wrong = 1 ] && failed=${failed:+$failed, }archives
    missing=()
    for gate in "${REQUIRED[@]}"; do
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
# The exit status follows the verdict (#1933): a FAIL row fails the run whatever the record says.
grep -q $'\tFAIL\t' "$RESULTS" && exit 1
case $(sed -n 's/^Verdict: \([A-Z]*\) .*$/\1/p' "$OUT/record.md") in
    PASS) exit 0 ;;
    INCOMPLETE) exit 3 ;;
    *) exit 1 ;;
esac
