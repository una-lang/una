#!/bin/sh
# The Una by Example release that the website's Learn pages link their downloads to (una#831),
# compared with the archives scripts/site-website.sh builds from the pinned commit: that script
# calls it, in the website's own flow (#2081), and crates/una-cli/tests/site_website.rs runs its
# self-test.
#
# The release is the `release` line of the website's learn.lock, read with the website's grammar
# (scripts/preview/learn.mjs, readLock: any spacing around `=`, trailing spacing, CRLF). A lock
# with no release line names none, and nothing is compared; a release line that does not parse,
# a release that is not a plain tag name, and more than one release line fail. The release must be
# published, not a draft, and hold exactly the files the build makes, every archive and
# SHA256SUMS: the same names, none missing and none extra, nothing but regular files, each file
# the same bytes, and each archive the digest SHA256SUMS gives it. The output names the source
# compared and the SHA-256 of every file, so the record says what was compared.
#
# The release is downloaded with `gh release download` from github.com, so `gh` must be signed in
# with read access to the repository. UNA_SITE_LEARN_RELEASE_DIR names a directory that holds the
# release's files in place of the download, for a local test; a check for a release leaves it unset.
#
# Usage: scripts/site-learn-release.sh LOCK ARCHIVES OUT
#          LOCK      the website's learn.lock
#          ARCHIVES  the directory of the archives the build made (scripts/archive.py)
#          OUT       a directory for the download and the file lists
#        scripts/site-learn-release.sh --release-of LOCK
#          prints the release the lock names, or nothing
#        scripts/site-learn-release.sh --self-test
set -eu

fail() {
    echo "site-learn-release: FAIL ($1)" >&2
    exit 1
}

# The release of the lock $1, on stdout: empty when the lock has no release line.
release_of() {
    lines=$(grep -c '^[[:space:]]*release[[:space:]]*=' "$1" || true)
    [ "$lines" -le 1 ] || fail "$1 has $lines release lines"
    [ "$lines" -eq 1 ] || return 0
    value=$(sed -n 's/^release[[:space:]]*=[[:space:]]*"\([^"]*\)"[[:space:]]*$/\1/p' "$1")
    [ -n "$value" ] || fail "$1 has a release line that does not parse as release = \"NAME\""
    printf '%s\n' "$value" | grep -Eqx '[A-Za-z0-9][A-Za-z0-9._-]*' \
        || fail "$1 names a release that is not a plain tag name"
    printf '%s\n' "$value"
}

# The regular files under $1, as ./NAME lines sorted bytewise, into $2; anything under $1 that is
# not a regular file or a directory, such as a symbolic link, fails.
files_of() {
    [ -z "$(find "$1" ! -type f ! -type d)" ] || fail "$1 holds something that is not a regular file or a directory"
    (cd "$1" && find . -type f) >"$2.unsorted" || fail "cannot list $1"
    LC_ALL=C sort "$2.unsorted" >"$2" || fail "cannot sort the files of $1"
}

compare() {
    lock=$1 archives=$2 out=$3
    [ -d "${out:?}" ] || fail "$out is not a directory"
    release=$(release_of "$lock") || exit 1
    if [ -z "$release" ]; then
        echo "una-by-example: no release named (learn.lock has no release line), so none is compared"
        return 0
    fi
    repositories=$(grep -c '^[[:space:]]*repository[[:space:]]*=' "$lock" || true)
    [ "$repositories" -eq 1 ] || fail "$lock has $repositories repository lines"
    repository=$(sed -n 's/^repository[[:space:]]*=[[:space:]]*"\([^"]*\)"[[:space:]]*$/\1/p' "$lock")
    slug=$(printf '%s\n' "$repository" | sed -n 's#^https://github\.com/\([A-Za-z0-9._-]*/[A-Za-z0-9._-]*\)\.git$#\1#p')
    [ -n "$slug" ] || fail "learn.lock does not name a GitHub repository as https://github.com/OWNER/NAME.git"
    released=${out:?}/una-by-example-release
    rm -rf "${released:?}"
    if [ -n "${UNA_SITE_LEARN_RELEASE_DIR:-}" ]; then
        [ -d "$UNA_SITE_LEARN_RELEASE_DIR" ] || fail "UNA_SITE_LEARN_RELEASE_DIR is not a directory"
        cp -R "$UNA_SITE_LEARN_RELEASE_DIR" "$released"
        source="the directory $UNA_SITE_LEARN_RELEASE_DIR, in place of the release $release of github.com/$slug"
    else
        command -v gh >/dev/null 2>&1 || fail "gh is needed to read the release $release"
        draft=$(gh release view "$release" -R "github.com/$slug" --json isDraft -q .isDraft) \
            || fail "cannot read the release $release of github.com/$slug"
        [ "$draft" = false ] || fail "the release $release of github.com/$slug is not published (isDraft: $draft)"
        mkdir -p "$released"
        gh release download "$release" -R "github.com/$slug" -D "$released" \
            || fail "cannot download the release $release of github.com/$slug"
        source="the release $release of github.com/$slug"
    fi
    files_of "$archives" "$out/una-by-example-built.txt"
    files_of "$released" "$out/una-by-example-released.txt"
    grep -qx './SHA256SUMS' "$out/una-by-example-built.txt" || fail "the build made no SHA256SUMS"
    diff "$out/una-by-example-built.txt" "$out/una-by-example-released.txt" >&2 \
        || fail "$source does not hold exactly the files the build makes (< built only, > released only)"
    while IFS= read -r file; do
        cmp -s "$archives/$file" "$released/$file" || fail "$source has another ${file#./} than the build makes"
    done <"$out/una-by-example-built.txt"
    # Every file's digest, which the output records; each archive's must be the one SHA256SUMS
    # gives it, and SHA256SUMS must name every archive.
    python3 -B - "$released" "$out/una-by-example-released.txt" <<'EOF' || fail "$source: SHA256SUMS does not give each archive its digest"
import hashlib, sys
root, listing = sys.argv[1], sys.argv[2]
names = [line.rstrip("\n")[2:] for line in open(listing, encoding="utf-8")]
digest = {n: hashlib.sha256(open(f"{root}/{n}", "rb").read()).hexdigest() for n in names}
for n in names:
    print(f"  {digest[n]}  {n}")
listed = {}
for line in open(f"{root}/SHA256SUMS", encoding="utf-8"):
    value, _, name = line.rstrip("\n").partition("  ")
    listed[name] = value
archives = {n: d for n, d in digest.items() if n != "SHA256SUMS"}
sys.exit(0 if listed == archives else 1)
EOF
    echo "una-by-example: $source holds the $(wc -l <"$out/una-by-example-built.txt" | tr -d ' ') files the build makes, byte for byte"
}

self_test() {
    t=$(mktemp -d "${TMPDIR:-/tmp}/site-learn-release.XXXXXX")
    trap 'rm -rf "${t:?}"' EXIT
    mkdir -p "$t/bin" "$t/build" "$t/gh-release"
    printf 'one\n' >"$t/build/01-a.tar.gz"
    printf 'two\n' >"$t/build/02-b.tar.gz"
    python3 -B -c 'import hashlib, sys
for n in sys.argv[2:]:
    print(hashlib.sha256(open(sys.argv[1] + "/" + n, "rb").read()).hexdigest() + "  " + n)' \
        "$t/build" 01-a.tar.gz 02-b.tar.gz >"$t/build/SHA256SUMS"
    cp "$t/build/"* "$t/gh-release/"
    # A gh that serves "$t/gh-release" for github.com/o/n's release r1 only, a draft when
    # FAKE_GH_DRAFT says so, and a download that fails or brings part of the files when
    # FAKE_GH_DOWNLOAD says so; any other repository or host fails.
    cat >"$t/bin/gh" <<EOF
#!/bin/sh
[ "\$1 \$2" = "release view" ] || [ "\$1 \$2" = "release download" ] || exit 9
[ "\$3" = r1 ] && [ "\$4" = -R ] && [ "\$5" = github.com/o/n ] || { echo "fake gh: \$*" >&2; exit 9; }
if [ "\$2" = view ]; then echo "\${FAKE_GH_DRAFT:-false}"; exit 0; fi
[ "\$6" = -D ] || exit 9
[ "\${FAKE_GH_DOWNLOAD:-}" = fail ] && exit 1
if [ "\${FAKE_GH_DOWNLOAD:-}" = partial ]; then cp "$t/gh-release/01-a.tar.gz" "\$7/"; exit 0; fi
cp "$t/gh-release/"* "\$7/"
EOF
    chmod +x "$t/bin/gh"
    lock() { printf '%s\n' 'repository = "https://github.com/o/n.git"' 'commit = "0000000000000000000000000000000000000000"' "$@" >"$t/lock"; }
    failures=0
    expect() { # pass|fail, name, then the command's environment and arguments
        want=$1 name=$2
        shift 2
        rm -rf "${t:?}/out"
        mkdir -p "$t/out"
        if env PATH="$t/bin:$PATH" "$@" "$0" "$t/lock" "$t/build" "$t/out" >"$t/log" 2>&1; then got=pass; else got=fail; fi
        if [ "$got" = "$want" ]; then
            echo "  ok: $name ($got)"
        else
            echo "  WRONG: $name: $got, expected $want"
            sed 's/^/    /' "$t/log"
            failures=$((failures + 1))
        fi
    }
    variant() { # a copy of the release, changed by the shell command $2, as $t/$1
        rm -rf "${t:?}/$1"
        cp -R "$t/gh-release" "$t/$1"
        (cd "$t/$1" && sh -c "$2")
    }
    lock 'release = "r1"'
    expect pass "the release, through gh" UNA_SITE_LEARN_RELEASE_DIR=
    grep -q 'the release r1 of github.com/o/n holds the 3 files' "$t/log" || { echo "  WRONG: the output does not name the release"; failures=$((failures + 1)); }
    grep -q "$(sed -n 's/  01-a.tar.gz$//p' "$t/build/SHA256SUMS")  01-a.tar.gz" "$t/log" || { echo "  WRONG: the output does not record the digests"; failures=$((failures + 1)); }
    expect pass "the release, from a directory" UNA_SITE_LEARN_RELEASE_DIR="$t/gh-release"
    grep -q "the directory $t/gh-release, in place of the release r1" "$t/log" || { echo "  WRONG: the output does not name the directory"; failures=$((failures + 1)); }
    expect fail "a draft release" UNA_SITE_LEARN_RELEASE_DIR= FAKE_GH_DRAFT=true
    expect fail "a download that fails after the release is read" UNA_SITE_LEARN_RELEASE_DIR= FAKE_GH_DOWNLOAD=fail
    grep -q 'cannot download the release r1' "$t/log" || { echo "  WRONG: a failed download is not what fails"; failures=$((failures + 1)); }
    expect fail "a download that brings part of the files" UNA_SITE_LEARN_RELEASE_DIR= FAKE_GH_DOWNLOAD=partial
    # A PATH that holds only the tools the script runs, so no gh is on it on any host.
    mkdir -p "$t/nogh"
    for tool in sh grep sed find sort diff cmp cp mkdir rm python3 wc tr env cat; do
        ln -s "$(command -v "$tool")" "$t/nogh/$tool"
    done
    expect fail "no gh" UNA_SITE_LEARN_RELEASE_DIR= PATH="$t/nogh"
    grep -q 'gh is needed' "$t/log" || { echo "  WRONG: no gh is not what fails"; failures=$((failures + 1)); }
    # The build's own output: a symbolic link in it, and no SHA256SUMS, fail too.
    ln -s 01-a.tar.gz "$t/build/05-e.tar.gz"
    expect fail "a symbolic link in the build" UNA_SITE_LEARN_RELEASE_DIR="$t/gh-release"
    rm "$t/build/05-e.tar.gz"
    mv "$t/build/SHA256SUMS" "$t/SHA256SUMS.kept"
    expect fail "a build without SHA256SUMS" UNA_SITE_LEARN_RELEASE_DIR="$t/gh-release"
    grep -q 'the build made no SHA256SUMS' "$t/log" || { echo "  WRONG: a build without SHA256SUMS is not what fails"; failures=$((failures + 1)); }
    mv "$t/SHA256SUMS.kept" "$t/build/SHA256SUMS"
    variant byte 'printf x >>01-a.tar.gz'
    expect fail "an archive with one more byte" UNA_SITE_LEARN_RELEASE_DIR="$t/byte"
    variant extra 'printf x >03-c.tar.gz'
    expect fail "an extra file" UNA_SITE_LEARN_RELEASE_DIR="$t/extra"
    variant missing 'rm 02-b.tar.gz'
    expect fail "a missing file" UNA_SITE_LEARN_RELEASE_DIR="$t/missing"
    variant link 'ln -s 01-a.tar.gz 04-d.tar.gz'
    expect fail "a symbolic link" UNA_SITE_LEARN_RELEASE_DIR="$t/link"
    variant sums 'printf " " >>SHA256SUMS'
    expect fail "a changed SHA256SUMS" UNA_SITE_LEARN_RELEASE_DIR="$t/sums"
    # A release that agrees with itself, an archive changed and SHA256SUMS written for it, but is
    # not what the build makes: only the comparison with the build refuses it.
    variant consistent 'printf x >>01-a.tar.gz && python3 -B -c "import hashlib
for n in (\"01-a.tar.gz\", \"02-b.tar.gz\"):
    print(hashlib.sha256(open(n, \"rb\").read()).hexdigest() + \"  \" + n)" >SHA256SUMS'
    expect fail "a release consistent with itself that differs from the build" UNA_SITE_LEARN_RELEASE_DIR="$t/consistent"
    # The build and the release agree, but SHA256SUMS gives an archive another digest.
    cp -R "$t/build" "$t/build-good"
    sed 's/^[0-9a-f]*  01-a/0000000000000000000000000000000000000000000000000000000000000000  01-a/' "$t/build-good/SHA256SUMS" >"$t/build/SHA256SUMS"
    variant wrongsum "cp '$t/build/SHA256SUMS' SHA256SUMS"
    expect fail "a SHA256SUMS that does not give an archive its digest" UNA_SITE_LEARN_RELEASE_DIR="$t/wrongsum"
    rm -rf "${t:?}/build"
    mv "$t/build-good" "$t/build"
    lock 'release="r1"'
    expect pass "no spaces around =" UNA_SITE_LEARN_RELEASE_DIR="$t/gh-release"
    lock 'release  =  "r1"  '
    expect pass "more spaces, and trailing ones" UNA_SITE_LEARN_RELEASE_DIR="$t/gh-release"
    printf '%s\r\n' 'repository = "https://github.com/o/n.git"' 'release = "r1"' >"$t/lock"
    expect pass "CRLF lines" UNA_SITE_LEARN_RELEASE_DIR="$t/gh-release"
    printf 'repository = "https://github.com/o/n.git"\nrelease = "r1"\r\n' >"$t/lock"
    expect pass "a CRLF release line" UNA_SITE_LEARN_RELEASE_DIR="$t/gh-release"
    lock 'release = "r1"' 'release = "r2"'
    expect fail "two release lines" UNA_SITE_LEARN_RELEASE_DIR="$t/gh-release"
    lock ' release = "r1"'
    expect fail "a release line that does not parse (leading space)" UNA_SITE_LEARN_RELEASE_DIR="$t/gh-release"
    lock 'release = r1'
    expect fail "a release line that does not parse (no quotes)" UNA_SITE_LEARN_RELEASE_DIR="$t/gh-release"
    lock 'release = ""'
    expect fail "an empty release" UNA_SITE_LEARN_RELEASE_DIR="$t/gh-release"
    lock 'release = "../r1"'
    expect fail "a release that is not a plain tag name" UNA_SITE_LEARN_RELEASE_DIR="$t/gh-release"
    lock 'release = "-r1"'
    expect fail "a release that would read as an option" UNA_SITE_LEARN_RELEASE_DIR="$t/gh-release"
    lock 'release = "r2"'
    expect fail "a release gh cannot read" UNA_SITE_LEARN_RELEASE_DIR=
    # A lock whose first repository spans two lines, which the website's reading takes, and a
    # second one on one line, which a line-by-line reading takes: two repository lines fail.
    printf '%s\n' '# c' 'repository =' '"https://github.com/evil/x.git"' 'repository = "https://github.com/o/n.git"' 'commit = "0000000000000000000000000000000000000000"' 'release = "r1"' >"$t/lock"
    expect fail "a repository that reads one way to the website and another line by line" UNA_SITE_LEARN_RELEASE_DIR="$t/gh-release"
    grep -q 'repository lines' "$t/log" || { echo "  WRONG: the two repository lines are not what fails"; failures=$((failures + 1)); }
    lock
    expect pass "no release line" UNA_SITE_LEARN_RELEASE_DIR="$t/extra"
    grep -q 'no release named' "$t/log" || { echo "  WRONG: no release line is not reported"; failures=$((failures + 1)); }
    [ "$failures" -eq 0 ] || fail "self-test: $failures case(s) wrong"
    echo "site-learn-release: self-test PASS"
}

case "${1:-}" in
    --self-test)
        [ $# -eq 1 ] || fail "usage: scripts/site-learn-release.sh --self-test"
        self_test
        ;;
    --release-of)
        [ $# -eq 2 ] || fail "usage: scripts/site-learn-release.sh --release-of LOCK"
        release_of "$2"
        ;;
    *)
        [ $# -eq 3 ] || {
            echo "usage: scripts/site-learn-release.sh LOCK ARCHIVES OUT | --release-of LOCK | --self-test" >&2
            exit 2
        }
        compare "$1" "$2" "$3"
        ;;
esac
