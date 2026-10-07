#!/bin/sh
# Builds the Una website (#244, N-34) into target/site, from the repository's own documents, so
# the site never drifts from them:
# - the landing page is README.md without its table of contents;
# - the tutorial, the guides, the specification, the charter, the ADRs, the validation records
#   and the changelog are the files under docs/ and CHANGELOG.md, as they are;
# - the downloads page takes the archives and their digests from docs/releases/*-validation.md,
#   the records install.sh trusts, and the install steps from README.md; it links no binary
#   that the repository does not hold;
# - the indexes of the specification and of the ADRs, and the table of contents, are generated.
# A link to a page of the site stays a link to that page; any other link into the repository,
# such as an example package or a script, points to the file on GitHub, and must name a file
# or directory that git tracks. After the build, every
# page must exist and every link between pages, or to an image, must resolve, or the script
# fails. Hosting and the domain are the owner's decision: this script only builds.
#
# It needs mdBook at exactly the version below, a development tool, not a dependency of any
# crate:
#   cargo install mdbook --version 0.5.4 --locked
#
# Usage: scripts/site.sh
set -eu

MDBOOK_VERSION=0.5.4
REPOSITORY=https://github.com/una-lang/una

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

fail() {
    echo "site: FAIL ($1)" >&2
    exit 1
}

command -v mdbook >/dev/null 2>&1 \
    || fail "mdbook not found; install it with: cargo install mdbook --version $MDBOOK_VERSION --locked"
found=$(mdbook --version)
[ "$found" = "mdbook v$MDBOOK_VERSION" ] \
    || fail "found $found, need mdbook v$MDBOOK_VERSION: cargo install mdbook --version $MDBOOK_VERSION --locked"

src=target/site-src
out=target/site
rm -rf "${src:?}" "${out:?}"
mkdir -p "$src"

# The first heading of a Markdown file, for the table of contents.
title() {
    sed -n 's/^# //p' "$1" | head -n 1
}

# --- The pages taken as they are.
pages=$(
    {
        echo CHANGELOG.md
        echo docs/00-charter.md
        echo docs/README.md
        ls docs/guide/*.md docs/spec/*.md docs/releases/*.md
        ls docs/adr/*.md | grep -v '/0000-template\.md$'
    } | sort
)
for page in $pages; do
    mkdir -p "$src/$(dirname "$page")"
done

# --- The landing page: README.md without its table of contents.
awk '
    /^## Contents$/ { skip = 1; next }
    skip && /^## / { skip = 0 }
    !skip { print }
' README.md >"$src/README.md.in"

# --- The index of the specification.
{
    echo "# Specification"
    echo
    echo "The normative definition of Una. Its keywords MUST, MUST NOT, SHOULD and MAY follow RFC 2119. When documents disagree, the specification wins, then the accepted ADRs."
    echo
    for chapter in docs/spec/*.md; do
        echo "- [$(title "$chapter")]($(basename "$chapter"))"
    done
} >"$src/docs/spec/README.md.in"

# --- The index of the ADRs, with the status of each.
{
    echo "# Design decisions"
    echo
    echo "Architecture decision records: why each key decision about Una was made. An accepted record is not edited; a later change is appended as a dated amendment."
    echo
    echo "| Record | Status |"
    echo "| --- | --- |"
    for adr in docs/adr/*.md; do
        case $adr in */0000-template.md) continue ;; esac
        status=$(sed -n 's/^- Status: //p' "$adr" | head -n 1)
        echo "| [$(title "$adr")]($(basename "$adr")) | ${status:-} |"
    done
} >"$src/docs/adr/README.md.in"

# --- The downloads page: the install steps of README.md, then each validation record's archives.
{
    echo "# Downloads"
    echo
    awk '
        /^## Install$/ { keep = 1; next }
        keep && /^## / { keep = 0 }
        keep { print }
    ' README.md
    echo
    echo "## Archives and digests"
    echo
    echo "Each release's archives, with the SHA-256 digests its validation record lists. \`install.sh\` checks a download against the same record, and installs it only when the record's verdict is PASS. An archive can be downloaded from the [releases page]($REPOSITORY/releases) once its release is published."
    for record in $(ls docs/releases/*-validation.md | sort -r); do
        version=$(basename "$record" -validation.md)
        verdict=$(sed -n 's/^Verdict: //p' "$record" | head -n 1)
        echo
        echo "### $version"
        echo
        echo "Validation record: [$version]($record), verdict ${verdict:-missing}."
        echo
        echo "| Archive | SHA-256 |"
        echo "| --- | --- |"
        grep -E '^\| `una-[^`]+\.tar\.gz` \| `[0-9a-f]{64}` \|$' "$record"
    done
    echo
    echo "## Editor support"
    echo
    echo "The VS Code extension, with Una's grammar, the Graúna theme, file icons and the language server \`una lsp\`, ships on the Visual Studio Marketplace and Open VSX with the Beta ([#243]($REPOSITORY/issues/243))."
} >"$src/downloads.md.in"

# --- The table of contents.
{
    echo "# Summary"
    echo
    echo "[Una](README.md)"
    echo "[Downloads](downloads.md)"
    echo
    echo "# Learn"
    echo
    echo "- [Getting started](docs/guide/getting-started.md)"
    echo "- [Una for agents](docs/guide/una-for-agents.md)"
    echo "- [Language guide](docs/guide/syntax-reference.md)"
    for guide in docs/guide/*.md; do
        case $guide in
            */getting-started.md | */una-for-agents.md | */syntax-reference.md) ;;
            *) echo "- [$(title "$guide")]($guide)" ;;
        esac
    done
    echo
    echo "# Reference"
    echo
    echo "- [Specification](docs/spec/README.md)"
    for chapter in docs/spec/*.md; do
        echo "  - [$(title "$chapter")]($chapter)"
    done
    echo "- [Charter](docs/00-charter.md)"
    echo "- [Design decisions](docs/adr/README.md)"
    for adr in docs/adr/*.md; do
        case $adr in */0000-template.md) continue ;; esac
        echo "  - [$(title "$adr")]($adr)"
    done
    echo "- [Documentation map](docs/README.md)"
    echo
    echo "# Releases"
    echo
    echo "- [Validation](docs/releases/validation.md)"
    for record in $(ls docs/releases/*-validation.md | sort -r); do
        echo "  - [$(basename "$record" -validation.md)]($record)"
    done
    echo "- [Changelog](CHANGELOG.md)"
} >"$src/SUMMARY.md"

for page in $pages; do
    cp "$page" "$src/$page.in"
done

# --- Links. Every page is staged at its own repository path, so a relative link between pages
# stays as written, except that a link to a README.md goes to the index.html that mdBook builds
# from it. A link to a directory with a generated index goes to that index; a link to
# any other file or directory of the repository goes to it on GitHub. Links inside code blocks
# and inline code are left alone.
staged=$(cd "$src" && find . -name '*.md.in' | sed -e 's|^\./||' -e 's|\.in$||' | sort)
images=$(sed -n 's/.*src="\(images\/[^"]*\)".*/\1/p' README.md | sort -u)
for image in $images; do
    mkdir -p "$src/$(dirname "$image")"
    cp "$image" "$src/$image"
done
{
    printf '%s\n' $staged $images | sed 's/^/page /'
    git ls-files | sed 's/^/file /'
    git ls-files | awk -F/ '{ path = ""; for (i = 1; i < NF; i++) { path = path (i > 1 ? "/" : "") $i; print "dir " path } }' | sort -u
} >"$src/.paths"

for page in $staged; do
    awk -v page="$page" -v repository="$REPOSITORY" -v paths="$src/.paths" -v missing="$src/.missing" '
        BEGIN {
            while ((getline line < paths) > 0) {
                split(line, f, " ")
                kind[f[2]] = (f[2] in kind && kind[f[2]] == "page") ? "page" : f[1]
            }
            n = split(page, parts, "/")
            base = ""
            for (i = 1; i < n; i++) base = base (i > 1 ? "/" : "") parts[i]
        }
        # The repository path of `target`, relative to the page, with `.` and `..` resolved.
        function resolve(target,    joined, segs, count, out, i, k) {
            joined = (base == "" ? target : base "/" target)
            count = split(joined, segs, "/")
            k = 0
            for (i = 1; i <= count; i++) {
                if (segs[i] == "" || segs[i] == ".") continue
                if (segs[i] == "..") { if (k > 0) k--; continue }
                out[++k] = segs[i]
            }
            joined = ""
            for (i = 1; i <= k; i++) joined = joined (i > 1 ? "/" : "") out[i]
            return joined
        }
        function rewrite(target,    path, frag, at, resolved) {
            if (target ~ /^[a-z]+:/ || target ~ /^#/ || target == "") return target
            frag = ""
            at = index(target, "#")
            if (at > 0) { frag = substr(target, at); path = substr(target, 1, at - 1) } else path = target
            resolved = resolve(path)
            # mdBook builds README.md as index.html, but does not rewrite links to it.
            if (kind[resolved] == "page") {
                if (resolved ~ /(^|\/)README\.md$/) sub(/README\.md$/, "index.md", path)
                return path frag
            }
            if (kind[resolved "/README.md"] == "page") {
                sub(/\/$/, "", path)
                return path "/index.md" frag
            }
            if (kind[resolved] == "dir") return repository "/tree/main/" resolved frag
            if (kind[resolved] != "file") print page ": " target >> missing
            return repository "/blob/main/" resolved frag
        }
        /^[[:space:]]*```/ { fence = !fence; print; next }
        fence { print; next }
        # Rewrites the links of a stretch of text outside inline code.
        function links(line,    result, token, target, at) {
            result = ""
            while (match(line, /\]\([^)[:space:]]+\)|(src|href)="[^"]+"/)) {
                token = substr(line, RSTART, RLENGTH)
                if (substr(token, 1, 2) == "](") {
                    target = substr(token, 3, length(token) - 3)
                    token = "](" rewrite(target) ")"
                } else {
                    at = index(token, "\"")
                    target = substr(token, at + 1, length(token) - at - 1)
                    token = substr(token, 1, at) rewrite(target) "\""
                }
                result = result substr(line, 1, RSTART - 1) token
                line = substr(line, RSTART + RLENGTH)
            }
            return result line
        }
        # Inline code spans, between backticks, are left alone.
        {
            count = split($0, spans, "`")
            out = ""
            for (i = 1; i <= count; i++) {
                out = out (i > 1 ? "`" : "") (i % 2 == 1 ? links(spans[i]) : spans[i])
            }
            print out
        }
    ' "$src/$page.in" >"$src/$page"
    rm "$src/$page.in"
done
rm "$src/.paths"
# A link into the repository must name a file or directory that git tracks (#305): an untracked
# file, such as a local build, does not exist on GitHub.
if [ -s "$src/.missing" ]; then
    cat "$src/.missing" >&2
    fail "$(wc -l <"$src/.missing" | tr -d ' ') links name no tracked file"
fi

# --- The build.
mdbook build site >"$src/.build.log" 2>&1 || {
    cat "$src/.build.log" >&2
    fail "mdbook build failed"
}
if grep -q 'WARN\|ERROR' "$src/.build.log"; then
    cat "$src/.build.log" >&2
    fail "mdbook reported warnings"
fi

# --- The check: every page was built, and every link between pages, or to an image, resolves.
for page in $staged; do
    case $page in
        */README.md) html=${page%README.md}index.html ;;
        README.md) html=index.html ;;
        *) html=${page%.md}.html ;;
    esac
    [ -f "$out/$html" ] || fail "the page $page was not built as $html"
done
broken=$(cd "$out" && find . -name '*.html' | sort | while read -r file; do
    dir=$(dirname "$file")
    grep -o -E '(href|src)="[^"#?]*' "$file" | sed -E 's/^(href|src)="//' | sort -u | while read -r target; do
        case $target in
            ('' | *:* | //*) continue ;;
        esac
        [ -e "$dir/$target" ] || echo "$file -> $target"
    done
done)
if [ -n "$broken" ]; then
    echo "$broken" >&2
    fail "$(echo "$broken" | wc -l | tr -d ' ') links do not resolve"
fi

count=$(echo "$staged" | wc -l | tr -d ' ')
echo "site: PASS ($count pages in $out)"
