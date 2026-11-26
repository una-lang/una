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
# Linux on arm64 (aarch64). The Linux `una` is a dynamic glibc executable that needs glibc 2.28
# or later (2.39 or later for the releases 0.4.0 to 0.5.1) and a 64-bit userland, so a musl
# system (Alpine and others), an older glibc, a system whose glibc version cannot be read, a
# 32-bit userland on a 64-bit kernel and a system whose aarch64 glibc loader does not answer are
# refused; so are Linux on x86_64, which this release does not ship, and every other system.
#
# As root, UNA_HOME must be a place that only root can change (#1938): UNA_HOME, the
# directories the installer writes in and the files there are not symbolic links, and they are
# owned by root and writable by no one else, and so is every directory above them, except that a
# directory above an existing, root-owned one may be writable by others when it is sticky, as
# /tmp is (so UNA_HOME=/tmp/una is refused unless /tmp/una already exists and is root's). The
# same holds for TMPDIR (default /tmp), where the archive is checked and unpacked, which may
# itself be sticky. Otherwise the installer refuses before anything is downloaded or removed, and
# it never runs the installed `una` as root. Only owners and mode bits are read: write access
# granted by an ACL is not seen. Each file is written as a new one, after its name is removed,
# never into an existing file, which could be a hard link to a file elsewhere (#1966). For every
# user, what is already at a name the installer writes must be something it may replace (a
# regular file or a symbolic link where a file goes, a directory where a directory goes), or the
# installer refuses before anything is written, and the fixed names before anything is downloaded
# (#2216). A regular file or a link at such a name is replaced, whoever made it; a directory or a
# special file is never removed to make room. As an ordinary user, an existing directory the
# installer writes in must also be writable by this user.
# A mismatch stops the installation with exit code 1, and nothing is installed. Nothing outside
# UNA_HOME is changed: the script prints the line to add to your shell's profile.
#
# The installation is recorded in UNA_HOME/.una-install: the version, each file it installed and
# each directory it created. Installed over an earlier installation, it removes the files of the
# earlier record that the new installation does not hold, such as the `z3` of a release before
# 0.4.0 and the `lib/libuna_rt.a` of one before ADR 0124 step 2, then the directories of that
# record left empty (ADR 0124 rule 32, #1992), so that no file stays that no record lists.
# `--uninstall` removes exactly the recorded files, then the recorded directories when they are
# empty, and refuses a UNA_HOME without that record, so it never removes what Una did not
# install. It also removes the one file `una` makes outside a package at run time, the proof
# cache's key $HOME/.una/proof-cache.key (ADR 0064), and then $HOME/.una when it is empty.
# A key file that UNA_PROOF_KEY names is the user's choice of file and is kept. The messages say
# what was removed and name what is left.
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
# Needs: curl or wget, tar, shasum or sha256sum, and id; on Linux, getconf.
set -eu

# The release this script installs by default; a test keeps it equal to the workspace version.
# The release commit bumps it together with the README's pins.
DEFAULT_VERSION=0.5.1

say() { printf '%s\n' "una install: $*"; }
fail() {
    printf '%s\n' "una install: error: $*" >&2
    exit 1
}

home=${UNA_HOME:-$HOME/.una}

# As root (#1938), every path the installer writes in or removes from must be one only root can
# change, or another user could redirect root's writes outside UNA_HOME, through a symbolic link
# or by replacing a directory or a file. `only_root PATH [above|file]` refuses PATH when it is a
# link, is not a directory (a regular file with `file`), is owned by another user than root, or
# is writable by its group or by others; a directory `above` may be writable by others when it is
# sticky. The owner is root by number, or this process's own user ([ -O ]), which is root
# whenever this check runs. Only the mode bits and the owner are read: an ACL that grants write
# (macOS shows it as a `+` after the mode) is not seen.
# The user is shown, never assumed: without `id -u`, the root checks could not tell whether to run.
uid=$(id -u 2> /dev/null) || fail "cannot read this process's user id (id -u)"
case $uid in
    "" | *[!0-9]*) fail "cannot read this process's user id (id -u printed '$uid')" ;;
esac
# Compared as a number, written without leading zeros: an `id` that prints `00` names root.
uid=${uid#"${uid%%[!0]*}"}
uid=${uid:-0}
only_root() {
    [ ! -L "$1" ] || fail "as root, the installer does not follow the symbolic link $1; as root, the installer writes only where only root can change"
    if [ "${2:-}" = file ]; then
        [ -f "$1" ] || fail "as root, $1 must be a regular file"
    else
        [ -d "$1" ] || fail "as root, $1 must be a directory"
    fi
    listing=$(ls -ldn "$1") || fail "as root, cannot read the owner and mode of $1"
    mode=$(printf '%s\n' "$listing" | awk 'NR == 1 { print $1 }')
    owner=$(printf '%s\n' "$listing" | awk 'NR == 1 { print $3 }')
    [ "$owner" = 0 ] || [ -O "$1" ] \
        || fail "as root, $1 must be owned by root, not by uid $owner; as root, the installer writes only where only root can change"
    case $mode in
        ?????w* | ????????w*)
            if [ "${2:-}" = above ]; then
                case $mode in ?????????[tT]*) return 0 ;; esac
            fi
            fail "as root, $1 must be writable only by its owner, root (its mode is $mode); as root, the installer writes only where only root can change"
            ;;
        [-d]?????????*) ;;
        *) fail "as root, cannot read the mode of $1 ($mode)" ;;
    esac
}
# `only_root_on REL`: every directory on the way from UNA_HOME to REL, as far as it exists.
only_root_on() {
    rest=$1
    on=$home
    while :; do
        case $rest in
            */*) on=$on/${rest%%/*}; rest=${rest#*/} ;;
            *) return 0 ;;
        esac
        [ -e "$on" ] || [ -L "$on" ] || return 0
        only_root "$on"
    done
}
# Where the work directory is made; as root, checked before it is made. A test points the one
# `default_tmp=/tmp` line at a directory of its own.
default_tmp=/tmp
tmp=${TMPDIR:-$default_tmp}
if [ "$uid" = 0 ]; then
    umask 022
    case $home in
        /*) ;;
        *) fail "as root, UNA_HOME must be an absolute path, not '$home'" ;;
    esac
    case /$home/ in
        */./* | */../*) fail "as root, UNA_HOME must have no '.' or '..' component: '$home'" ;;
    esac
    while :; do
        case $home in
            */) home=${home%/} ;;
            *) break ;;
        esac
    done
    [ -n "$home" ] || fail "as root, UNA_HOME must not be /"
    [ ! -L "$home" ] || fail "as root, the installer does not follow the symbolic link $home; as root, the installer writes only where only root can change"
    # The deepest directory of UNA_HOME that exists, as its physical path: the installer then
    # works on that path, so no link above it can be moved afterwards. What is missing below it,
    # the installer creates, as root.
    base=$home
    missing=
    while [ ! -e "$base" ] && [ ! -L "$base" ]; do
        missing=/${base##*/}$missing
        base=$(dirname "$base")
    done
    [ -d "$base" ] || fail "as root, $base must be a directory"
    physical=$(cd "$base" && pwd -P) || fail "as root, cannot resolve $base"
    only_root "$physical"
    above=$physical
    while [ "$above" != / ]; do
        above=$(dirname "$above")
        only_root "$above" above
    done
    home=${physical%/}$missing
    # The directories the installer writes in (a trailing / checks the directory itself), then
    # the files it writes, when they exist: `cp` and `>` would follow a link, and would keep the
    # owner and mode of a file that another user owns or can write.
    for rel in bin/ lib/ docs/guide/; do
        only_root_on "$rel"
    done
    for rel in bin/una bin/una.new lib/libuna_rt.a README.md LICENSE .una-install; do
        [ -e "$home/$rel" ] || [ -L "$home/$rel" ] || continue
        only_root "$home/$rel" file
    done
    if [ -d "$home/docs/guide" ]; then
        [ -z "$(find "$home/docs/guide" -type l | head -n 1)" ] \
            || fail "as root, $home/docs/guide holds a symbolic link, which the installer does not follow"
        for entry in "$home"/docs/guide/*; do
            [ -e "$entry" ] || continue
            [ -d "$entry" ] || only_root "$entry" file
        done
    fi
fi

marker=$home/.una-install

# `holds DIR`: the names in DIR, on one line, for a message.
holds() {
    ls -A "$1" 2> /dev/null | awk '{ printf "%s%s", (NR > 1 ? ", " : ""), $0 } END { if (NR == 0) printf "nothing" }'
}

# `no_link_on REL RECORDED`: no directory on the way from UNA_HOME to REL is a symbolic link, as
# far as it exists (ADR 0124 rule 32), for every user: `rm` and `rmdir` would follow it outside.
no_link_on() {
    rest=$1
    on=$home
    while :; do
        case $rest in
            */*) on=$on/${rest%%/*}; rest=${rest#*/} ;;
            *) return 0 ;;
        esac
        [ ! -L "$on" ] \
            || fail "the record $marker names '$2', which goes through the symbolic link $on; the installer follows no link in a recorded path"
        [ -e "$on" ] || return 0
    done
}
# `unlink_recorded KIND PATH`: removes a recorded file (`rm -f`) or an empty recorded directory
# (`rmdir`), after checking again, just before, that no directory on the way is a symbolic link
# (#2199 review 2). `check_record` ran before the download; this narrows the window in which a
# directory swapped for a link would be followed from the length of a download to the few system
# calls between the last check and the removal. A shell cannot close it: it has no `unlinkat` on
# a directory it holds open. Only someone who can write UNA_HOME while the installer runs can use
# that window, and as root every directory on the way was proven to be one only root can change.
unlink_recorded() {
    no_link_on "$2" "$2"
    if [ "$1" = file ]; then
        rm -f "${home:?}/$2"
    else
        no_link_on "$2/" "$2"
        rmdir "${home:?}/$2" 2> /dev/null || true
    fi
}
# `check_record`: every path of the record lies inside UNA_HOME, is spelled as the installer
# writes it, and goes through no symbolic link; as root, every directory on the way to it (and a
# recorded directory itself) is also one only root can change, or `rm` and `rmdir` could reach
# outside. Uninstall, and an installation over an earlier one, which removes what the new one
# does not hold, check every path before anything is downloaded or removed.
check_record() {
    while read -r kind rel; do
        case $kind in
            file | dir)
                case $rel in /* | *..* | "") fail "the record $marker names '$rel', which is not inside $home" ;; esac
                # One spelling per path, the one the installer writes: no `.` or empty component
                # (`./bin/una`, `bin//una`, `bin/`), which the comparison with the new files would
                # miss; the one exception is the record's own `dir .`.
                case $kind:$rel in
                    dir:.) ;;
                    *:. | *:./* | */. | */./* | *//* | */)
                        fail "the record $marker names '$rel', which has a '.' or empty component" ;;
                esac
                # As root, every directory on the way (and a recorded directory itself) is one
                # only root can change, which also refuses a link.
                if [ "$uid" = 0 ]; then
                    only_root_on "$rel"
                    [ "$kind" = file ] || only_root_on "$rel/"
                fi
                # Every user: no directory on the way is a link, nor a recorded directory itself.
                # The last component of a `file` line may be a link (`bin/z3` is one): `rm -f`
                # removes the link, never what it names.
                no_link_on "$rel" "$rel"
                [ "$kind" = file ] || no_link_on "$rel/" "$rel"
                ;;
        esac
    done < "$marker"
}

if [ "${1:-}" = --uninstall ]; then
    [ -f "$marker" ] \
        || fail "$home has no record of an installation by this script ($marker), so nothing is removed"
    # The files first, then the directories that the installation created, deepest first, when
    # they are empty. A path that leaves UNA_HOME is never followed.
    check_record
    while read -r kind rel; do
        [ "$kind" != file ] || unlink_recorded file "$rel"
    done < "$marker"
    # The proof cache's key (ADR 0064): `una` makes it on first use as $HOME/.una/proof-cache.key,
    # whatever UNA_HOME is, and it has no use without the installation. Only that one name is
    # removed, with `rm -f`, which removes a symbolic link and never what the link names; a
    # directory or anything else at the name is kept. As root, it is removed only when $HOME/.una
    # is UNA_HOME, which was checked above to be a place only root can change.
    keydir=
    key_note=
    if [ -n "${HOME:-}" ]; then
        keydir=$HOME/.una
        if [ "$uid" = 0 ]; then
            physical=$(cd "$keydir" 2> /dev/null && pwd -P) || physical=
            if [ "$physical" = "$home" ]; then
                keydir=$home
            else
                [ ! -e "$keydir/proof-cache.key" ] && [ ! -L "$keydir/proof-cache.key" ] \
                    || key_note="as root, kept the proof cache's key $keydir/proof-cache.key, which is outside $home"
                keydir=
            fi
        fi
    fi
    key_removed=
    if [ -n "$keydir" ]; then
        key=$keydir/proof-cache.key
        if [ -L "$key" ]; then
            rm -f "${keydir:?}/proof-cache.key"
            key_removed=1
            key_note="removed the proof cache's key $key, a symbolic link; the file it named is kept"
        elif [ -f "$key" ]; then
            rm -f "${keydir:?}/proof-cache.key"
            key_removed=1
            key_note="removed the proof cache's key $key"
        elif [ -e "$key" ]; then
            key_note="kept $key, which is not a file"
        fi
    fi
    dirs=$(sed -n 's/^dir //p' "$marker" | awk '{ print length($0) "\t" $0 }' | sort -rn | cut -f2-)
    printf '%s\n' "$dirs" | while read -r rel; do
        case $rel in
            "" | .) ;;
            *) unlink_recorded dir "$rel" ;;
        esac
    done
    # The record goes only now, once every recorded path was removed or kept: a check refused
    # above stops the uninstall with the record in place, so that it can be run again. Then
    # UNA_HOME itself, when the installation created it and it is empty.
    rm -f "$marker"
    if printf '%s\n' "$dirs" | grep -qx '\.'; then
        rmdir "$home" 2> /dev/null || true
    fi
    # $HOME/.una, when the key was removed from it and nothing else is there.
    [ -z "$key_removed" ] || rmdir "$keydir" 2> /dev/null || true
    if [ -d "$home" ]; then
        left=$(holds "$home")
        if [ "$left" = nothing ] && [ -z "$(ls -A "$home" 2> /dev/null)" ]; then
            say "removed Una from $home; the directory, which the installer did not create, is kept"
        else
            say "removed Una from $home; kept there, since the installer did not install them: $left"
        fi
    else
        say "removed $home"
    fi
    [ -z "$key_note" ] || say "$key_note"
    if [ -n "$key_removed" ] && [ "$keydir" != "$home" ] && [ -d "$keydir" ]; then
        say "kept $keydir, which holds: $(holds "$keydir")"
    fi
    # A key file that UNA_PROOF_KEY names is a file the user chose: it is named, never removed.
    if [ -n "${UNA_PROOF_KEY:-}" ] && { [ -e "$UNA_PROOF_KEY" ] || [ -L "$UNA_PROOF_KEY" ]; }; then
        say "kept the proof cache's key $UNA_PROOF_KEY, which UNA_PROOF_KEY names"
    fi
    say "remove the PATH line for $home/bin from your shell's profile too"
    exit 0
fi
[ "$#" -eq 0 ] || {
    printf '%s\n' "usage: sh install.sh [--uninstall]" >&2
    exit 2
}
# An earlier record, whose files this installation may remove: checked before any download.
[ ! -f "$marker" ] || check_record

# `replaceable REL [dir]`: what is at UNA_HOME/REL, where the installer writes, is something it
# may replace (#2216), for every user: nothing, a regular file, or a symbolic link that does not
# name a directory, which `rm -f` removes as itself, never what it names; with `dir`, nothing or
# a directory the installer writes in. A regular file or a link at a name the installer writes is
# replaced, whoever made it; anything else (a directory where a file goes, a FIFO, a file where
# a directory goes) is never removed to make room, and would stop the installation halfway,
# after `bin/una` was replaced: it is refused before anything is written. A link that names a
# directory is refused too: `mv` onto it would move `bin/una.new` into that directory, and one
# rule holds for every file target. `[ -f ]` and `[ -d ]` follow a link, so a link is told apart
# first; as root, the block above refused every link already. As an ordinary user, an existing
# directory the installer writes in must also be one this user can write in and enter, where
# `rm -f` and `cp` would fail halfway too (#2224 review 1); every file target lies in one of
# those directories, so this covers the directory each file is removed from. As root, the
# owner and mode checks above stand, unchanged.
replaceable() {
    at=$home
    [ "$1" = . ] || at=$home/$1
    [ -e "$at" ] || [ -L "$at" ] || return 0
    if [ "${2:-}" = dir ]; then
        [ -d "$at" ] \
            || fail "$at is not a directory, and the installer writes in it; it removes no file, directory or special file to make room for a directory, so nothing was changed"
        [ "$uid" = 0 ] || { [ -w "$at" ] && [ -x "$at" ]; } \
            || fail "$at is a directory this user cannot write in, and the installer writes in it; nothing was changed: make it writable (chmod u+wx) and run the installer again"
    elif [ -L "$at" ]; then
        [ ! -d "$at" ] \
            || fail "$at is a symbolic link to a directory, where the installer writes a file; it replaces only a regular file or a symbolic link that names no directory there, so nothing was changed"
    else
        [ -f "$at" ] \
            || fail "$at is neither a regular file nor a symbolic link, and the installer writes a file there; it replaces only a regular file or a symbolic link there, and removes no directory or special file to make room, so nothing was changed"
    fi
}
# The fixed targets of every installation, before anything is downloaded: UNA_HOME, `bin/`, the
# files the installer writes at fixed names and the record. The runtime and the guides, which
# only some archives hold, are checked once the archive is unpacked, still before anything is
# written, so that a release that installs no runtime keeps a directory at `lib/libuna_rt.a`.
replaceable . dir
replaceable bin dir
for rel in bin/una bin/una.new README.md LICENSE .una-install; do
    replaceable "$rel"
done

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
# built on the glibc of the build image (ADR 0124 rule 1), so it does not start on musl or on an
# older glibc. That glibc is 2.28 (AlmaLinux 8, #2105), the floor below, for the releases after
# 0.5.1; the releases 0.4.0 to 0.5.1 were built on glibc 2.39 (Ubuntu 24.04) and keep that floor,
# so that UNA_VERSION naming one of them is refused where its `una` could not start.
# scripts/check-release.sh reads `glibc_floor` and refuses an archive whose `una` asks for a later
# symbol version. glibc is shown, never assumed: `getconf GNU_LIBC_VERSION` must print
# `glibc MAJOR.MINOR` at or above the floor, compared as numbers (2.4 is before 2.28). The C
# library is the one the running system reports and loads (#1938), never inferred from a file
# being there: a glibc system that also has musl's loader installed (Debian's `musl` package) is
# glibc. A musl system is refused even when getconf's answer looks like glibc's: its
# `ldd --version` names musl, or the aarch64 loader that `una` needs, /lib/ld-linux-aarch64.so.1,
# is missing, is musl's (as with gcompat), or does not say it is glibc's when asked its version.
# musl's own loader, /lib/ld-musl-*, only names the reason when getconf shows no glibc. The
# userland must be 64-bit (`getconf LONG_BIT`): a 32-bit Arm userland on a 64-bit kernel reports
# aarch64, but cannot start the aarch64 `una`.
glibc_floor=2.28
# The last release built on glibc 2.39, and that floor.
glibc_239_through=0.5.1
if awk -v v="$version" -v last="$glibc_239_through" 'BEGIN {
    split(v, a, "."); split(last, b, ".")
    for (i = 1; i <= 3; i++) if (a[i] + 0 != b[i] + 0) exit !(a[i] + 0 < b[i] + 0)
    exit 0
}'; then
    glibc_floor_of_version=2.39
else
    glibc_floor_of_version=$glibc_floor
fi
# Where the loaders are; a test points this one line at a fake machine's.
loaders=/lib
if [ "${platform%% *}" = Linux ]; then
    needs="the una of $version needs glibc $glibc_floor_of_version or later; Alpine and other musl systems are not supported"
    if ldd --version 2>&1 | grep -qi musl; then
        fail "this system's C library is musl (ldd --version); $needs"
    fi
    libc=$(getconf GNU_LIBC_VERSION 2> /dev/null) || libc=
    glibc=${libc#glibc }
    case $libc in
        "glibc "[0-9]*.[0-9]*) ;;
        *) glibc= ;;
    esac
    case $glibc in
        "" | *[!0-9.]* | *..* | *.)
            for loader in "$loaders"/ld-musl-*; do
                [ -e "$loader" ] && fail "this system's C library is musl: getconf GNU_LIBC_VERSION shows no glibc, and musl's loader is $loader; $needs"
            done
            fail "cannot read this system's glibc version (getconf GNU_LIBC_VERSION); $needs"
            ;;
    esac
    awk -v have="$glibc" -v floor="$glibc_floor_of_version" 'BEGIN {
        split(have, h, "."); split(floor, f, ".")
        exit !(h[1] + 0 > f[1] + 0 || (h[1] + 0 == f[1] + 0 && h[2] + 0 >= f[2] + 0))
    }' || fail "this system has glibc $glibc; $needs"
    bits=$(getconf LONG_BIT 2> /dev/null) || bits=
    [ "$bits" = 64 ] \
        || fail "this system's userland is not 64-bit (getconf LONG_BIT: '$bits'): a 32-bit userland on a 64-bit kernel cannot start the aarch64 una; the Linux release needs a 64-bit Arm userland"
    aarch64_loader=$loaders/ld-linux-aarch64.so.1
    if answer=$("$aarch64_loader" --version 2>&1); then ran=1; else ran=; fi
    case $answer in
        *musl*) fail "this system's C library is musl: $aarch64_loader is musl's loader; $needs" ;;
    esac
    case $ran$answer in
        1*GLIBC* | 1*"GNU libc"*) ;;
        *) fail "glibc's aarch64 loader $aarch64_loader does not answer as glibc's ($aarch64_loader --version); $needs" ;;
    esac
fi

if command -v curl > /dev/null 2>&1; then
    fetch() { curl --fail --location --silent --show-error --retry 3 --output "$2" "$1"; }
elif command -v wget > /dev/null 2>&1; then
    fetch() { wget --quiet --output-document="$2" "$1"; }
else
    fail "neither curl nor wget is installed"
fi
# The digest of the file's bytes, read on stdin: a file name is never printed, so that a work
# directory whose name holds a newline is not escaped into the digest's line.
if command -v sha256sum > /dev/null 2>&1; then
    digest() { sha256sum < "$1" | cut -d ' ' -f 1; }
elif command -v shasum > /dev/null 2>&1; then
    digest() { shasum -a 256 < "$1" | cut -d ' ' -f 1; }
else
    fail "neither sha256sum nor shasum is installed"
fi
for tool in tar; do
    command -v "$tool" > /dev/null 2>&1 || fail "$tool is not installed"
done

# As root, the work directory, where the archive is checked and unpacked: in TMPDIR (default
# /tmp), which must be one only root can change as well, though it may be sticky, as /tmp is (an
# entry root makes there cannot be renamed by another user). Otherwise another user could
# replace the checked archive before it is unpacked. Checked here, before anything is
# downloaded, since uninstall makes no work directory; the work directory is made in the
# physical path that was checked.
if [ "$uid" = 0 ]; then
    [ -d "$tmp" ] || fail "as root, TMPDIR ($tmp) must be a directory"
    tmp=$(cd "$tmp" && pwd -P) || fail "as root, cannot resolve TMPDIR ($tmp)"
    only_root "$tmp" above
    above=$tmp
    while [ "$above" != / ]; do
        above=$(dirname "$above")
        only_root "$above" above
    done
fi
work=$(mktemp -d "$tmp/una-install.XXXXXX")
trap 'rm -rf "${work:?}"' EXIT
[ "$uid" != 0 ] || only_root "$work"

# The validation summary of the release: its verdict, and the archive's digest. It comes from
# the repository's tree, never from the release's own assets, so that the archive is checked
# against a digest the release cannot replace (#717).
record=$version.md
say "reading the validation summary of $version"
fetch "$records/v$version/docs/validation/$record" "$work/$record" \
    || fetch "$records/main/docs/validation/$record" "$work/$record" \
    || fail "no validation summary for $version could be read (docs/validation/$record, at v$version or on main), so this script does not install it; check that $version is released and that the repository can be reached"
grep -qx -e 'Verdict: PASS (every gate)' -e 'Verdict: ACCEPTED BY THE OWNER' "$work/$record" \
    || fail "the validation summary of $version is not a PASS of every gate, and is not accepted by the owner"
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
# The targets that only some archives hold, checked before anything is written, as the fixed
# ones were before the download (#2216): the runtime and its directory, then the guides' names,
# which only the archive gives. The record is one path per word, and `cp` takes options: a
# guide's name that would break either is refused, though no released archive has one.
if [ -n "$runtime" ]; then
    replaceable lib dir
    replaceable "$runtime"
fi
if [ -d "$unpacked/docs/guide" ]; then
    replaceable docs dir
    replaceable docs/guide dir
fi
for guide in "$unpacked"/docs/guide/*.md; do
    [ -f "$guide" ] || continue
    base=$(basename "$guide")
    case $base in
        -* | *[[:space:]]*) fail "$archive holds a guide named '$base', which this installer refuses" ;;
    esac
    replaceable "docs/guide/$base"
done

# The directories this installation creates, kept from an earlier record when there is one, so
# that uninstall removes them and no other.
created=$work/created
: > "$created"
[ -f "$marker" ] && sed -n 's/^dir //p' "$marker" > "$created"
for dir in . bin ${runtime:+lib}; do
    [ -d "$home/$dir" ] || echo "$dir" >> "$created"
done
# Each file is written as a new one: an existing name is removed first (#1966), so that `cp` and
# `>` never write into an existing file, which could be a hard link to another file (as root, one
# outside UNA_HOME). `rm -f` removes the name, never another link to the same file.
mkdir -p "$home/bin"
rm -f "$home/bin/una.new"
cp "$unpacked/una" "$home/bin/una.new" && mv "$home/bin/una.new" "$home/bin/una"
chmod +x "$home/bin/una"
files="bin/una"
if [ -n "$runtime" ]; then
    mkdir -p "$home/lib"
    rm -f "${home:?}/$runtime"
    cp "$unpacked/$runtime" "$home/$runtime"
    files="$files $runtime"
fi
for doc in README.md LICENSE; do
    if [ -f "$unpacked/$doc" ]; then
        rm -f "${home:?}/$doc"
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
        rm -f "$home/docs/guide/$base"
        cp "$guide" "$home/docs/guide/$base"
        files="$files docs/guide/$base"
    done
fi
# The migration of ADR 0124 rule 32 (#1992), now that the new files are in place: each file of
# the earlier record that this installation does not hold (the `z3` and its link in `bin/` of a
# release before 0.4.0, the `lib/libuna_rt.a` of one before ADR 0124 step 2, a guide a later
# release dropped) is removed, by the paths checked above, so that no file stays that no record
# lists and no stale program stays on PATH. `rm -f` removes a symbolic link, never what it names;
# a path that is now a directory is not a file of the installation and is kept. Then the
# directories of that record left empty, deepest first; the record keeps those that remain.
# A recorded name that is the same file or directory as one this installation holds, though
# spelled otherwise (`BIN/una` or `README.MD` on a file system that ignores case, a name in
# another Unicode normalization), is never removed (#2199 review 2): `held_new PATH` compares
# with `-ef` (the same device and inode), which dash, bash, busybox and macOS's sh all have. A
# symbolic link is never a file this installation wrote, so a recorded link is removed as itself.
held_new() {
    [ ! -L "$home/$1" ] || return 1
    for new in $files; do
        [ "$home/$1" -ef "$home/$new" ] && return 0
        up=$new
        while :; do
            case $up in
                */*) up=${up%/*} ;;
                *) break ;;
            esac
            [ "$home/$1" -ef "$home/$up" ] && return 0
        done
    done
    return 1
}
removed=
if [ -f "$marker" ]; then
    while read -r kind rel; do
        [ "$kind" = file ] || continue
        case " $files " in *" $rel "*) continue ;; esac
        ! held_new "$rel" || continue
        if [ -L "$home/$rel" ] || [ -f "$home/$rel" ]; then
            unlink_recorded file "$rel"
            removed="$removed $rel"
        fi
    done < "$marker"
    sed -n 's/^dir //p' "$marker" | awk '{ print length($0) "\t" $0 }' | sort -rn | cut -f2- \
        | while read -r rel; do
            case $rel in
                "" | .) ;;
                *) held_new "$rel" || unlink_recorded dir "$rel" ;;
            esac
        done
fi
rm -f "$marker"
{
    echo "# Una $version, installed by install.sh; \`sh install.sh --uninstall\` removes what follows."
    echo "version $version"
    sort -u "$created" | while read -r dir; do
        [ ! -d "$home/$dir" ] || echo "dir $dir"
    done
    for file in $files; do echo "file $file"; done
} > "$marker"

# The version is the one the validation summary vouched for. From 0.3.0 `una --version` also
# names the editions it serves (#305); an older `una` has no such flag, and prints nothing here.
say "installed una $version in $home"
if [ -n "$removed" ]; then
    say "removed what the earlier installation held and $version does not:$removed"
    case " $removed " in
        *" bin/z3 "* | *" z3/bin/z3 "*)
            say "removed z3, which una no longer needs: the solver is built into una; a z3 installed elsewhere is not touched" ;;
    esac
fi
# As root, the installed `una` is not run (#1938): `una --version` as an ordinary user shows it.
if [ "$uid" = 0 ]; then
    say "as root, the installer does not run the installed una; run una --version as an ordinary user"
else
    "$home/bin/una" --version 2>/dev/null | sed 's/^/    /' || true
fi
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
if [ "${platform%% *}" = Linux ] || [ "$uid" = 0 ]; then
    say "una check and every command that proves refuse to run as root: run them as an ordinary user"
fi
say "uninstall with: sh install.sh --uninstall"
