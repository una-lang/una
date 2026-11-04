#!/bin/sh
# Builds the release archives of TAG on this machine while GitHub Actions is unavailable (#156,
# ADR 0035 amendment of 2026-10-02): on a clean checkout of the tag's commit, the
# aarch64-apple-darwin archive natively, the x86_64-unknown-linux-gnu archive in the linux/amd64
# container of scripts/local-ci/Dockerfile and the aarch64-unknown-linux-gnu archive in its
# linux/arm64 one (#241), each with scripts/release.sh and checked with scripts/check-release.sh. It writes the archives, their .sha256 files and the release
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
# The archives' entries take the commit's time (#679), which the containers cannot read: the
# checkout's .git is a worktree link to this repository, outside their mount.
SOURCE_DATE_EPOCH=$(git log -1 --format=%ct "$commit") || fail "reading the time of $commit"
export SOURCE_DATE_EPOCH
# The Linux builds' target directories, volumes kept between releases; the reproducible gate
# builds a second time in volumes of its own, from cold (#679).
volume=${UNA_LOCAL_RELEASE_VOLUME:-una-local-release-target}

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

toolchain=$(sed -n 's/^channel *= *"\([^"]*\)"$/\1/p' "$checkout/rust-toolchain.toml")

# The Linux archive of `target`, in a container of `arch` (amd64, emulated on an arm64 Mac, or
# arm64, native on one, #241), with the image, the target directory and Z3 of that
# architecture. The image comes from this checkout's scripts/local-ci, which a tag made before
# it lacks; the toolchain is the one the tag's commit pins.
linux_archive() {
    arch=$1 target=$2 suffix=$3
    echo "== $target, in the linux/$arch container, from $commit"
    image="una-local-ci:$toolchain$suffix"
    docker build --platform "linux/$arch" --build-arg "TOOLCHAIN=$toolchain" --tag "$image" \
        "$root/scripts/local-ci" || fail "the container image for linux/$arch did not build"
    name="una-$version-$target.tar.gz"
    # The image runs as an unprivileged user, uid 1000 (#1394); the volumes are handed to it first.
    "$root/scripts/container-volumes.sh" "linux/$arch" "$image" "$volume$suffix:/work/target" \
        una-local-ci-registry:/opt/cargo/registry "una-local-ci-z3$suffix:/opt/z3" \
        || fail "handing the container's volumes to its user for linux/$arch"
    docker run --rm --platform "linux/$arch" \
        --volume "$checkout:/work" \
        --volume "$volume$suffix:/work/target" \
        --env "SOURCE_DATE_EPOCH=$SOURCE_DATE_EPOCH" \
        --volume una-local-ci-registry:/opt/cargo/registry \
        --volume "una-local-ci-z3$suffix:/opt/z3" \
        --volume "$out:/out" \
        "$image" sh -c "set -e
            [ -x /opt/z3/bin/z3 ] || scripts/install-z3.sh /opt/z3
            scripts/release.sh $target
            scripts/check-release.sh $target
            cp target/release-archives/$name target/release-archives/$name.sha256 /out/" \
        || fail "the Linux archive of $target"
}
linux_archive amd64 x86_64-unknown-linux-gnu ""
linux_archive arm64 aarch64-unknown-linux-gnu -arm64

echo "== digests"
(cd "$out" && shasum -a 256 -c ./*.sha256) || fail "a digest does not match its archive"

# The notes: the version's CHANGELOG section, and where the archives come from.
notes="$out/notes.md"
awk -v heading="## v$version" '
    $0 == heading { inside = 1; next }
    inside && /^## / { exit }
    inside { print }
' "$checkout/CHANGELOG.md" | sed '1{/^$/d;}' | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}' > "$notes"
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
for target in aarch64-apple-darwin x86_64-unknown-linux-gnu; do
    echo "    $out/una-$version-$target.tar.gz $out/una-$version-$target.tar.gz.sha256 \\"
done
echo "    $out/una-$version-aarch64-unknown-linux-gnu.tar.gz $out/una-$version-aarch64-unknown-linux-gnu.tar.gz.sha256"
