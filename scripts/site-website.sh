#!/bin/sh
# Builds the website, una-lang/website at the commit that site/website.lock pins, from a docs
# bundle of scripts/site-bundle.sh (#584), and checks it: the website's build renders every
# page from the bundle and fails on a broken budget, and its sample check runs every code
# sample of the site with this checkout's `una` (Z3 at the pinned version on PATH). Before the
# build, the heading ids that scripts/doc-links.py checks fragments against are compared with
# the ones the website gives the same documents (#708).
#
# It needs git access to the website's repository, Node.js (>= 20.19) with npm, and Python 3. It installs
# the website's packages with `npm ci --ignore-scripts`, from its package-lock.json, so no
# install script runs. It writes only under OUT. UNA_SITE_WEBSITE_SOURCE overrides where the
# website is cloned from, and UNA_BIN which `una` checks the samples (default:
# target/release/una).
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

website=$out/website
rm -rf "${website:?}"
git clone --quiet --no-checkout "$source" "$website" || fail "cannot clone $source"
git -C "$website" checkout --quiet --detach "$commit" || fail "the website has no commit $commit"
[ "$(git -C "$website" rev-parse HEAD)" = "$commit" ] || fail "the website clone is not $commit"
echo "website: $commit"

cd "$website"
# Supply chain: only the exact packages of the pinned lockfile, under its integrity hashes,
# and no package's install script runs on the release machine.
[ -f package-lock.json ] || fail "the website has no package-lock.json at $commit"
npm ci --ignore-scripts --no-audit --no-fund --loglevel=error || fail "npm ci"
node "$root/scripts/doc-links/website-anchors.mjs" "$out/anchors/ids.json" "$root" \
    || fail "the heading ids of scripts/doc-links.py are not the website's"
UNA_BUNDLE=$bundle npm run --silent build || fail "the website does not build from the bundle"
UNA_BUNDLE=$bundle UNA_BIN=$una npm run --silent check:samples || fail "a code sample of the website does not check"
[ -z "$(git status --porcelain --untracked-files=no)" ] || fail "the website build changed tracked files"

echo "site-website: PASS (website $commit, bundle $(sed -n 's/.*"version":"\([^"]*\)".*"commit":"\([0-9a-f]\{12\}\).*/una \1 at \2/p' "$bundle/manifest.json" | head -n 1))"
