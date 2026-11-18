#!/usr/bin/env python3
"""Checks the links of the repository's documents (#708), where scripts/site.sh checked them
with mdBook before the website replaced its book.

The documents are the tracked README.md, SECURITY.md, CONTRIBUTING.md, CODE_OF_CONDUCT.md,
CHANGELOG.md, the charter, the documentation map, the guides, the specification, the release
records, the ADRs and the security advisories: the set that scripts/site.sh staged, and the
documents that the website publishes besides. In each,
outside code blocks and inline code, every link that is not a URL, inline or in a reference
definition `[ref]: target` (#719):
- names a file or directory that git tracks (#305), so it exists on GitHub; an image included;
- with a `#fragment`, names a heading of its target document, or of the document itself, when
  the target is one of these documents;
- or, when it starts with `/`, is a page of the website's Learn section: `/learn/` and lowercase
  route segments, each ending in `/`, with an optional plain `#anchor` (#859). The website build
  checks that the route exists (its learn-pages.json); here only its form is checked.
A percent-encoded path or fragment is decoded first, and a malformed escape is refused.

A heading's fragment is the id the website gives it (una-lang/website,
scripts/preview/markdown.mjs, the "anchors" rule): the words of the heading, lowercased, without
backticks or any character but a letter, a digit, a space or a hyphen, with spaces made hyphens,
and `-1` added while the id is taken; in a specification chapter, a numbered heading `N.M.` is
`sN-M`. scripts/site-website.sh compares these ids with the ones the pinned website renders,
for every document here and for scripts/doc-links/anchors.md.

Usage:
  scripts/doc-links.py              check the documents of this checkout
  scripts/doc-links.py --self-test  check that each kind of broken link is refused
  scripts/doc-links.py --anchors    print every document's heading ids, as JSON, for the
                                    comparison with the website
"""

import html
import json
import os
import re
import subprocess
import sys
import tempfile
import unicodedata
from urllib.parse import unquote

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
FIXTURE = "scripts/doc-links/anchors.md"


ROOT_DOCUMENTS = [
    "README.md",
    "SECURITY.md",
    "CONTRIBUTING.md",
    "CODE_OF_CONDUCT.md",
    "CHANGELOG.md",
    "docs/00-charter.md",
    "docs/README.md",
    "docs/release-gates.md",
]
DIRECTORIES = ["docs/guide", "docs/spec", "docs/releases", "docs/validation", "docs/adr", "docs/security"]


def documents(files):
    """The documents whose links are checked, among the tracked `files`, sorted."""
    docs = [d for d in ROOT_DOCUMENTS if d in files]
    for f in files:
        directory, _, name = f.rpartition("/")
        if directory in DIRECTORIES and name.endswith(".md") and f != "docs/adr/0000-template.md":
            docs.append(f)
    return sorted(docs)


def tracked(root):
    """The files that git tracks, and the directories that hold them."""
    out = subprocess.run(
        ["git", "ls-files", "-z"], cwd=root, check=True, capture_output=True
    ).stdout.decode()
    files = set(f for f in out.split("\0") if f)
    dirs = set()
    for f in files:
        parts = f.split("/")
        for i in range(1, len(parts)):
            dirs.add("/".join(parts[:i]))
    return files, dirs


# The block structure the website's Markdown gives a document, as far as headings and code
# need it (#843): block quotes and list items, nested both ways, fenced code (backticks or tildes)
# and indented code, ATX headings, thematic breaks and paragraphs, with lazy continuation. Tabs
# count to the next multiple of 4 columns. HTML blocks and setext headings are not read; the
# website's own flow (scripts/site-website.sh) compares every document's heading ids with the
# website's, so a gap shows there.
QUOTE = re.compile(r"^ {0,3}> ?")
LIST = re.compile(r"^( {0,3})([-+*]|(\d{1,9})[.)])( {1,5}|$)")
OPENER = re.compile(r"^ {0,3}(`{3,}|~{3,})(.*)$")
BREAK = re.compile(r"^ {0,3}([-*_])(?:[ ]*\1){2,}[ ]*$")


def blocks(text):
    """Each line of `text` with its number and what it is: ("heading", number, the heading's
    text after its markers), ("code", number, line) or ("text", number, line), the line as
    written."""
    stack = []  # open containers: ("quote",) or ("list", content indent)
    fence = None  # (character, length, number of containers it is in)
    para = False
    for number, raw in enumerate(text.split("\n"), 1):
        line = raw.expandtabs(4)
        rest = line
        matched = 0
        for kind in stack:
            if kind[0] == "quote":
                m = QUOTE.match(rest)
                if not m:
                    break
                rest = rest[m.end():]
            else:
                if rest.strip() and len(rest) - len(rest.lstrip(" ")) < kind[1]:
                    break
                rest = rest[kind[1]:] if rest.strip() else ""
            matched += 1
        if fence is not None:
            if matched < fence[2]:
                # The fence's container ended, and the fence with it.
                fence = None
            else:
                m = re.match(r"^ {0,3}(" + re.escape(fence[0]) + "{" + str(fence[1]) + r",})[ ]*$", rest)
                if m:
                    fence = None
                yield "code", number, raw
                continue
        if matched < len(stack):
            lazy = para and rest.strip() and not (
                QUOTE.match(rest) or LIST.match(rest) or OPENER.match(rest) or BREAK.match(rest)
                or HEADING.match(rest))
            if lazy:
                yield "text", number, raw
                continue
            del stack[matched:]
            para = False
        # New containers.
        while True:
            m = QUOTE.match(rest)
            if m:
                stack.append(("quote",))
                rest = rest[m.end():]
                para = False
                continue
            m = LIST.match(rest)
            if m and not BREAK.match(rest):
                indent, marker, digits, spaces = m.groups()
                empty = rest[m.end():].strip() == "" and spaces != ""
                if para and ((digits is not None and int(digits) != 1) or not rest[m.end():].strip()):
                    break  # it cannot interrupt a paragraph
                width = len(indent) + len(marker)
                if spaces == "" or len(spaces) == 5 or empty:
                    width += 1  # code after the marker, or nothing: content one column on
                    rest = rest[m.end() - len(spaces) + (1 if spaces else 0):]
                else:
                    width += len(spaces)
                    rest = rest[m.end():]
                stack.append(("list", width))
                para = False
                continue
            break
        if not rest.strip():
            para = False
            yield "text", number, raw
            continue
        if not para and len(rest) - len(rest.lstrip(" ")) >= 4:
            yield "code", number, raw
            continue
        m = OPENER.match(rest)
        if m and not (m.group(1)[0] == "`" and "`" in m.group(2)):
            fence = (m.group(1)[0], len(m.group(1)), len(stack))
            para = False
            yield "code", number, raw
            continue
        m = HEADING.match(rest)
        if m:
            para = False
            yield "heading", number, m.group(2) or ""
            continue
        if BREAK.match(rest):
            para = False
            yield "text", number, raw
            continue
        para = True
        yield "text", number, raw


def prose(text):
    """Each line outside code blocks, with its number, as (number, line, [stretches outside
    inline code])."""
    for kind, number, line in blocks(text):
        if kind == "code":
            continue
        yield number, line, line.split("`")[0::2]


def slug(text):
    """The website's `slug`: lowercased, backticks and every character but a letter, a digit,
    a space or a hyphen dropped, trimmed, and runs of spaces made one hyphen."""
    text = text.lower().replace("`", "")
    kept = "".join(
        c
        for c in text
        if unicodedata.category(c)[0] in "LN" or c.isspace() or c == "-"
    )
    return re.sub(r"\s+", "-", kept.strip())


HEADING = re.compile(r"^ {0,3}(#{1,6})(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*$")
LINK = re.compile(r"!?\[([^\]]*)\]\([^)]*\)")


def heading_text(raw):
    """The text of a heading as the website reads it: its text and inline code, without the
    marks of emphasis, links, escapes and entities."""
    parts = raw.split("`")
    out = []
    for i, part in enumerate(parts):
        if i % 2 == 1 and i < len(parts) - 1:
            out.append(part.strip() if part.strip() else part)
            continue
        if i % 2 == 1:
            part = "`" + part
        part = LINK.sub(r"\1", part)
        part = re.sub(r"\\([!-/:-@\[-`{-~])", r"\1", part)
        part = re.sub(r"(\*+|(?<!\w)_+|_+(?!\w))", "", part)
        out.append(html.unescape(part))
    return "".join(out)


SPEC_NUMBER = re.compile(r"^(\d+(?:\.\d+)*)\.?\s")


def anchors(text, spec):
    """The ids of the headings of a document, in order."""
    used = set()
    ids = []
    for kind, _, heading in blocks(text):
        if kind != "heading":
            continue
        words = heading_text(heading)
        numbered = SPEC_NUMBER.match(words)
        name = "s" + numbered.group(1).replace(".", "-") if spec and numbered else slug(words)
        while name in used:
            name += "-1"
        used.add(name)
        ids.append(name)
    return ids


def is_spec(path):
    return path.startswith("docs/spec/")


TARGET = re.compile(r"\]\(([^)\s]+)\)|(?:src|href)=\"([^\"]+)\"")
# A reference definition, `[ref]: target` or `[ref]: <target>`, with an optional title.
REFERENCE = re.compile(
    r"^ {0,3}\[[^\]]+\]:[ \t]*(?:<([^>\s]*)>|(\S+))"
    r"(?:[ \t]+(?:\"[^\"]*\"|'[^']*'|\([^)]*\)))?[ \t]*$"
)
MALFORMED_ESCAPE = re.compile(r"%(?![0-9A-Fa-f]{2})")
# A link to a page of the website's Learn section (#859).
LEARN = re.compile(r"/learn/(?:[a-z0-9]+(?:-[a-z0-9]+)*/)*(?:#[a-z0-9]+(?:-[a-z0-9]+)*)?")


def resolve(page, target):
    """The repository path that `target`, written in `page`, names, or None outside it."""
    segments = []
    for seg in (os.path.dirname(page) + "/" + target).split("/"):
        if seg in ("", "."):
            continue
        if seg == "..":
            if not segments:
                return None
            segments.pop()
            continue
        segments.append(seg)
    return "/".join(segments)


def problems(root, docs, files, dirs):
    """Every broken link of `docs`, as `path:line: target: reason`."""
    texts = {d: open(os.path.join(root, d), encoding="utf-8").read() for d in docs}
    ids = {d: set(anchors(t, is_spec(d))) for d, t in texts.items()}
    found = []

    def target_problem(page, target):
        if re.match(r"^[a-z]+:", target) or target.startswith("//"):
            return None
        if target.startswith("/"):
            if LEARN.fullmatch(target):
                return None
            return "is a root-relative link that is not a /learn/ route with an optional anchor"
        if MALFORMED_ESCAPE.search(target):
            return "has a malformed percent escape"
        path, _, fragment = target.partition("#")
        path, fragment = unquote(path, errors="strict"), unquote(fragment, errors="strict")
        if path:
            resolved = resolve(page, path)
            if resolved is None or (resolved not in files and resolved not in dirs):
                return "names no tracked file or directory"
        else:
            resolved = page
        if fragment and resolved in ids and fragment not in ids[resolved]:
            return f"{resolved} has no heading with the id {fragment}"
        return None

    for page, text in texts.items():
        for number, line, stretches in prose(text):
            targets = [m.group(1) or m.group(2) for s in stretches for m in TARGET.finditer(s)]
            definition = REFERENCE.match(line)
            if definition:
                targets.append(definition.group(1) or definition.group(2))
            for target in targets:
                try:
                    problem = target_problem(page, target)
                except UnicodeDecodeError:
                    problem = "has a percent escape that is not UTF-8"
                if problem:
                    found.append(f"{page}:{number}: {target}: {problem}")
    return found


def check(root):
    files, dirs = tracked(root)
    docs = documents(files)
    found = problems(root, docs, files, dirs)
    if found:
        print("\n".join(found), file=sys.stderr)
        print(f"doc-links: FAIL ({len(found)} broken links)", file=sys.stderr)
        return 1
    print(f"doc-links: PASS ({len(docs)} documents)")
    return 0


def self_test():
    """Each kind of broken link is refused, and the fixture's ids are the expected ones."""
    failures = []
    good = (
        "# Title\n\nSee [b](b.md#part-two), [self](#title), [dir](sub), [img](sub/i.png),\n"
        '<img src="sub/i.png">, [web](https://example.org/x#y) and `[code](nowhere.md)`.\n\n'
        "```\n[fenced](nowhere.md)\n[def]: nowhere.md\n```\n\n"
        "See [ref][] and [space](sub/a%20b.png).\n\n"
        "[ref]: b.md#part-two\n"
        '[angle]: <sub/i.png> "A title"\n'
        "[web]: https://example.org/x\n"
        "[encoded]: b.md#part%2Dtwo\n"
        "Learn more: [uses](/learn/capabilities/declaring-what-a-function-may-do-uses/), "
        "[a part](/learn/failure/handling-a-failure-the-five-clauses/#s6-2) and [Learn](/learn/).\n"
    )
    bad = {
        "a repository link to an untracked file": "[x](missing.md)",
        "a relative link out of the repository": "[x](../../outside.md)",
        "a fragment that names no heading": "[x](b.md#part-three)",
        "a fragment of the page itself that names no heading": "[x](#nothing)",
        "an image that is not tracked": '<img src="sub/missing.png">',
        "a reference definition to an untracked file": "[r]: missing.md",
        "a reference definition in angle brackets to an untracked file": "[r]: <missing.md>",
        "a reference definition with a title to an untracked file": '[r]: missing.md "Title"',
        "a reference definition whose fragment names no heading": "[r]: b.md#part-three",
        "a percent-encoded path to an untracked file": "[x](sub/i%2Emissing)",
        "a percent-encoded fragment that names no heading": "[x](b.md#part%2Dthree)",
        "a malformed percent escape": "[x](sub/i%zz.png)",
        "a percent escape that is not UTF-8": "[x](sub/%FF.png)",
        "a root-relative link outside /learn/": "[x](/docs/spec/)",
        "a /learn/ link that climbs out": "[x](/learn/../x/)",
        "a /learn/ link with an empty segment": "[x](/learn//x/)",
        "a /learn/ link with an uppercase route": "[x](/learn/Capabilities/)",
        "a /learn/ link without its trailing slash": "[x](/learn/capabilities)",
        "a /learn/ link with a query": "[x](/learn/capabilities/?x=1)",
        "a /learn/ link with markup in its anchor": "[x](/learn/capabilities/#<b>)",
    }
    # `i%zz.png` is tracked, so the malformed escape that names it is refused for the escape alone.
    files = {"docs/a.md", "docs/b.md", "docs/sub/i.png", "docs/sub/a b.png", "docs/sub/i%zz.png"}
    dirs = {"docs", "docs/sub"}
    with tempfile.TemporaryDirectory() as tmp:
        os.makedirs(os.path.join(tmp, "docs"))
        with open(os.path.join(tmp, "docs/b.md"), "w", encoding="utf-8") as f:
            f.write("# B\n\n## Part two\n")

        def run(body):
            with open(os.path.join(tmp, "docs/a.md"), "w", encoding="utf-8") as f:
                f.write(body)
            return problems(tmp, ["docs/a.md", "docs/b.md"], files, dirs)

        found = run(good)
        if found:
            failures.append(f"a page with good links was refused: {found}")
        for name, link in bad.items():
            if not run(good + "\n" + link + "\n"):
                failures.append(f"accepted {name}: {link}")

    fixture = open(os.path.join(ROOT, FIXTURE), encoding="utf-8").read()
    expected = [
        line.split(" ", 2)[2]
        for line in fixture.split("\n")
        if line.startswith("<!-- id ")
    ]
    expected = [e.removesuffix(" -->") for e in expected]
    got = anchors(fixture, False)
    if got != expected:
        failures.append(f"the fixture's ids are {got}, expected {expected}")
    if failures:
        print("\n".join(failures), file=sys.stderr)
        print(f"doc-links: self-test FAIL ({len(failures)})", file=sys.stderr)
        return 1
    print(f"doc-links: self-test PASS ({len(bad)} broken links refused, {len(got)} fixture ids)")
    return 0


def main(args):
    if args == ["--self-test"]:
        return self_test()
    if args == ["--anchors"]:
        docs = documents(tracked(ROOT)[0]) + [FIXTURE]
        out = {}
        for d in docs:
            text = open(os.path.join(ROOT, d), encoding="utf-8").read()
            out[d] = {"spec": is_spec(d), "ids": anchors(text, is_spec(d))}
        print(json.dumps(out, ensure_ascii=False, indent=1))
        return 0
    if args:
        print(__doc__, file=sys.stderr)
        return 2
    return check(ROOT)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
