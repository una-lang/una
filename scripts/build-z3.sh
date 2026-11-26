#!/bin/sh
# Builds the Z3 that supply-chain/z3.lock pins, by the recipe of supply-chain/z3/ (ADR 0124
# rule 1): lib/libz3shell.a, the SMT-LIB front end as a library for una-z3, and bin/z3, the
# standalone front end from the same objects, for the solver-identity gate; with
# build-record.txt (pin, toolchain versions, the flags CMake added, time, digests).
#
# Usage: scripts/build-z3.sh [--source FILE] [--out DIR] [--jobs N] [--reproducible] [--key]
#   --source FILE   a local copy of the pinned tarball, instead of a download from the mirror
#   --out DIR       the cache root (default: UNA_Z3_CACHE, else ~/.cache/una/z3)
#   --jobs N        parallel compile jobs (default: the machine's processors)
#   --reproducible  two cold builds; fails unless both libraries have the same digest
#   --key           print the cache key of this machine's toolchain and the pin, and exit
#
# The output lands in <cache root>/<key>/, where the key is the SHA-256 over the pinned
# tarball's digest, each patch's digest in the lock's order, and the toolchain (the C++
# compiler's and cmake's version lines and the machine), so that one cache holds one build per
# toolchain and pin, outside every worktree and target directory. The directory holds
# lib/libz3shell.a, bin/z3, build-record.txt, configure.log and library.sha256, the library's
# digest, which scripts/local-ci.sh checks again before it uses the library: a cache entry
# whose bytes changed is refused and built again. The tarball's digest is checked against the
# lock before anything is extracted or compiled, and each patch's digest before it is applied;
# a mismatch stops the build. The tarball is the pristine upstream tree; every change of ours is
# a patch listed in the lock. Nothing is written outside the cache root and a temporary
# directory. It needs cmake, make, a C++20 compiler and git (which applies the recipe's patch).
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
lock="$root/supply-chain/z3.lock"
recipe="$root/supply-chain/z3"
source_file=
out=${UNA_Z3_CACHE:-$HOME/.cache/una/z3}
jobs=
reproducible=0
key_only=0
while [ "$#" -gt 0 ]; do
    case $1 in
        --source) shift; source_file=${1:?--source needs a file} ;;
        --out) shift; out=${1:?--out needs a directory} ;;
        --jobs) shift; jobs=${1:?--jobs needs a number} ;;
        --reproducible) reproducible=1 ;;
        --key) key_only=1 ;;
        *) echo "usage: scripts/build-z3.sh [--source FILE] [--out DIR] [--jobs N] [--reproducible] [--key]" >&2; exit 2 ;;
    esac
    shift
done
case $jobs in
    '') jobs=$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2) ;;
    *[!0-9]*|0) echo "error: --jobs needs a positive integer" >&2; exit 2 ;;
esac

fail() { echo "error: $*" >&2; exit 1; }
# A quoted string value of the lock, by key.
setting() { sed -n "s/^$1 *= *\"\([^\"]*\)\"\$/\1/p" "$lock"; }
digest() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d ' ' -f 1
    else shasum -a 256 "$1" | cut -d ' ' -f 1; fi
}
digest_stdin() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum | cut -d ' ' -f 1
    else shasum -a 256 | cut -d ' ' -f 1; fi
}

version=$(setting version)
commit=$(setting commit)
tree=$(setting tree)
source=$(setting source)
source_sha256=$(setting source_sha256)
source_tar_sha256=$(setting source_tar_sha256)
mirror=$(setting mirror)
for v in version commit tree source source_sha256 source_tar_sha256 mirror; do
    eval "[ -n \"\$$v\" ]" || fail "supply-chain/z3.lock has no $v"
done
[ -f "$recipe/cmake-options.txt" ] || fail "supply-chain/z3/cmake-options.txt is missing"
# The patches of the lock's [patches] table, `"name" = "sha256"`, in order, each checked
# against its digest before anything is applied.
patches=$(sed -n '/^\[patches\]$/,/^\[/s/^"\([^"]*\)" *= *"\([0-9a-f]*\)"$/\1 \2/p' "$lock")
[ -n "$patches" ] || fail "supply-chain/z3.lock has no [patches] table"
printf '%s\n' "$patches" | while read -r name sha; do
    [ -f "$recipe/$name" ] || fail "supply-chain/z3/$name is missing"
    actual=$(digest "$recipe/$name")
    [ "$actual" = "$sha" ] \
        || fail "SHA-256 mismatch for supply-chain/z3/$name: the lock pins $sha, the file is $actual"
done || exit 1
for tool in cmake make git; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is not installed"
done
cxx=${CXX:-c++}
command -v "$cxx" >/dev/null 2>&1 || fail "no C++ compiler: $cxx"

# The cache key: the pin (tarball and patches, by digest), the recipe's settings and this
# script (both by digest, so a change of either builds again), the toolchain that compiles it,
# and the flags the environment adds to it.
cxx_version=$("$cxx" --version 2>/dev/null | head -n 1)
cmake_version=$(cmake --version | head -n 1)
machine="$(uname -s) $(uname -m)"
options_sha256=$(digest "$recipe/cmake-options.txt")
script_sha256=$(digest "$root/scripts/build-z3.sh")
key=$(printf '%s\n%s\n%s\n%s\n%s\n%s\n%s\n%s\n%s\n' "$source_sha256" "$patches" "$options_sha256" "$script_sha256" "$cxx_version" "$cmake_version" "$machine" "${CXXFLAGS:-}" "${MACOSX_DEPLOYMENT_TARGET:-}" | digest_stdin)
dest="$out/$key"
if [ "$key_only" = 1 ]; then
    echo "$key"
    exit 0
fi

work=$(mktemp -d)
trap 'rm -rf "${work:?}"' EXIT

# 1. The tarball, checked by digest before anything else happens.
tarball="$work/$source"
if [ -n "$source_file" ]; then
    cp "$source_file" "$tarball"
else
    echo "downloading $mirror"
    curl --fail --location --silent --show-error --retry 3 --output "$tarball" "$mirror" \
        || fail "cannot download $mirror; pass a local copy with --source FILE"
fi
actual=$(digest "$tarball")
[ "$actual" = "$source_sha256" ] \
    || fail "SHA-256 mismatch for $source: the lock pins $source_sha256, the file is $actual"
actual=$(gzip -dc "$tarball" | digest_stdin)
[ "$actual" = "$source_tar_sha256" ] \
    || fail "SHA-256 mismatch for the uncompressed $source: the lock pins $source_tar_sha256, found $actual"
echo "ok: $source matches supply-chain/z3.lock ($source_sha256)"

# One cold build into $1 (a fresh directory); prints nothing but progress. The source is
# extracted anew for each build, so neither build sees the other's files.
build() {
    dir=$1
    src="$dir/src"
    bld="$dir/build"
    mkdir -p "$src" "$bld"
    tar -xzf "$tarball" -C "$src"
    src="$src/z3-$version"
    [ -d "$src" ] || fail "$source does not unpack to z3-$version/"
    # Our patches, in the lock's order, on the pristine tree: each checked, then applied.
    printf '%s\n' "$patches" | while read -r name sha; do
        git -C "$src" apply --check "$recipe/$name" \
            || fail "supply-chain/z3/$name does not apply to z3 $version"
        git -C "$src" apply "$recipe/$name"
    done || exit 1
    # The CMake settings, one -D per non-comment line of the recipe.
    set --
    while IFS= read -r line; do
        case $line in ''|'#'*) continue ;; esac
        set -- "$@" "-D$line"
    done < "$recipe/cmake-options.txt"
    # The source and build directories are mapped to fixed names, so the bytes do not depend on
    # where the build ran. No other flag is added here: the rest come from Z3's own CMake files
    # under the Release configuration, and the configure log records them.
    flags="-ffile-prefix-map=$src=/z3-src -ffile-prefix-map=$bld=/z3-build"
    case "$src$bld" in *" "*) fail "a build path has a space, which CMAKE_CXX_FLAGS cannot hold: $dir" ;; esac
    # The archive's member headers carry each object's modification time, owner and group, so
    # two builds of the same objects gave two libraries (#1277: the objects were equal byte for
    # byte, the headers' times were not). ZERO_AR_DATE=1 makes Apple's ar, ranlib and libtool
    # write zero for all three (`ar tv` shows 0/0 and the epoch). GNU ar's deterministic mode does
    # the same, and it is the default of Debian's and Ubuntu's binutils but not of every build of
    # binutils: gcc-toolset-13's, in the AlmaLinux 8 image of scripts/local-ci/ (#2105), writes
    # the times and owners unless asked. So with GNU ar the archive rules ask for it, with `D`,
    # and ranlib with `-D`, whatever the default; Apple's ar is left as it was.
    ZERO_AR_DATE=1
    export ZERO_AR_DATE
    if ar --version 2>/dev/null | head -n 1 | grep -q '^GNU ar'; then
        set -- "$@" \
            "-DCMAKE_CXX_ARCHIVE_CREATE=<CMAKE_AR> qcD <TARGET> <LINK_FLAGS> <OBJECTS>" \
            "-DCMAKE_CXX_ARCHIVE_APPEND=<CMAKE_AR> qD <TARGET> <LINK_FLAGS> <OBJECTS>" \
            "-DCMAKE_CXX_ARCHIVE_FINISH=<CMAKE_RANLIB> -D <TARGET>"
    fi
    start=$(date +%s)
    cmake -S "$src" -B "$bld" -G "Unix Makefiles" -DCMAKE_CXX_COMPILER="$cxx" \
        -DCMAKE_CXX_FLAGS="$flags" "$@" > "$dir/configure.log" 2>&1 \
        || { cat "$dir/configure.log" >&2; fail "cmake configure failed"; }
    cmake --build "$bld" --target shell shell_lib -- -j "$jobs" > "$dir/build.log" 2>&1 \
        || { tail -n 50 "$dir/build.log" >&2; fail "the build failed; the log is $dir/build.log"; }
    end=$(date +%s)
    [ -f "$bld/libz3shell.a" ] || fail "the build wrote no libz3shell.a"
    [ -f "$bld/z3" ] || fail "the build wrote no z3"
    echo $((end - start)) > "$dir/seconds"
}

mkdir -p "$out"
build "$work/one"
lib_sha256=$(digest "$work/one/build/libz3shell.a")
z3_sha256=$(digest "$work/one/build/z3")
if [ "$reproducible" = 1 ]; then
    build "$work/two"
    second=$(digest "$work/two/build/libz3shell.a")
    [ "$lib_sha256" = "$second" ] \
        || fail "not reproducible: the two cold builds gave libz3shell.a $lib_sha256 and $second"
    echo "ok: two cold builds of libz3shell.a gave the same digest"
fi

# 2. The output, under the cache key, with its record and the library's digest: staged
# beside the entry and moved into place in one step, so that a concurrent build or reader
# never sees a half-written entry, and an entry already there with the same library (another
# build that finished first) is kept, while one with another digest (refused by the reader) is
# set aside and replaced.
staged="$out/.staging.$$.$key"
rm -rf "${staged:?}"
mkdir -p "$staged/lib" "$staged/bin"
cp "$work/one/build/libz3shell.a" "$staged/lib/libz3shell.a"
cp "$work/one/build/z3" "$staged/bin/z3"
chmod 755 "$staged/bin/z3"
{
    echo "z3_version $version"
    echo "z3_tag z3-$version"
    echo "z3_commit $commit"
    echo "z3_tree $tree"
    echo "z3_source_sha256 $source_sha256"
    echo "z3_library_sha256 $lib_sha256"
    echo "z3_standalone_sha256 $z3_sha256"
    echo "target $(uname -s) $(uname -m)"
    echo "cxx $("$cxx" --version 2>/dev/null | head -n 1)"
    echo "cmake $(cmake --version | head -n 1)"
    echo "jobs $jobs"
    if ar --version 2>/dev/null | head -n 1 | grep -q '^GNU ar'; then
        echo "archive ZERO_AR_DATE=1, GNU ar qcD and ranlib -D"
    else
        echo "archive ZERO_AR_DATE=1"
    fi
    echo "seconds $(cat "$work/one/seconds")"
    [ "$reproducible" = 1 ] && echo "reproducible yes, two cold builds, second build $(cat "$work/two/seconds") s"
    echo "built $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    sed -n 's/^-- \(Z3_COMPONENT_CXX_FLAGS: .*\)$/\1/p; s/^-- \(Z3_COMPONENT_CXX_DEFINES: .*\)$/\1/p' "$work/one/configure.log"
} > "$staged/build-record.txt"
cp "$work/one/configure.log" "$staged/configure.log"
echo "$lib_sha256" > "$staged/library.sha256"
if [ -f "$dest/library.sha256" ] && [ "$(cat "$dest/library.sha256")" = "$lib_sha256" ]; then
    rm -rf "${staged:?}"
    echo "an entry with the same library is already in place; kept"
else
    if [ -e "$dest" ]; then
        mv "$dest" "$out/.refused.$$.$key"
        rm -rf "$out/.refused.$$.$key"
    fi
    mv "$staged" "$dest"
fi
echo "built z3 $version in $(cat "$work/one/seconds") s"
echo "key $key"
echo "libz3shell.a sha256 $lib_sha256"
echo "z3 sha256 $z3_sha256"
echo "output $dest"
