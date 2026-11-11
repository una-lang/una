#!/bin/sh
# Installs Una (#240): the `una` binary, with the solver and the native runtime built into it
# (ADR 0124), into UNA_HOME (default ~/.una), as
#
#   UNA_HOME/bin/una
#   UNA_HOME/README.md, LICENSE
#   UNA_HOME/docs/guide/*.md        (the guides, from releases that ship them, #410)
#
# A release before the runtime was built into `una` (ADR 0124 step 2) ships it as
# `lib/libuna_rt.a`, where its `una build` looks; for such a release the installer installs
# UNA_HOME/lib/libuna_rt.a as well. A later archive holds no `lib/`, and the installation has
# none. An archive whose top level holds anything but `una`, `LICENSE`, `README.md`, `docs/` and
# that `lib/libuna_rt.a` is refused, and so is one with an entry outside its own directory or an
# entry that is a link, symbolic or hard, or anything else that is neither a file nor a directory.
#
# Every download is checked before anything is installed:
# - the release archive, against the SHA-256 that the release's validation summary lists
#   (docs/validation/<version>.md, docs/release-gates.md), whose verdict must be PASS. The
#   summary is read from the release's tag and, for a release tagged before summaries were
#   committed, from the main branch. A macOS archive that was packed again after its release is
#   accepted only as docs/validation/repacked.md pairs it with the summary's digest.
# Nothing else is downloaded: the Z3 solver is built into `una` (ADR 0124), so no separate Z3
# is installed and none is looked for on PATH.
# The platform is checked before anything is downloaded. The releases are for macOS on arm64 and
# Linux on arm64 (aarch64). The Linux `una` is a dynamic glibc executable that needs glibc 2.39 or
# later, so a musl system (Alpine and others), an older glibc, or a system whose glibc version
# cannot be read is refused; so are Linux on x86_64, which this release does not ship, and every
# other system.
# A mismatch stops the installation with exit code 1, and nothing is installed. Nothing outside
# UNA_HOME is changed: the script prints the line to add to your shell's profile.
#
# The installation is recorded in UNA_HOME/.una-install: the version, each file it installed and
# each directory it created. `--uninstall` removes exactly those files, then those directories
# when they are empty, and refuses a UNA_HOME without that record, so it never removes what
# Una did not install.
#
# Usage:
#   curl -fsSL <site>/install.sh | sh
#   sh install.sh [--uninstall]
#
# Settings, all optional:
#   UNA_VERSION       the version to install, 0.4.0 or later (default: the one below)
#   UNA_HOME          where to install (default: ~/.una)
#   UNA_RELEASES      where release archives are, as <base>/v<version>/<archive>
#                     (default: https://github.com/una-lang/una/releases/download)
#   UNA_RECORDS       where validation summaries are, as <base>/<ref>/docs/validation/<version>.md
#                     with <ref> the tag v<version>, then main
#                     (default: https://raw.githubusercontent.com/una-lang/una)
#
# Needs: curl or wget, tar, and shasum or sha256sum; on Linux, getconf.
set -eu

# The release this script installs by default; a test keeps it equal to the workspace version.
# The release commit bumps it together with the README's pins.
DEFAULT_VERSION=0.3.16

say() { printf '%s\n' "una install: $*"; }
fail() {
    printf '%s\n' "una install: error: $*" >&2
    exit 1
}

home=${UNA_HOME:-$HOME/.una}

marker=$home/.una-install

if [ "${1:-}" = --uninstall ]; then
    [ -f "$marker" ] \
        || fail "$home has no record of an installation by this script ($marker), so nothing is removed"
    # The files first, then the directories that the installation created, deepest first, when
    # they are empty. A path that leaves UNA_HOME is never followed.
    # Every path is checked before anything is removed.
    while read -r kind path; do
        case $kind in
            file | dir)
                case $path in /* | *..* | "") fail "the record $marker names '$path', which is not inside $home" ;; esac ;;
        esac
    done < "$marker"
    while read -r kind path; do
        [ "$kind" = file ] && rm -f "${home:?}/$path"
    done < "$marker"
    dirs=$(sed -n 's/^dir //p' "$marker" | awk '{ print length($0) "\t" $0 }' | sort -rn | cut -f2-)
    rm -f "$marker"
    printf '%s\n' "$dirs" | while read -r path; do
        case $path in
            "") ;;
            .) rmdir "$home" 2> /dev/null || true ;;
            *) rmdir "$home/$path" 2> /dev/null || true ;;
        esac
    done
    if [ -d "$home" ]; then
        say "removed Una from $home; the other files there are kept"
    else
        say "removed $home"
    fi
    say "remove the PATH line for $home/bin from your shell's profile too"
    exit 0
fi
[ "$#" -eq 0 ] || {
    printf '%s\n' "usage: sh install.sh [--uninstall]" >&2
    exit 2
}

version=${UNA_VERSION:-$DEFAULT_VERSION}
releases=${UNA_RELEASES:-https://github.com/una-lang/una/releases/download}
records=${UNA_RECORDS:-https://raw.githubusercontent.com/una-lang/una}

# The version: MAJOR.MINOR.PATCH, compared as numbers, never as text (0.10.0 is after 0.4.0).
# A release before 0.4.0 needs a Z3 on PATH, which this installer no longer installs, so its
# `una` could prove nothing: it is refused.
case $version in
    "" | *[!0-9.]* | .* | *. | *..*) fail "'$version' is not a version: UNA_VERSION is MAJOR.MINOR.PATCH, such as 0.4.0" ;;
esac
[ "$(printf '%s\n' "$version" | awk -F. '{ print NF }')" -eq 3 ] \
    || fail "'$version' is not a version: UNA_VERSION is MAJOR.MINOR.PATCH, such as 0.4.0"
major=${version%%.*}
minor=${version#*.}
minor=${minor%%.*}
if [ "$major" -eq 0 ] && [ "$minor" -lt 4 ]; then
    fail "this installer installs 0.4.0 and later; $version needs a separate Z3 that it no longer installs"
fi

# The platform: the target of the archive, one of scripts/release-targets.txt (a test keeps the
# two equal). Linux on x86_64 is not shipped: its confinement has no native test run yet.
platform="$(uname -s) $(uname -m)"
shipped="the releases are for macOS on arm64 and Linux on arm64 (aarch64)"
case $platform in
    "Darwin arm64")
        target=aarch64-apple-darwin ;;
    "Linux aarch64" | "Linux arm64")
        target=aarch64-unknown-linux-gnu ;;
    "Linux x86_64" | "Linux amd64")
        fail "Linux on x86_64 is not shipped in this release; $shipped" ;;
    *) fail "no Una release for $platform; $shipped" ;;
esac

# On Linux, the C library, before anything is downloaded. `una` is a dynamic glibc executable
# built on glibc 2.39 (the build image's, ADR 0124 rule 1), so it does not start on musl or on an
# older glibc. glibc is shown, never assumed: `getconf GNU_LIBC_VERSION` must print
# `glibc MAJOR.MINOR` at or above the floor, compared as numbers (2.4 is before 2.39). A musl
# system is refused even when that answer looks like glibc's: its `ldd --version` names musl, or
# its loader is /lib/ld-musl-*.
glibc_floor=2.39
if [ "${platform%% *}" = Linux ]; then
    needs="this release's una needs glibc $glibc_floor or later; Alpine and other musl systems are not supported"
    musl=
    if ldd --version 2>&1 | grep -qi musl; then musl=1; fi
    for loader in /lib/ld-musl-*; do
        [ -e "$loader" ] && musl=1
    done
    [ -z "$musl" ] || fail "this system's C library is musl; $needs"
    libc=$(getconf GNU_LIBC_VERSION 2> /dev/null) || libc=
    glibc=${libc#glibc }
    case $libc in
        "glibc "[0-9]*.[0-9]*) ;;
        *) glibc= ;;
    esac
    case $glibc in
        "" | *[!0-9.]* | *..* | *.) fail "cannot read this system's glibc version (getconf GNU_LIBC_VERSION); $needs" ;;
    esac
    awk -v have="$glibc" -v floor="$glibc_floor" 'BEGIN {
        split(have, h, "."); split(floor, f, ".")
        exit !(h[1] + 0 > f[1] + 0 || (h[1] + 0 == f[1] + 0 && h[2] + 0 >= f[2] + 0))
    }' || fail "this system has glibc $glibc; $needs"
fi

if command -v curl > /dev/null 2>&1; then
    fetch() { curl --fail --location --silent --show-error --retry 3 --output "$2" "$1"; }
elif command -v wget > /dev/null 2>&1; then
    fetch() { wget --quiet --output-document="$2" "$1"; }
else
    fail "neither curl nor wget is installed"
fi
if command -v sha256sum > /dev/null 2>&1; then
    digest() { sha256sum "$1" | cut -d ' ' -f 1; }
elif command -v shasum > /dev/null 2>&1; then
    digest() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
else
    fail "neither sha256sum nor shasum is installed"
fi
for tool in tar; do
    command -v "$tool" > /dev/null 2>&1 || fail "$tool is not installed"
done

work=$(mktemp -d "${TMPDIR:-/tmp}/una-install.XXXXXX")
trap 'rm -rf "${work:?}"' EXIT

# The validation summary of the release: its verdict, and the archive's digest. It comes from
# the repository's tree, never from the release's own assets, so that the archive is checked
# against a digest the release cannot replace (#717).
record=$version.md
say "reading the validation summary of $version"
fetch "$records/v$version/docs/validation/$record" "$work/$record" \
    || fetch "$records/main/docs/validation/$record" "$work/$record" \
    || fail "no validation summary for $version (docs/validation/$record), so this script does not install it; a release without one, such as 0.1.x, can be installed by hand from its release assets"
grep -qx 'Verdict: PASS (every gate)' "$work/$record" \
    || fail "the validation summary of $version is not a PASS of every gate"
archive=una-$version-$target.tar.gz
expected=$(sed -n "s/^| \`$archive\` | \`\([0-9a-f]\{64\}\)\` |\$/\1/p" "$work/$record")
[ -n "$expected" ] || fail "the validation summary of $version lists no $archive"

# docs/validation/repacked.md, read like the summary (the tag, then main), only when it is needed:
# the macOS archives that were packed again after their release, each paired with its gated
# digest, and the ones that were withdrawn.
repacked() {
    [ -f "$work/repacked.md" ] \
        || fetch "$records/v$version/docs/validation/repacked.md" "$work/repacked.md" \
        || fetch "$records/main/docs/validation/repacked.md" "$work/repacked.md" \
        || : > "$work/repacked.md"
}

say "downloading $archive"
if ! fetch "$releases/v$version/$archive" "$work/$archive"; then
    repacked
    awk -F'`' -v a="$archive" '$1 == "| " && $2 == a && NF == 3 { found = 1 } END { exit !found }' "$work/repacked.md" \
        && fail "$archive was withdrawn (docs/validation/repacked.md); on this platform, install another version"
    fail "cannot download $archive"
fi
actual=$(digest "$work/$archive")
if [ "$actual" != "$expected" ]; then
    # A repacked archive: accepted only when repacked.md pairs exactly this archive's name and
    # exactly the summary's digest with the digest of the download.
    repacked
    published=$(awk -F'`' -v a="$archive" -v g="$expected" '$1 == "| " && $2 == a && $4 == g { print $6 }' "$work/repacked.md")
    [ -n "$published" ] && [ "$actual" = "$published" ] \
        || fail "SHA-256 mismatch for $archive: the validation summary says $expected, the download is $actual"
    say "$archive is the repacked archive of the gated $expected (docs/validation/repacked.md)"
fi

# Unpacked and checked in the work directory, then moved into place. `una` is the one program
# of the archive; `lib/libuna_rt.a`, the runtime of a release before ADR 0124 step 2, is the one
# other file that may be installed outside the documents.
# Before anything is unpacked: every entry lies under the archive's own directory, with no `..`
# component, and is a file or a directory, never a link (the first character of the listing's
# mode column, in both tars), so that nothing reaches outside the work directory or installs
# what the archive does not hold.
top=una-$version-$target
tar -tzf "$work/$archive" > "$work/entries" || fail "cannot list $archive"
awk -v top="$top" '
    $0 != top && $0 != top "/" && index($0, top "/") != 1 { bad = 1 }
    $0 ~ /(^|\/)\.\.(\/|$)/ { bad = 1 }
    END { exit bad }' "$work/entries" \
    || fail "$archive holds an entry outside $top/, which this installer refuses"
tar -tvzf "$work/$archive" > "$work/listing" || fail "cannot list $archive"
awk 'substr($1, 1, 1) != "-" && substr($1, 1, 1) != "d" { bad = 1 } END { exit bad }' "$work/listing" \
    || fail "$archive holds a link or another entry that is neither a file nor a directory, which this installer refuses"
tar -xzf "$work/$archive" -C "$work"
unpacked=$work/$top
# And after: what was unpacked holds no link and nothing but files and directories.
[ -z "$(find "$unpacked" ! -type f ! -type d | head -n 1)" ] \
    || fail "$archive holds a link or another entry that is neither a file nor a directory, which this installer refuses"
[ -f "$unpacked/una" ] && [ -x "$unpacked/una" ] || fail "$archive does not hold una"
for entry in "$unpacked"/* "$unpacked"/.[!.]* "$unpacked"/..?*; do
    [ -e "$entry" ] || [ -L "$entry" ] || continue
    case ${entry##*/} in
        una | LICENSE | README.md | docs) ;;
        lib)
            [ "$(ls -A "$entry")" = libuna_rt.a ] && [ -f "$entry/libuna_rt.a" ] && [ ! -L "$entry/libuna_rt.a" ] \
                || fail "$archive holds a lib/ with more than lib/libuna_rt.a, which this installer refuses"
            ;;
        *) fail "$archive holds '${entry##*/}', which this installer refuses: a release holds una, LICENSE, README.md and docs/" ;;
    esac
done
runtime=
[ -f "$unpacked/lib/libuna_rt.a" ] && runtime=lib/libuna_rt.a

# The directories this installation creates, kept from an earlier record when there is one, so
# that uninstall removes them and no other.
created=$work/created
: > "$created"
[ -f "$marker" ] && sed -n 's/^dir //p' "$marker" > "$created"
for dir in . bin ${runtime:+lib}; do
    [ -d "$home/$dir" ] || echo "$dir" >> "$created"
done
mkdir -p "$home/bin"
cp "$unpacked/una" "$home/bin/una.new" && mv "$home/bin/una.new" "$home/bin/una"
chmod +x "$home/bin/una"
files="bin/una"
if [ -n "$runtime" ]; then
    mkdir -p "$home/lib"
    cp "$unpacked/$runtime" "$home/$runtime"
    files="$files $runtime"
elif [ -f "$marker" ] && grep -qx 'file lib/libuna_rt.a' "$marker" && [ -f "$home/lib/libuna_rt.a" ]; then
    # The runtime an earlier installation put there, which this `una` never reads: it stays in
    # the record, so that uninstall still removes it. Removing it now is the migration of ADR
    # 0124 step 5.
    files="$files lib/libuna_rt.a"
fi
for doc in README.md LICENSE; do
    if [ -f "$unpacked/$doc" ]; then
        cp "$unpacked/$doc" "$home/$doc"
        files="$files $doc"
    fi
done
# The guides, from releases that ship them (#410), at the paths the README links.
if [ -d "$unpacked/docs/guide" ]; then
    for dir in docs docs/guide; do
        [ -d "$home/$dir" ] || echo "$dir" >> "$created"
    done
    mkdir -p "$home/docs/guide"
    for guide in "$unpacked"/docs/guide/*.md; do
        [ -f "$guide" ] || continue
        base=$(basename "$guide")
        # The record is one path per word, and `cp` takes options: refuse a name that would break
        # either, though no released archive has one.
        case $base in
            -* | *[[:space:]]*) fail "$archive holds a guide named '$base', which this installer refuses" ;;
        esac
        cp "$guide" "$home/docs/guide/$base"
        files="$files docs/guide/$base"
    done
fi
{
    echo "# Una $version, installed by install.sh; \`sh install.sh --uninstall\` removes what follows."
    echo "version $version"
    sort -u "$created" | sed 's/^/dir /'
    for file in $files; do echo "file $file"; done
} > "$marker"

# The version is the one the validation summary vouched for. From 0.3.0 `una --version` also
# names the editions it serves (#305); an older `una` has no such flag, and prints nothing here.
say "installed una $version in $home"
"$home/bin/una" --version 2>/dev/null | sed 's/^/    /' || true
case ":$PATH:" in
    *":$home/bin:"*) ;;
    *) say "add $home/bin to PATH, for example with this line in your shell's profile:"
       printf '\n    export PATH="%s/bin:$PATH"\n\n' "$home" ;;
esac
# On Linux, `una build` links through the system's C compiler (macOS's `una` links by itself).
if [ "${platform%% *}" = Linux ]; then
    say "una build needs a C compiler on PATH as cc (gcc or clang, with the C library's development files, such as build-essential); una check, test and run do not"
    say "the executables una build writes link the system's glibc dynamically: they run where the same or a newer glibc is installed"
fi
# The solver refuses to run as root on every platform, whatever installed `una`.
if [ "${platform%% *}" = Linux ] || [ "$(id -u 2> /dev/null)" = 0 ]; then
    say "una check and every command that proves refuse to run as root: run them as an ordinary user"
fi
say "uninstall with: sh install.sh --uninstall"
