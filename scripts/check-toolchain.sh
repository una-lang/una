#!/bin/sh
# Checks the Linux toolchain that links the release `una` against supply-chain/z3.lock (#1926,
# #1958): every package of the lock's [toolchain.TARGET] table must be installed with the
# version it records (`rpm -q`, as VERSION-RELEASE; the image is AlmaLinux 8's, #2105), and every file of its [toolchain-files.TARGET] table,
# found where `cc` takes it (`cc -print-file-name=<file>`; `cc` is the driver that links `una`),
# must have the SHA-256 it records. scripts/local-ci/Dockerfile.release installs the packages with no
# version, so a rebuilt image is refused here until the lock is updated deliberately. A missing
# table, package or file, or another version or digest, exits 2 with the reason on stderr; on
# success each package and file is printed on a line `toolchain: TARGET: <package> <version>`
# or `toolchain: TARGET: <file> sha256 <digest>`. scripts/release.sh runs it for a Linux target
# before `una` is built.
#
# What it checks is the image's files at the paths `cc` names. It does not trace the link:
# a search path added to the link (RUSTFLAGS `-L`, a `-C linker` or a planted file in a
# directory searched first) is outside it.
#
# Usage: scripts/check-toolchain.sh TARGET
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: scripts/check-toolchain.sh TARGET" >&2
    exit 2
fi
target=$1
case $target in
    *[!a-z0-9_-]* | '')
        echo "error: $target is not a target triple" >&2
        exit 2
        ;;
esac
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
lock="$root/supply-chain/z3.lock"
[ -f "$lock" ] || { echo "error: $lock is missing" >&2; exit 2; }
sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    else
        shasum -a 256 "$1" | cut -d ' ' -f 1
    fi
}

command -v rpm >/dev/null 2>&1 \
    || { echo "error: rpm is missing; a Linux release builds in the image of scripts/local-ci/Dockerfile.release, whose packages supply-chain/z3.lock records" >&2; exit 2; }
packages=$(sed -n '/^\[toolchain\.'"$target"'\]$/,/^\[/s/^"\([^"]*\)" *= *"\([^"]*\)"$/\1 \2/p' "$lock")
[ -n "$packages" ] \
    || { echo "error: supply-chain/z3.lock records no [toolchain.$target] package versions" >&2; exit 2; }
printf '%s\n' "$packages" | while read -r package pinned; do
    installed=$(rpm -q --queryformat '%{VERSION}-%{RELEASE}' "$package" 2>/dev/null) || installed=
    [ "$installed" = "$pinned" ] \
        || { echo "error: the package $package is ${installed:-not installed}${installed:+ installed}, but supply-chain/z3.lock records $pinned for $target; a new image updates the [toolchain.$target] table deliberately" >&2; exit 2; }
    echo "toolchain: $target: $package $installed"
done || exit 2

files=$(sed -n '/^\[toolchain-files\.'"$target"'\]$/,/^\[/s/^"\([^"]*\)" *= *"\([0-9a-f]\{64\}\)"$/\1 \2/p' "$lock")
[ -n "$files" ] \
    || { echo "error: supply-chain/z3.lock records no [toolchain-files.$target] digests" >&2; exit 2; }
printf '%s\n' "$files" | while read -r file pinned; do
    path=$(cc -print-file-name="$file" 2>/dev/null) || path=
    case $path in
        /*) ;;
        *) echo "error: cc finds no $file, which supply-chain/z3.lock pins for $target" >&2; exit 2 ;;
    esac
    [ -f "$path" ] \
        || { echo "error: cc names $path for $file, which is not a file" >&2; exit 2; }
    digest=$(sha256_of "$path")
    [ "$digest" = "$pinned" ] \
        || { echo "error: $path has SHA-256 $digest, but supply-chain/z3.lock pins $pinned for $file on $target; a new image updates the [toolchain-files.$target] table deliberately" >&2; exit 2; }
    echo "toolchain: $target: $file sha256 $digest"
done || exit 2
