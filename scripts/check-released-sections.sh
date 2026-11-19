#!/bin/sh
# Checks that every released section of CHANGELOG.md is the one published with its tag (#645).
# For each tag vX of this repository, v0.3.0 and later, the `## vX` section of CHANGELOG.md
# (from its heading to the next `## ` heading) must be byte-identical to the `## vX` section of
# CHANGELOG.md at tag vX. scripts/local-release.sh publishes that section as the release notes,
# so an entry that a rebase carries under a released heading, instead of `## Unreleased`, fails
# here before it merges. Each differing version is named.
#
# A released section that was amended on purpose after its tag is recorded in
# scripts/released-sections-amended.txt (#2111), one line per amendment: the tag, the SHA-256 of
# the amended section as this script extracts it (heading included), and the card that decided
# it, as `#<number>`. Blank lines and lines that start with `#` are comments. A section that
# differs from its tag passes only when its digest is the one recorded for that tag, and is
# named `amended`; any other text still fails, an amended section edited further included. A
# record whose section is identical to its tag is stale and fails, as do a line out of that
# shape, two lines for one tag, and a tag this check does not read. The record is read from the
# working tree, or from REV when given, as CHANGELOG.md is; without the file there is no
# amendment. A section that differs is printed with its digest. The tag and the release notes
# published from it keep the original text.
#
# It runs in the git repository of the current directory, and checks its working tree's
# CHANGELOG.md, or the one at REV when given. Every such tag is checked, so a commit that
# predates a release fails until it is rebased onto it. With no tag v0.3.0 or later, it fails:
# a shallow clone or a copy without tags cannot be checked, and passing there would hide a
# drift. Where that is known and legitimate, UNA_SKIP_RELEASED_SECTIONS set to the reason skips
# the check, visibly. scripts/local-ci.sh and scripts/release-gates.sh never take it from the
# caller's shell. A section ends at the next `## ` heading outside a fenced block.
#
# UNA_RELEASED_SECTIONS_SCOPE=ancestors checks only the tags the checked commit contains, and
# names each newer one as not checked. scripts/release-gates.sh sets it: its gates may run on a
# past release's commit after newer tags exist, and that commit was never meant to hold their
# sections. A pull request's branch keeps the default, every tag, so that it is rebased over each
# release before it merges.
#
# Usage: scripts/check-released-sections.sh [REV]
set -u

if [ "$#" -gt 1 ]; then
    echo "usage: scripts/check-released-sections.sh [REV]" >&2
    exit 2
fi
if [ -n "${UNA_SKIP_RELEASED_SECTIONS:-}" ]; then
    echo "skipped: $UNA_SKIP_RELEASED_SECTIONS (UNA_SKIP_RELEASED_SECTIONS)"
    exit 0
fi
rev=${1:-}
if [ -n "$rev" ]; then
    # A commit, never an option: what follows uses its full name.
    commit=$(git rev-parse --verify --quiet --end-of-options "$rev^{commit}" 2>/dev/null) || {
        echo "error: \`$rev\` is not a commit of this repository" >&2
        exit 2
    }
    rev=$commit
fi
scope=${UNA_RELEASED_SECTIONS_SCOPE:-all}
case $scope in
    all | ancestors) ;;
    *)
        echo "error: UNA_RELEASED_SECTIONS_SCOPE is \`all\` or \`ancestors\`, not \`$scope\`" >&2
        exit 2
        ;;
esac

top=$(git rev-parse --show-toplevel 2>/dev/null) || {
    echo "error: not in a git repository that git can read; the released sections need its tags" >&2
    exit 1
}
cd "$top" || exit 1

# The tags v0.3.0 and later, oldest first: vMAJOR.MINOR.PATCH, with any suffix, whose version
# is at least 0.3.
tags=$(git tag -l 'v[0-9]*' | awk -F'[v.-]' '
    $0 ~ /^v[0-9]+\.[0-9]+\.[0-9]+/ && ($2 > 0 || $3 >= 3) { print }' | sort -V)
if [ -z "$tags" ]; then
    # A clone has the tags of the repository it was cloned from: release-gates' clone those of
    # its source checkout.
    source=$(git remote get-url origin 2>/dev/null)
    fetch="fetch the tags (git fetch --tags)"
    if [ -n "$source" ]; then
        fetch="$fetch in the repository this one was cloned from, $source, and clone again,"
    fi
    echo "error: no tag v0.3.0 or later in this repository, so the released sections cannot be" \
        "checked; $fetch or clone without --depth" >&2
    exit 1
fi
# A shallow history ends early, so whether the checked commit contains a tag cannot be told, and
# git answers "no" at the cut instead of failing: under the ancestors scope every tag would be
# skipped. So a shallow repository fails, whatever the scope.
if [ "$(git rev-parse --is-shallow-repository 2>/dev/null)" != false ]; then
    echo "error: this repository's history is shallow, or git cannot tell, so the released" \
        "sections cannot be checked; fetch the whole history (git fetch --tags --unshallow)" >&2
    exit 1
fi

scratch=$(mktemp -d) || exit 1
trap 'rm -rf "${scratch:?}"' EXIT

# CHANGELOG.md at revision $1, or in the working tree when $1 is empty, into file $2.
changelog() {
    if [ -z "$1" ]; then
        cat CHANGELOG.md > "$2"
    else
        git show "$1:CHANGELOG.md" > "$2"
    fi
}

# Whether the checked commit contains tag $1: 0 yes, 1 no, and any other status when git cannot
# tell, which the caller fails on.
contains() {
    git merge-base --is-ancestor "$1" "${rev:-HEAD}" 2>/dev/null
}

# The SHA-256 of file $1, in lower-case hexadecimal; nothing where no tool computes one, which
# matches no record.
digest() {
    if command -v sha256sum > /dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    else
        shasum -a 256 "$1" 2> /dev/null | cut -d ' ' -f 1
    fi
}

# The section `## $2` of file $1: its heading and every line up to the next `## ` heading
# outside a fenced block, so that a `## ` line inside a fence does not end it.
section() {
    awk -v h="## $2" '
        !p && $0 == h { p = 1; print; next }
        p && /^```/ { fenced = !fenced }
        p && !fenced && /^## / { exit }
        p { print }' "$1"
}

changelog "$rev" "$scratch/checked" 2>/dev/null || {
    echo "error: no CHANGELOG.md ${rev:+at $rev }to check" >&2
    exit 1
}

# The recorded amendments, as `tag digest card` lines in $scratch/records: none without the
# file. A line out of shape fails here, before any section is judged.
amended=scripts/released-sections-amended.txt
: > "$scratch/raw"
if [ -z "$rev" ]; then
    if [ -e "$amended" ] || [ -L "$amended" ]; then
        cat "$amended" > "$scratch/raw" 2>/dev/null || {
            echo "error: $amended cannot be read" >&2
            exit 1
        }
    fi
elif git cat-file -e "$rev:$amended" 2>/dev/null; then
    git show "$rev:$amended" > "$scratch/raw" 2>/dev/null || {
        echo "error: $amended at $rev cannot be read" >&2
        exit 1
    }
fi
awk -v file="$amended" '
    /^[ \t]*$/ || /^[ \t]*#/ { next }
    NF != 3 || $1 !~ /^v[0-9]+\.[0-9]+\.[0-9]+[0-9A-Za-z.+-]*$/ \
        || length($2) != 64 || $2 !~ /^[0-9a-f]+$/ || $3 !~ /^#[0-9]+$/ {
        printf "error: %s line %d is not `<tag> <sha256, 64 lower-case hex digits> #<card>`\n", \
            file, NR > "/dev/stderr"
        bad = 1
        next
    }
    seen[$1]++ {
        printf "error: %s line %d records %s a second time\n", file, NR, $1 > "/dev/stderr"
        bad = 1
        next
    }
    { print $1, $2, $3 }
    END { exit bad }' "$scratch/raw" > "$scratch/records" || exit 1
while read -r tag _ card; do
    if ! printf '%s\n' "$tags" | grep -qxF -- "$tag"; then
        echo "error: $amended records $tag ($card), which is not a tag v0.3.0 or later of this" \
            "repository" >&2
        exit 1
    fi
done < "$scratch/records"

rc=0
amendments=0
for tag in $tags; do
    contains "$tag"
    contained=$?
    if [ "$contained" -gt 1 ]; then
        echo "$tag: FAIL, git cannot tell whether the checked commit contains it;" \
            "fetch the whole history (git fetch --tags --unshallow)"
        rc=1
        continue
    fi
    if [ "$scope" = ancestors ] && [ "$contained" -eq 1 ]; then
        echo "$tag: not checked, newer than the checked commit" \
            "(UNA_RELEASED_SECTIONS_SCOPE=ancestors)"
        continue
    fi
    if ! changelog "$tag" "$scratch/tagged" 2>/dev/null; then
        echo "$tag: FAIL, CHANGELOG.md at the tag cannot be read;" \
            "fetch the tags' history (git fetch --tags --unshallow)"
        rc=1
        continue
    fi
    section "$scratch/tagged" "$tag" > "$scratch/published"
    if [ ! -s "$scratch/published" ]; then
        echo "$tag: FAIL, CHANGELOG.md at the tag has no \`## $tag\` section"
        rc=1
        continue
    fi
    count=$(grep -cxF "## $tag" "$scratch/checked")
    if [ "$count" -eq 0 ] && [ "$contained" -eq 1 ]; then
        # A branch that predates the release: rebased onto it, it gets the section.
        echo "$tag: FAIL, the checked commit predates the release;" \
            "rebase it onto a commit that has $tag"
        rc=1
        continue
    fi
    if [ "$count" -ne 1 ]; then
        echo "$tag: FAIL, the checked CHANGELOG.md has $count \`## $tag\` headings, not one"
        rc=1
        continue
    fi
    section "$scratch/checked" "$tag" > "$scratch/current"
    recorded=$(awk -v t="$tag" '$1 == t { print $2 }' "$scratch/records")
    card=$(awk -v t="$tag" '$1 == t { print $3 }' "$scratch/records")
    if cmp -s "$scratch/published" "$scratch/current"; then
        if [ -n "$recorded" ]; then
            echo "$tag: FAIL, the section is identical to its tag, but $amended records an" \
                "amendment ($card); remove the stale record"
            rc=1
        else
            echo "$tag: identical"
        fi
        continue
    fi
    now=$(digest "$scratch/current")
    if [ -n "$recorded" ] && [ "$now" = "$recorded" ]; then
        echo "$tag: amended (recorded, $card)"
        amendments=$((amendments + 1))
        continue
    fi
    echo "$tag: DIFFERS from the section published with the tag"
    if [ -n "$recorded" ]; then
        echo "$tag: and from the amendment that $amended records ($card, sha256 $recorded)"
    fi
    diff -u "$scratch/published" "$scratch/current" | sed -n '3,$p' | head -20
    echo "$tag: sha256 of the checked section: ${now:-not computed, no sha256sum or shasum}"
    rc=1
done
if [ "$rc" -ne 0 ]; then
    echo "error: a released section of CHANGELOG.md is neither the one its tag published nor a" \
        "recorded amendment of it; move new entries under \`## Unreleased\`" >&2
elif [ "$amendments" -eq 0 ]; then
    echo "ok: every released section of CHANGELOG.md is the one its tag published"
else
    echo "ok: every released section of CHANGELOG.md is the one its tag published, or its" \
        "recorded amendment ($amendments amended)"
fi
exit "$rc"
