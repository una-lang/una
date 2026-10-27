#!/bin/sh
# Installs Una (#240): the `una` binary, the native runtime `lib/libuna_rt.a` and Z3 at the
# version the release pins, into UNA_HOME (default ~/.una), as
#
#   UNA_HOME/bin/una
#   UNA_HOME/lib/libuna_rt.a        (where `una build` looks, beside `bin/`)
#   UNA_HOME/z3/bin/z3, and UNA_HOME/bin/z3 linking to it
#   UNA_HOME/docs/guide/*.md        (the guides, from releases that ship them, #410)
#
# Every download is checked before anything is installed:
# - the release archive, against the SHA-256 that the release's validation summary lists
#   (docs/validation/<version>.md, docs/release-gates.md), whose verdict must be PASS. The
#   summary is read from the release's tag and, for a release tagged before summaries were
#   committed, from the main branch. A macOS archive that was packed again after its release is
#   accepted only as docs/validation/repacked.md pairs it with the summary's digest;
# - Z3, against the SHA-256 that the Z3 project publishes for its release, pinned below as in
#   scripts/install-z3.sh.
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
#   UNA_VERSION       the version to install (default: the one below)
#   UNA_HOME          where to install (default: ~/.una)
#   UNA_RELEASES      where release archives are, as <base>/v<version>/<archive>
#                     (default: https://github.com/una-lang/una/releases/download)
#   UNA_RECORDS       where validation summaries are, as <base>/<ref>/docs/validation/<version>.md
#                     with <ref> the tag v<version>, then main
#                     (default: https://raw.githubusercontent.com/una-lang/una)
#   UNA_Z3_RELEASES   where Z3 archives are, as <base>/z3-<version>/<asset>.zip
#                     (default: https://github.com/Z3Prover/z3/releases/download)
#
# Needs: curl or wget, tar, unzip, and shasum or sha256sum.
set -eu

# The release this script installs by default, and the Z3 it pins; a test keeps them equal to
# the workspace version and to verification.lock and scripts/install-z3.sh.
DEFAULT_VERSION=0.3.16
Z3_VERSION=5.1.0

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
z3_releases=${UNA_Z3_RELEASES:-https://github.com/Z3Prover/z3/releases/download}

# The platform: the target of the archive and the Z3 asset with its published digest.
platform="$(uname -s) $(uname -m)"
case $platform in
    "Darwin arm64")
        target=aarch64-apple-darwin
        z3_asset=z3-5.1.0-arm64-osx-13.3
        z3_sha256=81d29e934fd863079a74af35eecaeaef8047e0e12414d33ca322b358d68383db ;;
    "Linux x86_64")
        target=x86_64-unknown-linux-gnu
        z3_asset=z3-5.1.0-x64-glibc-2.39
        z3_sha256=f47be8d27d3230e823bf1eeede2fe0abaca55bb78d0b59974370e6689a92284a ;;
    "Linux aarch64" | "Linux arm64")
        target=aarch64-unknown-linux-gnu
        z3_asset=z3-5.1.0-arm64-glibc-2.38
        z3_sha256=2832cfd43c6862fbaf78834b5f1e35d031b4a87e85456b696c1c6f609cd5609e ;;
    *) fail "no Una release for $platform; the releases are for macOS on arm64 and Linux on x86_64 and arm64" ;;
esac

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
for tool in tar unzip; do
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

say "downloading Z3 $Z3_VERSION"
fetch "$z3_releases/z3-$Z3_VERSION/$z3_asset.zip" "$work/$z3_asset.zip" \
    || fail "cannot download $z3_asset.zip"
actual=$(digest "$work/$z3_asset.zip")
[ "$actual" = "$z3_sha256" ] \
    || fail "SHA-256 mismatch for $z3_asset.zip: expected $z3_sha256, the download is $actual"

# Unpacked and checked in the work directory, then moved into place.
tar -xzf "$work/$archive" -C "$work"
unpacked=$work/una-$version-$target
[ -x "$unpacked/una" ] && [ -f "$unpacked/lib/libuna_rt.a" ] \
    || fail "$archive does not hold una and lib/libuna_rt.a"
unzip -q "$work/$z3_asset.zip" "$z3_asset/bin/z3" -d "$work"

# The directories this installation creates, kept from an earlier record when there is one, so
# that uninstall removes them and no other.
created=$work/created
: > "$created"
[ -f "$marker" ] && sed -n 's/^dir //p' "$marker" > "$created"
for dir in . bin lib z3 z3/bin; do
    [ -d "$home/$dir" ] || echo "$dir" >> "$created"
done
mkdir -p "$home/bin" "$home/lib" "$home/z3/bin"
cp "$unpacked/una" "$home/bin/una.new" && mv "$home/bin/una.new" "$home/bin/una"
cp "$unpacked/lib/libuna_rt.a" "$home/lib/libuna_rt.a"
cp "$work/$z3_asset/bin/z3" "$home/z3/bin/z3"
chmod +x "$home/bin/una" "$home/z3/bin/z3"
ln -sf ../z3/bin/z3 "$home/bin/z3"
files="bin/una bin/z3 lib/libuna_rt.a z3/bin/z3"
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
say "installed una $version and Z3 $Z3_VERSION in $home"
"$home/bin/una" --version 2>/dev/null | sed 's/^/    /' || true
case ":$PATH:" in
    *":$home/bin:"*) ;;
    *) say "add $home/bin to PATH, for example with this line in your shell's profile:"
       printf '\n    export PATH="%s/bin:$PATH"\n\n' "$home" ;;
esac
say "uninstall with: sh install.sh --uninstall"
