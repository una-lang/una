#!/bin/sh
# Assembles the changelog fragments into CHANGELOG.md (#744, CONTRIBUTING.md, docs/release-gates.md
# §4). A pull request never edits CHANGELOG.md's `## Unreleased` section: it adds one fragment,
# changelog.d/<issue>-<slug>.md, a single line that is one bullet, `- **…** (#<issue>…`. Two pull
# requests then never change the same lines, so merging one never makes the other conflict.
#
# The release pull request runs this script. It checks every fragment, writes their bullets under
# `## Unreleased` in a fixed order (the issue number, then the file name), and removes the
# fragments; the release then renames `## Unreleased` to `## v<version>` as before. The order
# depends only on the file names, so two runs on the same fragments give the same bytes.
#
#   --check   checks the fragments and that `## Unreleased` is empty, and changes nothing
#
# Exit code: 0 done, 1 a fragment out of shape or a non-empty `## Unreleased`, 2 a usage error.
#
# Usage: scripts/changelog-assemble.sh [--check] [ROOT]
set -eu

check=no
if [ "${1:-}" = --check ]; then
    check=yes
    shift
fi
[ $# -le 1 ] || {
    echo "usage: scripts/changelog-assemble.sh [--check] [ROOT]" >&2
    exit 2
}
root=${1:-$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)}
changelog=$root/CHANGELOG.md
fragments=$root/changelog.d

fail() {
    echo "changelog: FAIL ($1)" >&2
    exit 1
}

[ -f "$changelog" ] || fail "no CHANGELOG.md in $root"
grep -qx '## Unreleased' "$changelog" || fail "CHANGELOG.md has no ## Unreleased heading"

# The `## Unreleased` section holds no entry: entries live in fragments until a release.
entries=$(awk '/^## Unreleased$/ { inside = 1; next } inside && /^## / { exit } inside && NF { print }' "$changelog")
[ -z "$entries" ] || fail "## Unreleased has entries; a pull request adds a fragment in changelog.d/ instead"

# The fragments, in their fixed order: the issue number as a number, then the whole name.
list=""
if [ -d "$fragments" ]; then
    list=$(cd "$fragments" && for f in *; do [ -e "$f" ] && printf '%s\n' "$f"; done | LC_ALL=C sort -t- -k1,1n -k1)
fi
for name in $list; do
    case $name in
        *[!a-z0-9.-]* | .* ) fail "changelog.d/$name: a name is <issue>-<slug>.md, in lower case" ;;
    esac
    issue=${name%%-*}
    slug=${name#*-}
    case $issue in '' | *[!0-9]*) fail "changelog.d/$name: the name starts with the issue number" ;; esac
    case $slug in *.md) ;; *) fail "changelog.d/$name: the name ends in .md" ;; esac
    [ "$slug" != ".md" ] || fail "changelog.d/$name: the name has a slug after the issue number"
    file=$fragments/$name
    [ -f "$file" ] || fail "changelog.d/$name is not a file"
    lines=$(wc -l < "$file" | tr -d ' ')
    [ "$lines" = 1 ] && [ "$(tail -c 1 "$file" | od -An -c | tr -d ' ')" = '\n' ] \
        || fail "changelog.d/$name: a fragment is exactly one line, ending in a newline"
    grep -q '^- \*\*[^*].*\*\* (' "$file" || fail "changelog.d/$name: the line is one bullet, - **Title** (#${issue}…"
    grep -q "(#$issue[,)]" "$file" || fail "changelog.d/$name: the bullet names its issue first, (#${issue}"
done
[ "$check" = no ] || exit 0
[ -n "$list" ] || {
    echo "changelog: no fragment to assemble"
    exit 0
}

# The bullets under `## Unreleased`, then the fragments removed. awk reads the bullets from a
# file, since a -v value cannot hold a newline in every awk.
bullets=$changelog.bullets
tmp=$changelog.tmp
for name in $list; do cat "$fragments/$name"; done > "$bullets"
awk -v bullets="$bullets" '
    { print }
    /^## Unreleased$/ && !done {
        print ""
        while ((getline line < bullets) > 0) print line
        done = 1
    }
' "$changelog" > "$tmp"
mv "$tmp" "$changelog"
rm -f "$bullets"
for name in $list; do
    rm -- "${fragments:?}/$name"
done
echo "changelog: $(printf '%s\n' "$list" | wc -l | tr -d ' ') fragments assembled under ## Unreleased"
