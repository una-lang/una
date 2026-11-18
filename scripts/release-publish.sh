#!/bin/sh
# Prints the command that publishes a release (#717, docs/release-gates.md §4): its notes are
# OUT/release-notes.md (the validation summary, then the version's CHANGELOG section), and its
# assets are the archives in BUILT with their .sha256 files, then the summary as
# una-<version>-validation.md. A pre-release, a version with a `-` suffix such as
# 0.4.0-alpha.1, is published with --prerelease --latest=false, as scripts/local-release.sh
# does. It publishes nothing.
#
# Usage: scripts/release-publish.sh VERSION OUT BUILT
set -eu

[ $# -eq 3 ] || {
    echo "usage: scripts/release-publish.sh VERSION OUT BUILT" >&2
    exit 2
}
version=$1
out=$2
built=$3
prerelease=""
case "$version" in
    *-*) prerelease=" --prerelease --latest=false" ;;
esac
echo "publish, once the record commit is tagged v$version (docs/release-gates.md §4):"
echo "  gh release create v$version --verify-tag --title \"Una v$version\" --notes-file $out/release-notes.md$prerelease \\"
for archive in "$built"/*.tar.gz; do
    echo "    $archive $archive.sha256 \\"
done
echo "    $out/una-$version-validation.md"
