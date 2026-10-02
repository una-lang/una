#!/bin/sh
# Builds the release archives of TAG on this machine while GitHub Actions is unavailable (#156,
# ADR 0035 amendment of 2026-10-02): on a clean checkout of the tag's commit, the
# aarch64-apple-darwin archive natively and the x86_64-unknown-linux-gnu archive in the
# linux/amd64 container of scripts/local-ci/Dockerfile, each with scripts/release.sh and checked
# with scripts/check-release.sh. It writes the archives, their .sha256 files and the release
# notes to target/local-release/TAG/, and prints the `gh release create` command that publishes
# them. It publishes nothing.
#
# The tag must exist locally and be `v` followed by the workspace version of its commit;
# --allow-version-mismatch lifts that check for a dry run under another name.
#
# Usage: scripts/local-release.sh [--allow-version-mismatch] TAG
set -u

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

fail() {
    echo "local release: FAIL ($1)" >&2
    exit 1
}

mismatch=no
if [ "${1:-}" = --allow-version-mismatch ]; then
    mismatch=yes
    shift
fi
if [ "$#" -ne 1 ]; then
    echo "usage: scripts/local-release.sh [--allow-version-mismatch] TAG" >&2
    exit 2
fi
tag=$1
commit=$(git rev-parse --verify --quiet "refs/tags/$tag^{commit}") \
    || fail "no tag $tag here; fetch it with git fetch --tags"
host=$(rustc -vV | sed -n 's/^host: //p')
[ "$host" = aarch64-apple-darwin ] \
    || fail "this machine builds $host; the macOS archive needs aarch64-apple-darwin"
command -v docker >/dev/null 2>&1 || fail "docker not found; the Linux archive needs Docker"

# A clean checkout of the tag's commit, outside the working tree, removed at the end.
work=$(mktemp -d "${TMPDIR:-/tmp}/una-release.XXXXXX")
checkout="$work/checkout"
cleanup() {
    git -C "$root" worktree remove --force "$checkout" 2>/dev/null
    rm -rf "${work:?}"
}
trap cleanup EXIT
git worktree add --detach --quiet "$checkout" "$commit" || fail "checking out $commit"

version=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([^"]*\)"$/\1/p' \
    "$checkout/Cargo.toml")
if [ "$tag" != "v$version" ] && [ "$mismatch" = no ]; then
    fail "$tag is not v$version, the workspace version of $commit"
fi
out="$root/target/local-release/$tag"
rm -rf "${out:?}"
mkdir -p "$out"

# Z3, which check-release.sh needs, as scripts/local-ci.sh installs it.
z3="$root/target/z3"
if [ ! -x "$z3/bin/z3" ]; then
    scripts/install-z3.sh "$z3" || fail "installing Z3"
fi

echo "== aarch64-apple-darwin, natively, from $commit"
(
    cd "$checkout" || exit 1
    PATH="$z3/bin:$PATH"
    export PATH
    scripts/release.sh aarch64-apple-darwin && scripts/check-release.sh aarch64-apple-darwin
) || fail "the macOS archive"
cp "$checkout/target/release-archives/una-$version-aarch64-apple-darwin.tar.gz" \
    "$checkout/target/release-archives/una-$version-aarch64-apple-darwin.tar.gz.sha256" "$out/" \
    || fail "copying the macOS archive"

echo "== x86_64-unknown-linux-gnu, in the linux/amd64 container, from $commit"
toolchain=$(sed -n 's/^channel *= *"\([^"]*\)"$/\1/p' "$checkout/rust-toolchain.toml")
image="una-local-ci:$toolchain"
# The image comes from this checkout's scripts/local-ci, which a tag made before it lacks; the
# toolchain is the one the tag's commit pins.
docker build --platform linux/amd64 --build-arg "TOOLCHAIN=$toolchain" --tag "$image" \
    "$root/scripts/local-ci" || fail "the container image did not build"
name="una-$version-x86_64-unknown-linux-gnu.tar.gz"
docker run --rm --platform linux/amd64 \
    --volume "$checkout:/work" \
    --volume una-local-release-target:/work/target \
    --volume una-local-ci-registry:/opt/cargo/registry \
    --volume una-local-ci-z3:/opt/z3 \
    --volume "$out:/out" \
    "$image" sh -c "set -e
        [ -x /opt/z3/bin/z3 ] || scripts/install-z3.sh /opt/z3
        scripts/release.sh x86_64-unknown-linux-gnu
        scripts/check-release.sh x86_64-unknown-linux-gnu
        cp target/release-archives/$name target/release-archives/$name.sha256 /out/" \
    || fail "the Linux archive"

echo "== digests"
(cd "$out" && shasum -a 256 -c ./*.sha256) || fail "a digest does not match its archive"

# The notes: the version's CHANGELOG section, and where the archives come from.
notes="$out/notes.md"
awk -v heading="## v$version" '
    $0 == heading { inside = 1; next }
    inside && /^## / { exit }
    inside { print }
' "$checkout/CHANGELOG.md" | sed '1{/^$/d;}' > "$notes"
[ -s "$notes" ] || fail "CHANGELOG.md of $commit has no section ## v$version"
printf '\nThe archives were built locally from commit %s with `scripts/local-release.sh`, while GitHub Actions is unavailable.\n' "$commit" >> "$notes"

prerelease=""
case "$tag" in
    *-*) prerelease=" --prerelease --latest=false" ;;
esac
echo "local release: PASS ($tag, commit $commit)"
echo "archives, digests and notes: $out"
echo "publish with:"
echo "  gh release create $tag --verify-tag --title \"Una $tag\" --notes-file $notes$prerelease \\"
echo "    $out/una-$version-aarch64-apple-darwin.tar.gz $out/una-$version-aarch64-apple-darwin.tar.gz.sha256 \\"
echo "    $out/$name $out/$name.sha256"
