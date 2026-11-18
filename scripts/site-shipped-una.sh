#!/bin/sh
# The `una` that the release archive of this host ships (#2029), to give scripts/site-website.sh
# as UNA_BIN in the website's own flow (#2081); crates/una-cli/tests/release_gates.rs runs its
# self-test.
#
# The book, Una by Example, and the code samples of the website are written for the `una` that a
# release ships: its report names the Z3 library that supply-chain/z3.lock records for the target
# (`solver build: ..., library <sha>`) and the runtime that is embedded in it (`runtime sha256:`).
# A `una` built by `cargo build --release` in the clone uses the machine's own Z3 library and
# embeds no runtime, so its reports differ and the book does not check with it. The book and
# the samples are therefore checked with the `una` of the archive that the release gate
# built in this clone, the one scripts/toolchain-free.sh runs too: the archive is checked against
# its .sha256 file, unpacked outside the clone, and its `una --version` must name the version.
# An archive that is absent, one whose digest differs, a binary that is absent or says another
# version fail; nothing falls back to target/release/una.
#
# Usage: scripts/site-shipped-una.sh VERSION BUILT OUT
#          VERSION  the workspace version
#          BUILT    the directory of the release gate's archives (target/local-release/v<version>)
#          OUT      a directory for the unpacked archive
#        prints the path of the unpacked `una`; what it chose goes to stderr.
#        scripts/site-shipped-una.sh --self-test
set -eu

fail() {
    echo "site-shipped-una: FAIL ($1)" >&2
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

shipped() {
    version=$1 built=$2 out=$3
    target=$(host_target)
    name=una-$version-$target
    [ -f "$built/$name.tar.gz" ] && [ -f "$built/$name.tar.gz.sha256" ] \
        || fail "no $built/$name.tar.gz with its .sha256: the book is checked with the una that the release archive ships, so run the release gate first"
    want=$(cut -d ' ' -f 1 "$built/$name.tar.gz.sha256")
    found=$(sha256_of "$built/$name.tar.gz")
    [ -n "$want" ] && [ "$found" = "$want" ] \
        || fail "$name.tar.gz has SHA-256 $found, and $name.tar.gz.sha256 gives ${want:-nothing}"
    rm -rf "${out:?}"
    mkdir -p "$out"
    tar -xzf "$built/$name.tar.gz" -C "$out" || fail "$name.tar.gz does not unpack"
    una=$out/$name/una
    [ -x "$una" ] || fail "$name.tar.gz holds no $name/una"
    said=$("$una" --version | head -n 1)
    [ "$said" = "una $version" ] || fail "the una of $name.tar.gz says \"$said\", not \"una $version\""
    echo "site-shipped-una: the una of $name.tar.gz, SHA-256 $found" >&2
    echo "$una"
}

self_test() {
    t=$(mktemp -d "${TMPDIR:-/tmp}/site-shipped-una.XXXXXX")
    trap 'rm -rf "${t:?}"' EXIT
    target=$(host_target)
    failures=0
    name=una-9.8.7-$target
    # A development una and a shipped una, which report different solver libraries, and a
    # book whose expected report is the shipped una's.
    printf '#!/bin/sh\n[ "$1" = --version ] && { echo "una 9.8.7"; exit 0; }\necho "  solver build: library 621d0c73"\n' >"$t/dev-una"
    mkdir -p "$t/pack/$name" "$t/built"
    printf '#!/bin/sh\n[ "$1" = --version ] && { echo "una 9.8.7"; exit 0; }\necho "  solver build: library 4fcda925"\necho "  runtime sha256: 036e2df8"\n' >"$t/pack/$name/una"
    chmod 755 "$t/dev-una" "$t/pack/$name/una"
    tar -czf "$t/built/$name.tar.gz" -C "$t/pack" "$name"
    (cd "$t/built" && printf '%s  %s\n' "$(sha256_of "$name.tar.gz")" "$name.tar.gz" >"$name.tar.gz.sha256")
    printf '  solver build: library 4fcda925\n  runtime sha256: 036e2df8\n' >"$t/book-report"
    # The book's check: a report is compared with the book's expected one.
    book_checks() { "$1" check | cmp -s - "$t/book-report"; }
    in_subshell() { (shipped "$@"); }
    expect() { # pass|fail WHAT, then the command
        want=$1 what=$2
        shift 2
        if "$@" >"$t/path" 2>"$t/log"; then got=pass; else got=fail; fi
        if [ "$got" = "$want" ]; then
            echo "  ok: $what ($want)"
        else
            echo "  WRONG: $what: expected $want, got $got"
            sed 's/^/    /' "$t/log"
            failures=$((failures + 1))
        fi
    }
    expect fail "a book report with the shipped library, checked with the build's una" book_checks "$t/dev-una"
    expect pass "the shipped una" in_subshell 9.8.7 "$t/built" "$t/out"
    [ "$(cat "$t/path")" = "$t/out/$name/una" ] || { echo "  WRONG: the path is not the unpacked una"; failures=$((failures + 1)); }
    expect pass "a book report with the shipped library, checked with the shipped una" book_checks "$t/out/$name/una"
    expect fail "no archive" in_subshell 9.8.7 "$t/empty" "$t/out2"
    mv "$t/built/$name.tar.gz.sha256" "$t/sha"
    expect fail "an archive with no .sha256 file" in_subshell 9.8.7 "$t/built" "$t/out2"
    printf '%s  x\n' 0000000000000000000000000000000000000000000000000000000000000000 >"$t/built/$name.tar.gz.sha256"
    expect fail "a digest that differs" in_subshell 9.8.7 "$t/built" "$t/out2"
    mv "$t/sha" "$t/built/$name.tar.gz.sha256"
    expect fail "a binary of another version" in_subshell 9.9.0 "$t/built" "$t/out2"
    mkdir -p "$t/pack2/$name"
    : >"$t/pack2/$name/other"
    tar -czf "$t/built/$name.tar.gz" -C "$t/pack2" "$name"
    (cd "$t/built" && printf '%s  %s\n' "$(sha256_of "$name.tar.gz")" "$name.tar.gz" >"$name.tar.gz.sha256")
    expect fail "an archive with no una" in_subshell 9.8.7 "$t/built" "$t/out2"
    [ "$failures" -eq 0 ] || fail "self-test: $failures case(s) wrong"
    echo "site-shipped-una: self-test PASS"
}

case "${1:-}" in
    --self-test)
        [ $# -eq 1 ] || fail "usage: scripts/site-shipped-una.sh --self-test"
        self_test
        ;;
    *)
        [ $# -eq 3 ] || {
            echo "usage: scripts/site-shipped-una.sh VERSION BUILT OUT | --self-test" >&2
            exit 2
        }
        shipped "$1" "$2" "$3"
        ;;
esac
