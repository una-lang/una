#!/bin/sh
# Builds the website, una-lang/website at the commit that site/website.lock pins, from a docs
# bundle of scripts/site-bundle.sh (#584), and checks it: the website's build renders every
# page from the bundle and fails on a broken budget, and its sample check runs every code
# sample of the site with this checkout's `una` (Z3 at the pinned version on PATH). Before the
# build, the heading ids that scripts/doc-links.py checks fragments against are compared with
# the ones the website gives the same documents (#708).
#
# When the pinned website has a learn.lock (website#30), its Learn section comes from
# una-lang/una-by-example at the commit that lock names (una#733): that repository is cloned at
# the commit, its download archives are built with its scripts/archive.py, and its
# scripts/check.py checks every example, each Try it change and the archives with this
# checkout's `una`, before the website builds /learn/ from the clone. The build must then
# report its Learn pages. A website without learn.lock is built without them, and this says so.
#
# Neutrality of the Learn pages (#789, una-lang/una-by-example#15): this script checks every
# text file of the una-by-example clone, English or translated, its packages, outcomes and
# changes included, against the neutrality pattern with grep -E (the dialect of every check of
# the gates), and fails on a match, listing the files by path only. The pattern comes on the
# file descriptor that UNA_NEUTRALITY_PATTERN_FD names, as scripts/release-gates.sh passes it,
# else from UNA_NEUTRALITY_PATTERN, for a run by hand; it never reaches an environment or an
# argument of any command: not check.py, which runs without it, and not the website's build.
# A run without the pattern fails, unless UNA_GATES_UNCHECKED=1 marks it as not for publication
# (scripts/release-gates.sh), and then says that it skipped the check. The website is built
# with UNA_LEARN_DRAFTS unset, so no environment can turn its build into the preview of draft
# translations.
#
# It needs git access to the website's repository (and to una-by-example when the website pins it), Node.js (>= 20.19) with npm, and Python 3. It installs
# the website's packages with `npm ci --ignore-scripts`, from its package-lock.json, so no
# install script runs. Before that, the website's scripts/deps-audit.mjs audits the lockfile
# against the npm registry's advisories with npm at the website's pinned version, failing closed
# on any advisory or an audit that does not answer, and the website's tests, which its build
# runs, check the lockfile against its supply-chain review record (#783). It writes only under OUT. UNA_SITE_WEBSITE_SOURCE overrides where the
# website is cloned from, UNA_SITE_LEARN_SOURCE where una-by-example is cloned from, and UNA_BIN which `una` checks the samples (default:
# target/release/una).
#
# Usage: scripts/site-website.sh BUNDLE OUT
#   BUNDLE  the bundle directory, OUT/una-docs-<version> of scripts/site-bundle.sh
#   OUT     a directory for the website's clone and build
set -eu
# A trace would print the pattern: refuse it; drop the shells' startup files; no core file.
case $- in
    *x*)
        echo "site-website: refusing to run with xtrace (set -x), which would print the pattern" >&2
        exit 2
        ;;
esac
unset BASH_ENV ENV
ulimit -c 0

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

fail() {
    echo "site-website: FAIL ($1)" >&2
    exit 1
}

# The neutrality pattern, read into a variable at once; no command this script runs inherits
# it, and only the check of the Learn pages below gets it, on a pipe.
# A name the caller exported would stay exported: unset first.
unset neutral
neutral=
if [ -n "${UNA_NEUTRALITY_PATTERN_FD:-}" ]; then
    case $UNA_NEUTRALITY_PATTERN_FD in
        3) neutral=$(cat <&3); exec 3<&- ;;
        *) fail "UNA_NEUTRALITY_PATTERN_FD must be 3" ;;
    esac
else
    neutral=${UNA_NEUTRALITY_PATTERN:-}
fi
unset UNA_NEUTRALITY_PATTERN UNA_NEUTRALITY_PATTERN_FD

[ $# -eq 2 ] || {
    echo "usage: scripts/site-website.sh BUNDLE OUT" >&2
    exit 2
}
[ -f "$1/manifest.json" ] || fail "$1 is not a docs bundle: no manifest.json"
bundle=$(CDPATH='' cd -- "$1" && pwd)
mkdir -p "$2"
out=$(CDPATH='' cd -- "$2" && pwd)

lock=site/website.lock
repository=$(sed -n 's/^repository = "\(.*\)"$/\1/p' "$lock")
commit=$(sed -n 's/^commit = "\([0-9a-f]\{40\}\)"$/\1/p' "$lock")
[ -n "$repository" ] && [ -n "$commit" ] || fail "$lock must name a repository and a 40-digit commit"
source=${UNA_SITE_WEBSITE_SOURCE:-$repository}

command -v node >/dev/null 2>&1 && command -v npm >/dev/null 2>&1 || fail "Node.js and npm are needed"
node -e 'const [a, b] = process.versions.node.split(".").map(Number); process.exit(a > 20 || (a === 20 && b >= 19) ? 0 : 1)' \
    || fail "Node.js $(node --version) is older than 20.19"
una=${UNA_BIN:-target/release/una}
[ -x "$una" ] || fail "no una binary at $una: build it, or set UNA_BIN"
una=$(CDPATH='' cd -- "$(dirname -- "$una")" && pwd)/$(basename -- "$una")
mkdir -p "$out/anchors"
python3 scripts/doc-links.py --anchors >"$out/anchors/ids.json" || fail "scripts/doc-links.py --anchors"

website=$out/website
rm -rf "${website:?}"
git clone --quiet --no-checkout "$source" "$website" || fail "cannot clone $source"
git -C "$website" checkout --quiet --detach "$commit" || fail "the website has no commit $commit"
[ "$(git -C "$website" rev-parse HEAD)" = "$commit" ] || fail "the website clone is not $commit"
echo "website: $commit"

# The Learn section (una#733, website#30): una-by-example at the commit the website pins.
learn=
if [ -f "$website/learn.lock" ]; then
    learn_repository=$(sed -n 's/^repository = "\(.*\)"$/\1/p' "$website/learn.lock")
    learn_commit=$(sed -n 's/^commit = "\([0-9a-f]\{40\}\)"$/\1/p' "$website/learn.lock")
    [ -n "$learn_repository" ] && [ -n "$learn_commit" ] || fail "the website's learn.lock must name a repository and a 40-digit commit"
    learn=$out/una-by-example
    rm -rf "${learn:?}"
    git clone --quiet --no-checkout "${UNA_SITE_LEARN_SOURCE:-$learn_repository}" "$learn" || fail "cannot clone una-by-example"
    git -C "$learn" checkout --quiet --detach "$learn_commit" || fail "una-by-example has no commit $learn_commit"
    [ "$(git -C "$learn" rev-parse HEAD)" = "$learn_commit" ] || fail "the una-by-example clone is not $learn_commit"
    echo "una-by-example: $learn_commit"
    # -B: Python writes no bytecode into the clone, which check.py refuses.
    archives=$out/una-by-example-archives
    rm -rf "${archives:?}"
    python3 -B "$learn/scripts/archive.py" "$archives" || fail "una-by-example's archives do not build"
    python3 -B "$learn/scripts/check.py" --una "$una" --una-version-from-binary --archives "$archives" \
        || fail "an example of una-by-example does not check with this una"
    # The neutrality of every text file of the clone (binary files skipped, .git aside): one
    # grep, the pattern on its descriptor 3 from a pipe, matches listed by path only (-l).
    if [ -n "$neutral" ]; then
        matched=$out/una-by-example-not-neutral.txt
        status=0
        printf '%s\n' "$neutral" \
            | (cd "$learn" && grep -rEIil --exclude-dir=.git -f /dev/fd/3 . 3<&0 </dev/null 2>/dev/null) \
            >"$matched" || status=$?
        case $status in
            0)
                sed -e 's#^\./##' -e 's#^#una-by-example: not neutral: #' "$matched" >&2
                fail "files of una-by-example match the neutrality pattern (listed above)"
                ;;
            1) echo "una-by-example: neutrality checked, $(git -C "$learn" ls-files | wc -l | tr -d ' ') files" ;;
            *) fail "the neutrality pattern could not be applied to una-by-example (grep -E): it is not a valid extended regular expression, or a file cannot be read" ;;
        esac
    elif [ "${UNA_GATES_UNCHECKED:-}" = 1 ]; then
        echo "una-by-example: neutrality skipped: no pattern, and UNA_GATES_UNCHECKED=1 marks this run as not for publication"
    else
        fail "no neutrality pattern for una-by-example: scripts/release-gates.sh passes it, or set UNA_NEUTRALITY_PATTERN for a run by hand"
    fi
else
    echo "una-by-example: skipped (the website at $commit has no learn.lock)"
fi

cd "$website"
# Supply chain: only the exact packages of the pinned lockfile, under its integrity hashes,
# and no package's install script runs on the release machine.
[ -f package-lock.json ] || fail "the website has no package-lock.json at $commit"
# The dependency audit (#783), before anything is installed, so a package with a known advisory
# never reaches the machine. It needs the network; offline, it fails.
[ -f scripts/deps-audit.mjs ] || fail "the website at $commit has no scripts/deps-audit.mjs (#783)"
node scripts/deps-audit.mjs || fail "the website's dependency audit"
npm ci --ignore-scripts --no-audit --no-fund --loglevel=error || fail "npm ci"
node "$root/scripts/doc-links/website-anchors.mjs" "$out/anchors/ids.json" "$root" \
    || fail "the heading ids of scripts/doc-links.py are not the website's"
env -u UNA_LEARN_DRAFTS UNA_BUNDLE="$bundle" UNA_BY_EXAMPLE="$learn" npm run --silent build >"$out/website-build.log" 2>&1 \
    || { cat "$out/website-build.log" >&2; fail "the website does not build from the bundle"; }
cat "$out/website-build.log"
if [ -n "$learn" ]; then
    grep -q '^learn: [1-9][0-9]* pages from una-by-example' "$out/website-build.log" \
        || fail "the website built no Learn pages from una-by-example"
fi
env -u UNA_LEARN_DRAFTS UNA_BUNDLE="$bundle" UNA_BY_EXAMPLE="$learn" UNA_BIN="$una" npm run --silent check:samples || fail "a code sample of the website does not check"
[ -z "$(git status --porcelain --untracked-files=no)" ] || fail "the website build changed tracked files"

echo "site-website: PASS (website $commit, bundle $(sed -n 's/.*"version":"\([^"]*\)".*"commit":"\([0-9a-f]\{12\}\).*/una \1 at \2/p' "$bundle/manifest.json" | head -n 1))"
