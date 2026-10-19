#!/bin/sh
# The `una` that the site gate checks the Learn book, Una by Example, with (#1039):
# scripts/site-website.sh calls it, and the release gate `site` runs its self-test first.
#
# The book pins the una release it is written for in its una.lock (`version = "X"`), and its
# expected outputs are that release's. The book is checked with this checkout's `una` while that
# release is being prepared: this checkout's version is the lock's, and it holds no validation
# record docs/validation/<version>.md yet, since a release commits its record with its tag. Once
# the release is published, or when the versions differ, a development commit's `una` may be
# ahead of the pin (the version in Cargo.toml stays the released one until the next release), so
# the book is checked with the published release of the lock's version: this host's archive, una-<version>-<target>.tar.gz, is downloaded with `gh release download`
# from github.com/una-lang/una, its SHA-256 must be the one that release's validation record,
# docs/validation/<version>.md of this checkout, gives it, and the unpacked `una --version` must
# name the lock's version. A lock without exactly one version line, a version with no record or
# no digest for this host's archive, a digest that differs and a binary of another version fail.
# A release's own gate checks the book with the release's `una`, so a book is re-released for a
# new una before the tag: scripts/release-gates.sh sets UNA_SITE_LEARN_SAME_VERSION=1, and then
# a lock that pins another version fails instead of falling back to a published release, and
# this checkout's `una` is used.
#
# UNA_SITE_LEARN_UNA_DIR names a directory that holds the archive in place of the download, for a
# local test; the release gates unset it.
#
# Usage: scripts/site-learn-una.sh LEARN UNA OUT
#          LEARN  the clone of una-by-example, which holds una.lock
#          UNA    this checkout's `una`
#          OUT    a directory for the download and the unpacked release
#        prints the path of the `una` to check the book with; what it chose goes to stderr.
#        scripts/site-learn-una.sh --self-test
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)

fail() {
    echo "site-learn-una: FAIL ($1)" >&2
    exit 1
}

# The target of this host's release archive.
host_target() {
    case "$(uname -s) $(uname -m)" in
        "Darwin arm64") echo aarch64-apple-darwin ;;
        "Linux aarch64" | "Linux arm64") echo aarch64-unknown-linux-gnu ;;
        "Linux x86_64") echo x86_64-unknown-linux-gnu ;;
        *) fail "no release archive for $(uname -s) $(uname -m)" ;;
    esac
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    else
        shasum -a 256 "$1" | cut -d ' ' -f 1
    fi
}

# The `una` for the book of LEARN: this checkout's VERSION and UNA, the validation records in
# RECORDS, the download into OUT.
resolve() {
    learn=$1 version=$2 una=$3 records=$4 out=$5
    [ -f "$learn/una.lock" ] || fail "una-by-example has no una.lock"
    count=$(grep -c '^version = "' "$learn/una.lock" || true)
    [ "$count" -eq 1 ] || fail "una.lock must have exactly one version line, and has $count"
    pinned=$(sed -n 's/^version = "\([0-9][0-9.]*\)"$/\1/p' "$learn/una.lock")
    [ -n "$pinned" ] || fail "the version line of una.lock is not a version"
    record="$records/$pinned.md"
    if [ "${UNA_SITE_LEARN_SAME_VERSION:-}" = 1 ]; then
        [ "$pinned" = "$version" ] \
            || fail "this is una $version, and una.lock pins $pinned: a release checks the book with its own una, so release una-by-example for $version and pin it first"
        echo "una-by-example: checked with this release's una $version, the version its una.lock pins" >&2
        echo "$una"
        return 0
    fi
    if [ "$pinned" = "$version" ] && [ ! -f "$record" ]; then
        echo "una-by-example: checked with this checkout's una $version, the version its una.lock pins, not yet released" >&2
        echo "$una"
        return 0
    fi
    target=$(host_target)
    name="una-$pinned-$target.tar.gz"
    [ -f "$record" ] || fail "una.lock pins $pinned, which has no validation record $record"
    digest=$(sed -n "s/^| \`$name\` | \`\([0-9a-f]\{64\}\)\` |\$/\1/p" "$record")
    [ "$(printf '%s\n' "$digest" | grep -c .)" -eq 1 ] \
        || fail "the validation record of $pinned gives no single digest for $name"
    download=$out/una-release
    rm -rf "${download:?}"
    mkdir -p "$download"
    if [ -n "${UNA_SITE_LEARN_UNA_DIR:-}" ]; then
        echo "una-by-example: the archive is read from $UNA_SITE_LEARN_UNA_DIR, in place of the download" >&2
        cp "$UNA_SITE_LEARN_UNA_DIR/$name" "$download/" 2>/dev/null || fail "$UNA_SITE_LEARN_UNA_DIR has no $name"
    else
        gh release download "v$pinned" -R github.com/una-lang/una -p "$name" -D "$download" >&2 \
            || fail "cannot download $name of release v$pinned"
    fi
    found=$(sha256_of "$download/$name")
    [ "$found" = "$digest" ] || fail "$name has SHA-256 $found, and its validation record gives $digest"
    tar -xzf "$download/$name" -C "$download" || fail "$name does not unpack"
    pinned_una=$download/una-$pinned-$target/una
    [ -x "$pinned_una" ] || fail "$name holds no una-$pinned-$target/una"
    said=$("$pinned_una" --version | head -n 1)
    [ "$said" = "una $pinned" ] || fail "the una of $name says \"$said\", not \"una $pinned\""
    echo "una-by-example: this checkout is una $version, and una.lock pins $pinned: checked with release v$pinned, $name, SHA-256 $digest" >&2
    echo "$pinned_una"
}

self_test() {
    t=$(mktemp -d "${TMPDIR:-/tmp}/site-learn-una.XXXXXX")
    trap 'rm -rf "${t:?}"' EXIT
    target=$(host_target)
    failures=0
    # A release 9.8.7 whose `una` prints its version, packed as the release packs it.
    mkdir -p "$t/pack/una-9.8.7-$target" "$t/release" "$t/records" "$t/learn"
    printf '#!/bin/sh\necho "una ${FAKE_UNA_VERSION:-9.8.7}"\n' >"$t/pack/una-9.8.7-$target/una"
    chmod 755 "$t/pack/una-9.8.7-$target/una"
    tar -czf "$t/release/una-9.8.7-$target.tar.gz" -C "$t/pack" "una-9.8.7-$target"
    good=$(sha256_of "$t/release/una-9.8.7-$target.tar.gz")
    record() {
        printf '| Archive | sha256 |\n| --- | --- |\n| `una-9.8.7-%s.tar.gz` | `%s` |\n' "$target" "$1" >"$t/records/9.8.7.md"
    }
    lock() {
        printf '%s\n' '# c' "$@" 'commit = "0000000000000000000000000000000000000000"' >"$t/learn/una.lock"
    }
    # expect pass|fail WHAT VERSION [VAR=value ...]: resolves with this checkout at VERSION.
    expect() {
        want=$1 what=$2 version=$3
        shift 3
        if (for assignment in "$@"; do export "${assignment?}"; done
            resolve "$t/learn" "$version" "$t/this-una" "$t/records" "$t/out") >"$t/path" 2>"$t/log"; then
            got=pass
        else
            got=fail
        fi
        if [ "$got" = "$want" ]; then
            echo "  ok: $what ($want)"
        else
            echo "  WRONG: $what: expected $want, got $got"
            sed 's/^/    /' "$t/log"
            failures=$((failures + 1))
        fi
    }
    mkdir -p "$t/out"
    lock 'version = "9.8.7"'
    expect pass "the pinned version, not yet released" 9.8.7
    [ "$(cat "$t/path")" = "$t/this-una" ] || { echo "  WRONG: an unreleased pinned version does not give this checkout's una"; failures=$((failures + 1)); }
    record "$good"
    expect pass "the pinned version, released, with this checkout past it" 9.8.7 UNA_SITE_LEARN_UNA_DIR="$t/release"
    [ "$(cat "$t/path")" = "$t/out/una-release/una-9.8.7-$target/una" ] \
        || { echo "  WRONG: a released pinned version does not give the release's una"; failures=$((failures + 1)); }
    expect pass "a checkout ahead of the pin" 9.9.0 UNA_SITE_LEARN_UNA_DIR="$t/release"
    [ "$(cat "$t/path")" = "$t/out/una-release/una-9.8.7-$target/una" ] \
        || { echo "  WRONG: a checkout ahead does not give the pinned release's una"; failures=$((failures + 1)); }
    expect fail "a release gate, whose versions must be equal" 9.9.0 UNA_SITE_LEARN_UNA_DIR="$t/release" UNA_SITE_LEARN_SAME_VERSION=1
    expect pass "a release gate with equal versions" 9.8.7 UNA_SITE_LEARN_SAME_VERSION=1
    [ "$(cat "$t/path")" = "$t/this-una" ] || { echo "  WRONG: a release gate does not give this checkout's una"; failures=$((failures + 1)); }
    expect fail "a binary of another version" 9.9.0 UNA_SITE_LEARN_UNA_DIR="$t/release" FAKE_UNA_VERSION=9.9.0
    record 0000000000000000000000000000000000000000000000000000000000000000
    expect fail "a digest that differs from the record's" 9.9.0 UNA_SITE_LEARN_UNA_DIR="$t/release"
    rm "$t/records/9.8.7.md"
    expect fail "no validation record" 9.9.0 UNA_SITE_LEARN_UNA_DIR="$t/release"
    printf '| Archive | sha256 |\n' >"$t/records/9.8.7.md"
    expect fail "a record without this host's archive" 9.9.0 UNA_SITE_LEARN_UNA_DIR="$t/release"
    record "$good"
    expect fail "no archive in the local directory" 9.9.0 UNA_SITE_LEARN_UNA_DIR="$t/records"
    lock
    expect fail "a lock without a version" 9.9.0 UNA_SITE_LEARN_UNA_DIR="$t/release"
    lock 'version = "9.8.7"' 'version = "9.8.6"'
    expect fail "a lock with two versions" 9.9.0 UNA_SITE_LEARN_UNA_DIR="$t/release"
    lock 'version = "latest"'
    expect fail "a version that is not a version" 9.9.0 UNA_SITE_LEARN_UNA_DIR="$t/release"
    [ "$failures" -eq 0 ] || fail "self-test: $failures case(s) wrong"
    echo "site-learn-una: self-test PASS"
}

case "${1:-}" in
    --self-test)
        [ $# -eq 1 ] || fail "usage: scripts/site-learn-una.sh --self-test"
        self_test
        ;;
    *)
        [ $# -eq 3 ] || {
            echo "usage: scripts/site-learn-una.sh LEARN UNA OUT | --self-test" >&2
            exit 2
        }
        version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n 1)
        [ -n "$version" ] || fail "Cargo.toml has no version"
        resolve "$1" "$version" "$2" "$root/docs/validation" "$3"
        ;;
esac
