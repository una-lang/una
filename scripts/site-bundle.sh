#!/bin/sh
# Writes the docs bundle (#584): the documents the website renders, taken from this checkout as
# they are, with a manifest and the compiler's own help texts, so the website never copies the
# documentation by hand (una-lang/website, docs/site-map.md §4.2).
#
# OUT/una-docs-<version>/
#   manifest.json   schema una.site-bundle/1: the version, the commit, the editions, and every
#                   file with its kind, title, sha256, the `spec/NN §M` references it makes,
#                   and whether it is neutral
#   files/<path>    each document at its repository path
#   help/<cmd>.txt  `una --help` (una.txt) and `una <cmd> --help`, as the built binary prints them
#   version.txt     `una --version`
# OUT/una-docs-<version>.tar.gz and its .sha256
#
# Neutrality: public pages must be neutral (ADR 0088). The pattern, an extended regular
# expression (grep -E), comes on the file descriptor that UNA_NEUTRALITY_PATTERN_FD names, as
# scripts/release-gates.sh passes it, else from the environment variable UNA_NEUTRALITY_PATTERN,
# for a run by hand; never from a file. When there is one, each file is marked neutral or not,
# a pattern grep cannot apply fails the bundle, and the manifest says `"neutrality": "checked"`; otherwise every file
# is marked `null` and the manifest says `"unchecked"`. The website publishes only files it
# knows are neutral.
#
# It needs a built `una` of this checkout: UNA_BIN, else target/release/una, else
# target/debug/una. Its version must be the workspace's. When docs name a newer
# edition, build una from this commit. Python 3.11+ validates the catalog and its
# fixture manifests; the pinned Z3 must be on PATH for their positive checks.
#
# Usage: scripts/site-bundle.sh OUT
set -eu
# A trace would print the pattern: refuse it; drop the shells' startup files; no core file.
case $- in
    *x*)
        echo "site-bundle: refusing to run with xtrace (set -x), which would print the pattern" >&2
        exit 2
        ;;
esac
unset BASH_ENV ENV
ulimit -c 0

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

fail() {
    echo "site-bundle: FAIL ($1)" >&2
    exit 1
}

[ $# -eq 1 ] || {
    echo "usage: scripts/site-bundle.sh OUT" >&2
    exit 2
}
mkdir -p "$1"
out=$(CDPATH='' cd -- "$1" && pwd)

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
[ -n "$version" ] || fail "no workspace version in Cargo.toml"
una=${UNA_BIN:-}
if [ -z "$una" ]; then
    for candidate in target/release/una target/debug/una; do
        if [ -x "$candidate" ]; then
            una=$candidate
            break
        fi
    done
fi
[ -n "$una" ] && [ -x "$una" ] || fail "no una binary: build it, or set UNA_BIN"
found=$("$una" --version | head -n 1)
[ "$found" = "una $version" ] || fail "$una is $found, the workspace is una $version"
commit=$(git rev-parse HEAD)
[ -z "$(git status --porcelain --untracked-files=no)" ] || echo "site-bundle: warning: the checkout has uncommitted changes; the manifest names $commit" >&2

name=una-docs-$version
bundle=$out/$name
rm -rf "${bundle:?}" "${out:?}/$name.tar.gz" "${out:?}/$name.tar.gz.sha256"
mkdir -p "$bundle/files" "$bundle/help"

# --- The help texts, from the binary.
"$una" --version >"$bundle/version.txt"
editions=$(sed -n 's/^editions: //p' "$bundle/version.txt")
[ -n "$editions" ] || fail "una --version names no editions"
python3 -B scripts/site-features.py --editions "$editions" --una "$una" >"$bundle/.features" \
    || fail "feature evidence or document editions"
NO_COLOR=1 "$una" --help >"$bundle/help/una.txt"
commands=$(sed -n 's/^  \([a-z][a-z]*\)  .*/\1/p' "$bundle/help/una.txt" | grep -v '^help$')
[ -n "$commands" ] || fail "una --help lists no commands"
for command in $commands; do
    NO_COLOR=1 "$una" "$command" --help >"$bundle/help/$command.txt" || fail "una $command --help"
done

# --- The documents: the root documents, the guides, the specification, the ADRs, the release's
# validation summary (#717), the security advisories (#649), the explain pages, the examples and the
# standard library listing (docs/std/, #648), each with its kind. Only files that git tracks.
files=$(
    {
        for f in README.md SECURITY.md CONTRIBUTING.md CODE_OF_CONDUCT.md CHANGELOG.md docs/00-charter.md; do
            echo "root $f"
        done
        git ls-files 'docs/guide/*.md' | sed 's/^/guide /'
        git ls-files 'docs/spec/*.md' | sed 's/^/spec /'
        git ls-files 'docs/adr/*.md' | grep -v '/0000-template\.md$' | sed 's/^/adr /'
        git ls-files "docs/validation/$version.md" | sed 's/^/release /'
        git ls-files docs/security/advisories.md | sed 's/^/security /'
        git ls-files 'crates/una-explain/explain/U*.md' | sed 's/^/explain /'
        git ls-files examples | sed 's/^/example /'
        git ls-files 'docs/std/*' | sed 's/^/std /'
        git ls-files docs/features.json | sed 's/^/features /'
    } | awk '!seen[$2]++'
)

# A JSON string of stdin's single line.
json() {
    sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/	/\\t/g' | awk '{ printf "\"%s\"", $0 }'
}

# A name the caller exported would stay exported: unset first.
unset pattern
pattern=
if [ -n "${UNA_NEUTRALITY_PATTERN_FD:-}" ]; then
    case $UNA_NEUTRALITY_PATTERN_FD in
        3) pattern=$(cat <&3); exec 3<&- ;;
        *) fail "UNA_NEUTRALITY_PATTERN_FD must be 3" ;;
    esac
else
    pattern=${UNA_NEUTRALITY_PATTERN:-}
fi
# Held in this variable only: no command this script runs inherits it.
unset UNA_NEUTRALITY_PATTERN UNA_NEUTRALITY_PATTERN_FD
neutrality=unchecked
[ -n "$pattern" ] && neutrality=checked

entries=$bundle/.entries
: >"$entries"
count=0
not_neutral=0
echo "$files" | while read -r kind path; do
    [ -f "$path" ] || fail "$path is tracked but missing"
    mkdir -p "$bundle/files/$(dirname "$path")"
    cp "$path" "$bundle/files/$path"
    digest=$(shasum -a 256 "$path" | cut -d' ' -f1)
    title=null
    refs=""
    case $path in
        *.md)
            first=$(sed -n 's/^# //p' "$path" | head -n 1)
            [ -n "$first" ] && title=$(printf '%s\n' "$first" | json)
            refs=$(grep -o 'spec/[0-9][0-9]\( §[0-9][0-9.]*[0-9]\)\{0,1\}' "$path" | LC_ALL=C sort -u | while read -r ref; do printf '%s\n' "$ref" | json; printf ','; done | sed 's/,$//')
            ;;
    esac
    neutral=null
    if [ -n "$pattern" ]; then
        # The pattern goes to grep on its standard input (the shell's own printf), never as an
        # argument, which other processes could read. grep's own message is dropped, and a
        # status other than match (0) or no match (1) fails: a pattern grep cannot apply never
        # marks a file neutral.
        status=0
        printf '%s\n' "$pattern" | grep -Eiq -f - -- "$path" 2>/dev/null || status=$?
        case $status in
            0) neutral=false ;;
            1) neutral=true ;;
            *) fail "the neutrality pattern could not be applied (grep -E): it is not a valid extended regular expression, or $path cannot be read" ;;
        esac
    fi
    printf '{"path":%s,"kind":"%s","title":%s,"sha256":"%s","spec_refs":[%s],"neutral":%s}\n' \
        "$(printf '%s\n' "$path" | json)" "$kind" "$title" "$digest" "$refs" "$neutral" >>"$entries"
done

count=$(wc -l <"$entries" | tr -d ' ')
not_neutral=$(grep -c '"neutral":false' "$entries" || true)
{
    printf '{"schema":"una.site-bundle/1","version":"%s","commit":"%s","editions":[%s],"neutrality":"%s","commands":[%s],"feature_catalog":%s,"files":[\n' \
        "$version" "$commit" \
        "$(echo "$editions" | tr ',' '\n' | sed 's/^ *//' | while read -r e; do printf '"%s",' "$e"; done | sed 's/,$//')" \
        "$neutrality" \
        "$(for c in $commands; do printf '"%s",' "$c"; done | sed 's/,$//')" \
        "$(cat "$bundle/.features")"
    sed '$!s/$/,/' "$entries"
    printf ']}\n'
} >"$bundle/manifest.json"
rm "$entries" "$bundle/.features"

# --- The check: every listed file is in the bundle with its digest, and every command has its help.
grep -o '"path":"[^"]*","kind":"[a-z]*","title":[^,]*\(,[^,]*\)*,"sha256":"[0-9a-f]\{64\}"' "$bundle/manifest.json" >/dev/null \
    || fail "the manifest is malformed"
sed -n 's/^{"path":"\([^"]*\)".*"sha256":"\([0-9a-f]\{64\}\)".*/\2  files\/\1/p' "$bundle/manifest.json" >"$out/.check"
(cd "$bundle" && shasum -a 256 -c --quiet "$out/.check") || fail "a file of the bundle does not match its digest"
rm "$out/.check"
for command in $commands; do
    [ -s "$bundle/help/$command.txt" ] || fail "no help for una $command"
done

# --- The archive, the same bytes for the same commit: a fixed order, fixed times, owners and
# modes, the ustar format (no extended headers), and gzip without a name or time.
find "$bundle" -exec env TZ=UTC touch -h -t 200001010000 {} +
find "$bundle" -type d -exec chmod 755 {} +
find "$bundle" -type f -exec chmod 644 {} +
if tar --version 2>/dev/null | grep -q bsdtar; then
    owner="--uid 0 --gid 0 --uname root --gname root"
else
    owner="--owner=0 --group=0 --numeric-owner"
fi
# shellcheck disable=SC2086 # $owner holds several flags
(cd "$out" && find "$name" -type f | LC_ALL=C sort | tar --format ustar $owner -cf - -T - | gzip -n -9 >"$name.tar.gz")
(cd "$out" && shasum -a 256 "$name.tar.gz" >"$name.tar.gz.sha256")

echo "site-bundle: PASS ($count files, $(echo "$commands" | wc -w | tr -d ' ') commands, neutrality $neutrality${pattern:+, $not_neutral not neutral}; $(cut -d' ' -f1 "$out/$name.tar.gz.sha256")  $name.tar.gz)"
