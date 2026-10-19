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
# scripts/check.py checks every example, each Try it change and the archives with the `una` that
# the clone's una.lock pins, before the website builds /learn/ from the clone: this checkout's
# `una` when its version is the lock's, else that published release (scripts/site-learn-una.sh,
# #1039). When the lock names a
# release, the one whose archives the Learn pages link to, that release must hold exactly the
# files the build makes, the archives and SHA256SUMS, byte for byte (una#831,
# scripts/site-learn-release.sh, which reads it with `gh release download`), and the website's
# own reading of learn.lock must name the same repository, commit and release. The build must then report its Learn pages. A website without
# learn.lock is built without them, and this says so.
#
# It needs git access to the website's repository (and to una-by-example when the website pins it), Node.js (>= 20.19) with npm, and Python 3. It installs
# the website's packages with `npm ci --ignore-scripts`, from its package-lock.json, so no
# install script runs. Before that, the website's scripts/deps-audit.mjs audits the lockfile
# against the npm registry's advisories with npm at the website's pinned version, failing closed
# on any advisory or an audit that does not answer, and the website's tests, which its build
# runs, check the lockfile against its supply-chain review record (#783). It writes only under OUT. UNA_SITE_WEBSITE_SOURCE overrides where the
# website is cloned from, UNA_SITE_LEARN_SOURCE where una-by-example is cloned from,
# UNA_SITE_LEARN_RELEASE_DIR a directory that holds the release's files in place of the
# download, and UNA_BIN which `una` checks the samples (default: target/release/una). With a
# learn.lock that names a release, it needs `gh`, signed in with read access to una-by-example.
#
# Usage: scripts/site-website.sh BUNDLE OUT
#   BUNDLE  the bundle directory, OUT/una-docs-<version> of scripts/site-bundle.sh
#   OUT     a directory for the website's clone and build
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

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

website=${out:?}/website
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
    learn=${out:?}/una-by-example
    rm -rf "${learn:?}"
    git clone --quiet --no-checkout "${UNA_SITE_LEARN_SOURCE:-$learn_repository}" "$learn" || fail "cannot clone una-by-example"
    git -C "$learn" checkout --quiet --detach "$learn_commit" || fail "una-by-example has no commit $learn_commit"
    [ "$(git -C "$learn" rev-parse HEAD)" = "$learn_commit" ] || fail "the una-by-example clone is not $learn_commit"
    echo "una-by-example: $learn_commit"
    # -B: Python writes no bytecode into the clone, which check.py refuses.
    archives=${out:?}/una-by-example-archives
    rm -rf "${archives:?}"
    python3 -B "$learn/scripts/archive.py" "$archives" || fail "una-by-example's archives do not build"
    # The book is checked with the una its una.lock pins: this checkout's when the versions are
    # equal, as at release time, else that published release, verified (#1039).
    learn_una=$(scripts/site-learn-una.sh "$learn" "$una" "$out") || fail "the una that una-by-example pins"
    python3 -B "$learn/scripts/check.py" --una "$learn_una" --una-version-from-binary --archives "$archives" \
        || fail "an example of una-by-example does not check with the una its una.lock pins"
    # The release the Learn pages link their downloads to (una#831): when learn.lock names one,
    # it must hold exactly the files this build makes, byte for byte (scripts/site-learn-release.sh).
    learn_release=$(scripts/site-learn-release.sh --release-of "$website/learn.lock") || fail "the website's learn.lock"
    scripts/site-learn-release.sh "$website/learn.lock" "$archives" "$out" || fail "the Learn release"
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
if [ -n "$learn" ]; then
    # The website's own reading of learn.lock (scripts/preview/learn.mjs, readLock), the one its
    # pages' clone and download links come from, must name the repository, the commit and the
    # release the gate read, each line by line (una#831): a lock that reads one way to the website
    # and another to the gate fails.
    # Both sides as JSON, so no value can hold the separator between them (una#851).
    website_lock=$(node --input-type=module -e 'import { readFileSync } from "node:fs"; import { readLock } from "./scripts/preview/learn.mjs"; const l = readLock(readFileSync("learn.lock", "utf8")); process.stdout.write(JSON.stringify([l.repository, l.commit, l.release ?? ""]))') \
        || fail "the website cannot read its learn.lock"
    gate_lock=$(python3 -I -c 'import json, sys; sys.stdout.write(json.dumps(sys.argv[1:], separators=(",", ":"), ensure_ascii=False))' "$learn_repository" "$learn_commit" "$learn_release") \
        || fail "cannot write the gate's reading of learn.lock"
    [ "$website_lock" = "$gate_lock" ] \
        || fail "the website reads learn.lock as $website_lock, the gate as $gate_lock"
fi
node "$root/scripts/doc-links/website-anchors.mjs" "$out/anchors/ids.json" "$root" \
    || fail "the heading ids of scripts/doc-links.py are not the website's"
UNA_BUNDLE=$bundle UNA_BY_EXAMPLE=$learn npm run --silent build >"$out/website-build.log" 2>&1 \
    || { cat "$out/website-build.log" >&2; fail "the website does not build from the bundle"; }
cat "$out/website-build.log"
if [ -n "$learn" ]; then
    grep -q '^learn: [1-9][0-9]* pages from una-by-example' "$out/website-build.log" \
        || fail "the website built no Learn pages from una-by-example"
fi
UNA_BUNDLE=$bundle UNA_BY_EXAMPLE=$learn UNA_BIN=$una npm run --silent check:samples || fail "a code sample of the website does not check"
[ -z "$(git status --porcelain --untracked-files=no)" ] || fail "the website build changed tracked files"

echo "site-website: PASS (website $commit, bundle $(sed -n 's/.*"version":"\([^"]*\)".*"commit":"\([0-9a-f]\{12\}\).*/una \1 at \2/p' "$bundle/manifest.json" | head -n 1))"
