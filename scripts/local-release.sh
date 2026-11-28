#!/bin/sh
# Builds the release archives of TAG on this machine while GitHub Actions is unavailable (#156,
# ADR 0035 amendment of 2026-10-02): on a clean checkout of the tag's commit, the archive of each
# target that scripts/release-targets.txt lists and of no other (#1903), in its order: the
# aarch64-apple-darwin archive natively, a Linux one in the container of scripts/local-ci/Dockerfile.release
# of its architecture (linux/arm64, native here, #241; linux/amd64, emulated), each with
# scripts/release.sh and checked with scripts/check-release.sh, but for the emulated linux/amd64
# one, which must equal the archive the hosted x86-64 runner checked natively (#2106). It writes the archives, their .sha256 files and the release
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
. "$root/scripts/release-targets.sh"
targets=$(release_targets "$root/scripts/release-targets.txt") \
    || fail "the shipped targets of scripts/release-targets.txt"

# A clean checkout of the tag's commit, outside the working tree, removed at the end.
work=$(mktemp -d "${TMPDIR:-/tmp}/una-release.XXXXXX")
checkout="$work/checkout"
cleanup() {
    git -C "$root" worktree remove --force "$checkout" 2>/dev/null
    rm -rf "${work:?}"
}
trap cleanup EXIT
git worktree add --detach --quiet "$checkout" "$commit" || fail "checking out $commit"
# The archives are the tag's commit's: its own list, which must also be this checkout's, since the
# recipes below are this checkout's (#1903). A tag with no list, or another list, fails here.
tag_targets=$(release_targets "$checkout/scripts/release-targets.txt") \
    || fail "the shipped targets of $commit"
[ "$tag_targets" = "$targets" ] \
    || fail "the shipped targets of $commit are not those of this checkout's scripts/release-targets.txt"

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
# builds a second time in volumes of its own, from cold (#679), named by UNA_LOCAL_RELEASE_VOLUME,
# which is used as given (plus the architecture's suffix). The kept volume is named after the
# release image (#2272): una-local-release-target-<image>, where <image> is the first 12 hex
# digits of the ID of the image built from scripts/local-ci/Dockerfile.release for the toolchain,
# so that a new image (another Dockerfile, toolchain or base layer) starts from a cold volume. A
# volume kept through a change of image held fingerprints that outlived their rlibs, and the new
# image took them as fresh (#2271: E0463, can't find crate for serde).
volume_prefix=una-local-release-target
volume_override=${UNA_LOCAL_RELEASE_VOLUME:-}

# The kept volumes of an architecture other than `keep`: every volume named `$volume_prefix`
# (the name before #2272) or `$volume_prefix-<anything>`, whose name ends with `suffix` (-arm64)
# or, for no suffix, does not end with -arm64. `docker volume rm` refuses a volume in use, which
# is left.
stale_volumes() {
    keep=$1 suffix=$2
    docker volume ls --quiet | while IFS= read -r name; do
        case $name in
            "$volume_prefix" | "$volume_prefix"-*) ;;
            *) continue ;;
        esac
        [ "$name" = "$keep" ] && continue
        case $suffix in
            '') case $name in *-arm64) continue ;; esac ;;
            *) case $name in *"$suffix") ;; *) continue ;; esac ;;
        esac
        printf '%s\n' "$name"
    done
}

# No standalone Z3 is installed (#1902): una embeds the Z3 library (ADR 0124), which
# scripts/release.sh takes from the cache by the digest supply-chain/z3.lock records for each
# target, never rebuilding it, and fails (exit 2) when no entry holds that digest; and
# scripts/check-release.sh runs no z3. On this machine the cache is UNA_Z3_CACHE, else
# ~/.cache/una/z3.

# The macOS archive, natively; the host is aarch64-apple-darwin (checked above).
mac_archive() {
    echo "== $1, natively, from $commit"
    (
        cd "$checkout" || exit 1
        scripts/release.sh "$1" && scripts/check-release.sh "$1"
    ) || fail "the macOS archive"
    cp "$checkout/target/release-archives/una-$version-$1.tar.gz" \
        "$checkout/target/release-archives/una-$version-$1.tar.gz.sha256" "$out/" \
        || fail "copying the macOS archive"
}

toolchain=$(sed -n 's/^channel *= *"\([^"]*\)"$/\1/p' "$checkout/rust-toolchain.toml")

# The Linux archive of `target`, in a container of `arch` (amd64, emulated on an arm64 Mac, or
# arm64, native on one, #241), with the image, the target directory and the Z3 library cache of
# that architecture: the volume of scripts/local-ci.sh, mounted read-only at /opt/z3-cache and
# named by UNA_Z3_CACHE, where scripts/release.sh finds the library by its pinned digest (#1902).
# The reproducible gate's second build, with target volumes of its own, reads the same cache:
# the library is checked by digest, not rebuilt (ADR 0124 rule 31). The image comes from this checkout's scripts/local-ci, which a tag made before
# it lacks; the toolchain is the one the tag's commit pins.
linux_archive() {
    arch=$1 target=$2 suffix=$3
    echo "== $target, in the linux/$arch container, from $commit"
    # The release image (#2105): AlmaLinux 8, glibc 2.28, the toolchain supply-chain/z3.lock
    # records; the tests run in the Ubuntu image of scripts/local-ci/Dockerfile.
    image="una-local-ci-release:$toolchain$suffix"
    docker build --platform "linux/$arch" --build-arg "TOOLCHAIN=$toolchain" \
        --file "$root/scripts/local-ci/Dockerfile.release" --tag "$image" \
        "$root/scripts/local-ci" || fail "the container image for linux/$arch did not build"
    if [ -n "$volume_override" ]; then
        volume="$volume_override$suffix"
    else
        image_id=$(docker image inspect --format '{{.Id}}' "$image") \
            || fail "reading the ID of the image for linux/$arch"
        image_id=${image_id#sha256:}
        case $image_id in
            [0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]*) ;;
            *) fail "the image for linux/$arch has no ID of hex digits: '$image_id'" ;;
        esac
        volume="$volume_prefix-$(printf '%s' "$image_id" | cut -c1-12)$suffix"
    fi
    echo "== the target volume of linux/$arch: $volume"
    name="una-$version-$target.tar.gz"
    # The image runs as an unprivileged user, uid 1000 (#1394); the volumes are handed to it first.
    "$root/scripts/container-volumes.sh" "linux/$arch" "$image" "$volume:/work/target" \
        una-local-ci-registry:/opt/cargo/registry "una-local-ci-z3-cache$suffix:/opt/z3-cache" \
        || fail "handing the container's volumes to its user for linux/$arch"
    # The x86-64 archive on a machine whose linux/amd64 container is emulated (#2106, ADR 0124
    # rule 31 as its amendment of 2026-11-27 reads it): scripts/check-release.sh starts solver
    # children, whose seccomp filter does not install under emulation (#1442), so it cannot run
    # here. The archive is built here all the same, and must be byte for byte the one that the
    # hosted x86-64 runner packed and checked natively on this commit (scripts/hosted-evidence.sh);
    # no other archive, and no unchecked one, is kept.
    check="scripts/check-release.sh $target"
    hosted=""
    if [ "$arch" = amd64 ] && ! "$root/scripts/hosted-evidence.sh" --native-engine; then
        check="echo the check: the archive the hosted runner checked natively, below"
        hosted=1
    fi
    docker run --rm --platform "linux/$arch" \
        --volume "$checkout:/work" \
        --volume "$volume:/work/target" \
        --env "SOURCE_DATE_EPOCH=$SOURCE_DATE_EPOCH" \
        --volume una-local-ci-registry:/opt/cargo/registry \
        --volume "una-local-ci-z3-cache$suffix:/opt/z3-cache:ro" \
        --env UNA_Z3_CACHE=/opt/z3-cache \
        --volume "$out:/out" \
        "$image" sh -c "set -e
            scripts/release.sh $target
            $check
            cp target/release-archives/$name target/release-archives/$name.sha256 /out/" \
        || fail "the Linux archive of $target"
    if [ -n "$hosted" ]; then
        # The evidence stays where the caller keeps it (UNA_HOSTED_DIR: the release gates give
        # OUT/hosted/release and OUT/hosted/reproducible), and its whole output, the runner's
        # log included, goes into this output (ADR 0124's amendment of 2026-11-27, item 4; #2327).
        hosted_dir=${UNA_HOSTED_DIR:-$root/target/local-release/$tag.hosted}
        evidence=$("$root/scripts/hosted-evidence.sh" "$commit" archive "$hosted_dir")
        evidence_status=$?
        printf '%s\n' "$evidence"
        if [ "$evidence_status" != 0 ]; then
            rm -f "$out/$name" "$out/$name.sha256"
            fail "the hosted x86-64 evidence of $commit"
        fi
        ours=$(shasum -a 256 "$out/$name" | cut -d ' ' -f 1)
        if ! printf '%s\n' "$evidence" | grep -qx "hosted: archive $name sha256 $ours"; then
            rm -f "$out/$name" "$out/$name.sha256"
            fail "$name built here is $ours, not the archive the hosted runner checked natively"
        fi
        echo "== $name: sha256 $ours, the archive the hosted runner checked natively"
    fi
    # Only the volume of the current image is kept: once its build passed, the run removes the
    # architecture's other kept volumes, those of earlier images (#2272).
    if [ -z "$volume_override" ]; then
        for stale in $(stale_volumes "$volume" "$suffix"); do
            if docker volume rm "$stale" > /dev/null 2>&1; then
                echo "== removed the target volume of an earlier image: $stale"
            else
                echo "== left the target volume $stale (docker volume rm refused it)"
            fi
        done
    fi
}
for target in $targets; do
    case $(release_target_platform "$target") in
        macos) mac_archive "$target" ;;
        linux/arm64) linux_archive arm64 "$target" -arm64 ;;
        linux/amd64) linux_archive amd64 "$target" "" ;;
        *) fail "no recipe for the archive of $target" ;;
    esac
done

echo "== digests"
(cd "$out" && shasum -a 256 -c ./*.sha256) || fail "a digest does not match its archive"
# Exactly one archive per shipped target, and no other.
expected=$(for target in $targets; do echo "una-$version-$target.tar.gz"; done | LC_ALL=C sort)
built=$(cd "$out" && ls ./*.tar.gz 2>/dev/null | sed 's|^\./||' | LC_ALL=C sort)
[ "$built" = "$expected" ] || fail "the archives are not exactly those of scripts/release-targets.txt"

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
last=$(printf '%s\n' "$targets" | tail -n 1)
for target in $targets; do
    continued=" \\"
    [ "$target" = "$last" ] && continued=""
    echo "    $out/una-$version-$target.tar.gz $out/una-$version-$target.tar.gz.sha256$continued"
done
