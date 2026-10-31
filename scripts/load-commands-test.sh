#!/bin/sh
# Canaries of scripts/load-commands.sh (#1277): small executables, linked here with the pinned
# rustc, each of which asks the dynamic loader for one library that it must not, must make the
# check fail with that library, and the clean ones must pass. They cover the scopes: an entry of
# scripts/load-commands-una.txt (on macOS the C++ runtime, `/usr/lib/libc++.1.dylib`) is allowed
# in `una` alone, never in an executable that `una build` links, and a library on neither list
# fails everywhere. The library on neither list is a dynamic library built here (`cdylib`), so
# the canary needs nothing of the system. The real artifacts are never changed.
#
# Usage: scripts/load-commands-test.sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
check="$root/scripts/load-commands.sh"
work=$(mktemp -d "${TMPDIR:-/tmp}/load-commands-test.XXXXXX")
trap 'rm -rf "${work:?}"' EXIT
status=0

case $(uname -s) in
Darwin) os=macos ;;
Linux) os=linux ;;
*) echo "error: no rule for the platform $(uname -s)" >&2; exit 1 ;;
esac

# rustc of the pinned toolchain, run from the repository so that rust-toolchain.toml applies.
compile() {
    (cd "$root" && rustc --edition 2021 -O "$@") > "$work/rustc.log" 2>&1 || {
        cat "$work/rustc.log" >&2
        echo "error: rustc $* failed" >&2
        exit 1
    }
}

# A library on neither list, and executables: clean, linked with it, and (macOS) linked with
# the C++ runtime that scripts/load-commands-una.txt allows `una` alone.
mkdir "$work/lib"
printf '#[no_mangle]\npub extern "C" fn una_canary(x: u32) -> u32 {\n    x + 1\n}\n' > "$work/unacanary.rs"
compile --crate-type cdylib -o "$work/lib/$( [ "$os" = macos ] && echo libunacanary.dylib || echo libunacanary.so)" \
    "$work/unacanary.rs"
printf 'fn main() {\n    println!("{}", std::hint::black_box(1u32));\n}\n' > "$work/clean.rs"
printf 'extern "C" {\n    fn una_canary(x: u32) -> u32;\n}\nfn main() {\n    println!("{}", unsafe { una_canary(std::hint::black_box(1)) });\n}\n' > "$work/foreign.rs"
compile -o "$work/clean" "$work/clean.rs"
compile -o "$work/foreign" -L "$work/lib" -l dylib=unacanary "$work/foreign.rs"
if [ "$os" = macos ]; then
    # Without -dead_strip_dylibs the linker keeps the load command although nothing is called.
    compile -o "$work/cxx" -l dylib=c++ "$work/clean.rs"
    otool -L "$work/cxx" | grep -q '/usr/lib/libc++.1.dylib' || {
        echo "error: the canary linked with -lc++ has no load command for /usr/lib/libc++.1.dylib" >&2
        exit 1
    }
fi

# A directory holding the executable $1 alone.
dir_of() {
    rm -rf "${work:?}/exes-$1"
    mkdir "$work/exes-$1"
    cp "$work/$1" "$work/exes-$1/"
    echo "$work/exes-$1"
}

# pass NAME UNA EXECUTABLES: the check must pass.
pass() {
    name=$1
    shift
    if ! "$check" "$@" > "$work/$name.log" 2>&1; then
        echo "error: canary $name fails the check:" >&2
        cat "$work/$name.log" >&2
        status=1
    fi
}

# reject NAME MESSAGE UNA EXECUTABLES: the check must fail, and say MESSAGE on a line that also
# names the library LIBRARY (a variable, the line itself when empty).
reject() {
    name=$1 message=$2
    shift 2
    if "$check" "$@" > "$work/$name.log" 2>&1; then
        echo "error: canary $name passes the check" >&2
        status=1
    elif ! grep -F -- "$message" "$work/$name.log" | grep -qF -- "${library:-$message}"; then
        echo "error: canary $name fails for another reason than \"$message\":" >&2
        cat "$work/$name.log" >&2
        status=1
    fi
}

# The install name of the canary library is the linker's choice (its path, or @rpath/...), so
# the line is matched by its kind and the library's name.
if [ "$os" = macos ]; then kind=dylib; else kind=needed; fi
pass clean "$work/clean" "$(dir_of clean)"
library=libunacanary
reject una-foreign "$work/foreign: not allowed: $kind " "$work/foreign" "$(dir_of clean)"
reject executable-foreign "exes-foreign/foreign: not allowed: $kind " "$work/clean" "$(dir_of foreign)"
library=
count=2
if [ "$os" = macos ]; then
    pass una-cxx "$work/cxx" "$(dir_of clean)"
    reject executable-cxx "cxx: not allowed: dylib /usr/lib/libc++.1.dylib" "$work/clean" "$(dir_of cxx)"
    count=3
fi

if [ "$status" -ne 0 ]; then
    echo "load-commands-test: FAIL" >&2
    exit 1
fi
echo "load-commands-test: PASS ($os: the clean runs pass, $count canaries fail as they must)"
