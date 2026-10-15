#!/bin/bash
# The release gates (#232, docs/release-gates.md): the local, recorded run that replaces
# hosted CI for merges, tags and releases. It runs each gate on exactly one commit, in a fresh
# clone made for this run, and checks before every gate that the clone is still that commit
# with no change. A failing gate does not stop the next one.
#
# Gates, in order (all of them when none is named):
#   clone        a fresh clone of the commit, from the repository's remote
#   build        cargo build --release -p una-cli -p una-rt, then cargo test --workspace, cold
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
#   site         scripts/doc-links.py: its self-test, then every link of the repository's
#                documents names a tracked file and every fragment a heading (#708). Then (#584)
#                scripts/site-bundle.sh writes the docs bundle into OUT/site-bundle with the
#                clone's release `una`, and scripts/site-website.sh compares the heading ids with
#                the website's and builds the website that site/website.lock pins from the
#                bundle, with its budget and its code samples checked; a missing Python 3,
#                Node.js or npm, or no access to the website's repository, is a FAIL
#   formal       scripts/formal.sh: the Lean development of the formal track builds with its pinned
#                toolchain, and no stated theorem depends on `sorry` (#46)
#
# Output, in OUT: results.tsv (gate, sha, PASS or FAIL, seconds, sha256 of the gate's log),
# logs/<gate>.log, record.md, the full validation record, and, when the record names its
# version, summary.md, its public summary (scripts/release-summary.py, #717), and
# release-notes.md, the summary followed by the notes that the release gate wrote. All are
# described in docs/release-gates.md. The clone is OUT/clone.
#
# UNA_GATES_SOURCE overrides where the clone comes from (default: the remote `origin`), and
# UNA_GATES_Z3 where the pinned solver is (default: target/z3 of this checkout or of the clone).
# Exit code: 0 when no gate failed, 1 when one did, 2 for a usage error.
#
# Usage: scripts/release-gates.sh SHA OUT [GATE...]
set -u

ALL=(clone build mac linux linux-arm64 release reproducible determinism native site formal)

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

# The privacy pattern of the public summary (#717, docs/release-gates.md §3): UNA_PRIVACY_PATTERN
# when the caller set it, else the macOS Keychain item una-privacy-pattern. It is held in a shell
# variable that is never exported, and the caller's UNA_PRIVACY_PATTERN is unset at once, so no
# gate and no child of a gate inherits it; only the summary step receives it. It is never
# printed, logged or written. Without it the run stops before any gate (fail closed), unless
# UNA_GATES_UNCHECKED=1 asks for a run whose summary is unchecked and that prints no publish
# command, such as a dry run.
PRIVACY=${UNA_PRIVACY_PATTERN:-}
unset UNA_PRIVACY_PATTERN
if [ -z "$PRIVACY" ] && command -v security > /dev/null 2>&1; then
    PRIVACY=$(security find-generic-password -s una-privacy-pattern -w 2> /dev/null) || PRIVACY=""
fi
if [ -z "$PRIVACY" ]; then
    if [ "${UNA_GATES_UNCHECKED:-}" = 1 ]; then
        echo "release gates: no privacy pattern; UNA_GATES_UNCHECKED=1, so the summary is unchecked and no publish command is printed" >&2
    else
        echo "release gates: no privacy pattern: set UNA_PRIVACY_PATTERN, or create the Keychain item una-privacy-pattern; UNA_GATES_UNCHECKED=1 runs the gates without it, for a run that is not published" >&2
        exit 2
    fi
fi
SOURCE=${UNA_GATES_SOURCE:-$(git -C "$REPO" remote get-url origin)}
mkdir -p "$OUT/logs"
OUT=$(CDPATH='' cd -- "$OUT" && pwd)
CLONE=$OUT/clone
RESULTS=$OUT/results.tsv
printf 'gate\tsha\tresult\tseconds\tlog_sha256\n' > "$RESULTS"
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
    elif (cd "$CLONE" && "$@") > "$log" 2>&1; then
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
    run build sh -c '
        if [ ! -x "'"$Z3"'/bin/z3" ]; then scripts/install-z3.sh "'"$Z3"'" || exit 1; fi
        scripts/check-solver.sh \
            && cargo build --locked --release -p una-cli -p una-rt \
            && cargo test --workspace --locked'
}

gate_mac() { run mac scripts/local-ci.sh; }

gate_linux() { run linux scripts/local-ci.sh --linux; }

gate_linux-arm64() { run linux-arm64 scripts/local-ci.sh --linux-arm64; }

gate_release() {
    # The archives are named after the workspace version, so the run tags it here, in the
    # clone only; the tag is never pushed.
    local version tag
    version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$CLONE/Cargo.toml" 2>/dev/null | head -1)
    tag=v$version
    run release sh -c "git tag -f '$tag' '$SHA' >/dev/null && git status --porcelain && scripts/local-release.sh '$tag'"
}

gate_reproducible() {
    # The release gate's archives, kept aside, against a second build of the same commit: a new
    # checkout (local-release.sh makes one per run, at a new path) and new Docker volumes for
    # the Linux target directories, so every artifact is built again from cold (#679). Its cost
    # is one more release gate. The release gate's archives are put back at the end, so the
    # record lists the ones the release gate built.
    local version tag
    version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$CLONE/Cargo.toml" 2>/dev/null | head -1)
    tag=v$version
    run reproducible bash -c '
        set -u
        tag='"'$tag'"'
        first='"'$OUT'"'/reproducible-first
        again=una-local-release-again
        built=target/local-release/$tag
        ls "$built"/*.tar.gz > /dev/null 2>&1 || { echo "no archives in $built; run the release gate first"; exit 1; }
        rm -rf "${first:?}"
        cp -R "$built" "$first" || exit 1
        docker volume rm "$again" "$again-arm64" > /dev/null 2>&1
        git tag -f "$tag" '"'$SHA'"' > /dev/null || exit 1
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
        first=$(cargo test --quiet -p una-cli --test conformance -- --nocapture 2>&1 | grep "conformance:")
        second=$(cargo test --quiet -p una-cli --test conformance -- --nocapture 2>&1 | grep "conformance:")
        echo "conformance, run 1: $first"
        echo "conformance, run 2: $second"
        [ "$first" = "$second" ]
        case "$first" in *" 0 failures"*) ;; *) exit 1 ;; esac
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
    # aside. One test, so the gate and `cargo test` cannot drift apart.
    run native sh -c 'cargo test -p una-cli --test native every_runnable_program_runs_natively_as_in_the_interpreter -- --exact --nocapture'
}

gate_site() {
    run site sh -c '
        version=$(sed -n "s/^version = \"\(.*\)\"$/\1/p" Cargo.toml | head -n 1)
        scripts/doc-links.py --self-test \
            && scripts/doc-links.py \
            && UNA_BIN=target/release/una scripts/site-bundle.sh "'"$OUT"'/site-bundle" \
            && UNA_BIN=target/release/una scripts/site-website.sh "'"$OUT"'/site-bundle/una-docs-$version" "'"$OUT"'/site-website"'
}
gate_formal() { run formal scripts/formal.sh; }

for gate in "${GATES[@]}"; do
    "gate_$gate"
done

# The Docker volume that local-ci made for this clone's target directory (named from its
# path), so that runs do not pile volumes up.
if command -v docker > /dev/null 2>&1; then
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
if [ -n "$version" ] && UNA_PRIVACY_PATTERN="$PRIVACY" python3 -B "$REPO/scripts/release-summary.py" "$version" "$OUT/record.md" "$OUT/summary.md"; then
    built=$CLONE/target/local-release/v$version
    if [ -f "$built/notes.md" ]; then
        { cat "$OUT/summary.md"; echo; cat "$built/notes.md"; } > "$OUT/release-notes.md"
        if grep -qx 'Verdict: PASS (every gate)' "$OUT/summary.md" && [ -n "$PRIVACY" ]; then
            cp "$OUT/summary.md" "$OUT/una-$version-validation.md"
            sh "$REPO/scripts/release-publish.sh" "$version" "$OUT" "$built"
        fi
    fi
fi
grep -q $'\tFAIL\t' "$RESULTS" && exit 1
exit 0
