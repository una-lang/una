#!/bin/sh
# Builds the Z3 that supply-chain/z3.lock pins, by the recipe of supply-chain/z3/ (ADR 0124
# rule 1), into target/z3/<source digest>/: lib/libz3shell.a, the SMT-LIB front end as a library
# for una-z3, and bin/z3, the standalone front end from the same objects, for the solver-identity
# gate; with build-record.txt (pin, toolchain versions, the flags CMake added, time, digests).
#
# Usage: scripts/build-z3.sh [--source FILE] [--out DIR] [--jobs N] [--reproducible]
#   --source FILE   a local copy of the pinned tarball, instead of a download from the mirror
#   --out DIR       the output root (default: target/z3 under the repository)
#   --jobs N        parallel compile jobs (default: the machine's processors)
#   --reproducible  two cold builds; fails unless both libraries have the same digest
#
# The tarball's digest is checked against the lock before anything is extracted or compiled,
# and a mismatch stops the build. Nothing is written outside the output root and a temporary
# directory. It needs cmake, make, a C++20 compiler and git (which applies the recipe's patch).
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
lock="$root/supply-chain/z3.lock"
recipe="$root/supply-chain/z3"
source_file=
out="$root/target/z3"
jobs=
reproducible=0
while [ "$#" -gt 0 ]; do
    case $1 in
        --source) shift; source_file=${1:?--source needs a file} ;;
        --out) shift; out=${1:?--out needs a directory} ;;
        --jobs) shift; jobs=${1:?--jobs needs a number} ;;
        --reproducible) reproducible=1 ;;
        *) echo "usage: scripts/build-z3.sh [--source FILE] [--out DIR] [--jobs N] [--reproducible]" >&2; exit 2 ;;
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
for f in cmake-options.txt shell-library.patch; do
    [ -f "$recipe/$f" ] || fail "supply-chain/z3/$f is missing"
done
for tool in cmake make git; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is not installed"
done
cxx=${CXX:-c++}
command -v "$cxx" >/dev/null 2>&1 || fail "no C++ compiler: $cxx"

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
    # The recipe's patch: the front end as a library with its entry renamed, nothing else.
    git -C "$src" apply --check "$recipe/shell-library.patch" \
        || fail "supply-chain/z3/shell-library.patch does not apply to z3 $version"
    git -C "$src" apply "$recipe/shell-library.patch"
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

# 2. The output, under the source digest, and its record.
dest="$out/$source_sha256"
rm -rf "${dest:?}"
mkdir -p "$dest/lib" "$dest/bin"
cp "$work/one/build/libz3shell.a" "$dest/lib/libz3shell.a"
cp "$work/one/build/z3" "$dest/bin/z3"
chmod 755 "$dest/bin/z3"
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
    echo "seconds $(cat "$work/one/seconds")"
    [ "$reproducible" = 1 ] && echo "reproducible yes, two cold builds, second build $(cat "$work/two/seconds") s"
    echo "built $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    sed -n 's/^-- \(Z3_COMPONENT_CXX_FLAGS: .*\)$/\1/p; s/^-- \(Z3_COMPONENT_CXX_DEFINES: .*\)$/\1/p' "$work/one/configure.log"
} > "$dest/build-record.txt"
cp "$work/one/configure.log" "$dest/configure.log"
echo "built z3 $version in $(cat "$work/one/seconds") s"
echo "libz3shell.a sha256 $lib_sha256"
echo "z3 sha256 $z3_sha256"
echo "output $dest"
