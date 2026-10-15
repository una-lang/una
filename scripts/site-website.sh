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
# Neutrality of the Learn pages (#789, una-lang/una-by-example#15): check.py also runs with
# --require-neutrality, so every README of una-by-example, English or translated, is checked
# against UNA_NEUTRALITY_PATTERN, which comes only from this script's environment (from
# scripts/release-gates.sh, which reads it from its caller or the Keychain), and a run without
# the pattern fails. Only under UNA_GATES_UNCHECKED=1, a run whose summary is not for
# publication (scripts/release-gates.sh), is the check skipped, and check.py says so. The
# website is built with UNA_LEARN_DRAFTS unset, so no environment can turn its build into the
# preview of draft translations.
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

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

# The neutrality pattern, taken from the environment into a variable at once and unset, so that
# only the two commands below that check pages receive it, each in its own environment.
neutral=${UNA_NEUTRALITY_PATTERN:-}
unset UNA_NEUTRALITY_PATTERN

fail() {
    echo "site-website: FAIL ($1)" >&2
    exit 1
}

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
    neutrality=--require-neutrality
    if [ "${UNA_GATES_UNCHECKED:-}" = 1 ]; then
        neutrality=
        echo "una-by-example: UNA_GATES_UNCHECKED=1, so the neutrality pattern is not required"
    fi
    # shellcheck disable=SC2086 # $neutrality is one flag or none.
    UNA_NEUTRALITY_PATTERN=$neutral python3 -B "$learn/scripts/check.py" --una "$una" --una-version-from-binary --archives "$archives" $neutrality \
        || fail "an example of una-by-example does not check with this una"
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
UNA_NEUTRALITY_PATTERN=$neutral env -u UNA_LEARN_DRAFTS UNA_BUNDLE="$bundle" UNA_BY_EXAMPLE="$learn" npm run --silent build >"$out/website-build.log" 2>&1 \
    || { cat "$out/website-build.log" >&2; fail "the website does not build from the bundle"; }
cat "$out/website-build.log"
if [ -n "$learn" ]; then
    grep -q '^learn: [1-9][0-9]* pages from una-by-example' "$out/website-build.log" \
        || fail "the website built no Learn pages from una-by-example"
fi
UNA_BUNDLE=$bundle UNA_BY_EXAMPLE=$learn UNA_BIN=$una npm run --silent check:samples || fail "a code sample of the website does not check"
[ -z "$(git status --porcelain --untracked-files=no)" ] || fail "the website build changed tracked files"

echo "site-website: PASS (website $commit, bundle $(sed -n 's/.*"version":"\([^"]*\)".*"commit":"\([0-9a-f]\{12\}\).*/una \1 at \2/p' "$bundle/manifest.json" | head -n 1))"
