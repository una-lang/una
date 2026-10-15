"""Family R: refactors that keep every label (docs/plan/security-hunt-generators.md §4 R).

Each takes a program's files and returns new files, or `None` when it does not apply. The
must-keep driver checks both and compares the verdicts. They work on the canonical layout of
`spec/15`, so they are line-based: a top-level declaration starts at column 0.
"""

import re

KEYWORDS = {
    "module", "trusted", "import", "pub", "kind", "policy", "from", "to", "via", "purpose",
    "owner", "capability", "op", "accepts", "handler", "for", "uses", "fails", "requires",
    "ensures", "where", "let", "if", "then", "else", "when", "is", "in", "none", "error",
    "use", "pass", "fail", "attempt", "with", "together", "fn", "true", "false", "unit", "it",
    "result", "main", "example", "expect", "do", "pred", "loop", "checked", "and", "or", "not",
}

DECL_NAME = re.compile(r"^(?:pub )?([a-z_][a-z0-9_]*)(?:\[[^\]]*\])?\(", re.M)
HEAD = re.compile(r"^(?:trusted module|module|import) ")


def _split_code(text):
    """Splits `text` into (is_code, piece) pairs, so that rewrites skip string literals and
    comments. The interpolations `{...}` inside a string are code: a rename must reach a call
    written there, or the twin refers to a name that no longer exists (U1001) and its verdict
    says nothing (from the review of #517). A text literal cannot nest in an interpolation
    (U0006, `spec/02`), so an interpolation ends at its matching `}`."""
    out, i, n = [], 0, len(text)
    start = 0
    while i < n:
        c = text[i]
        if c == '"':
            out.append((True, text[start:i]))
            j = i + 1
            seg = i
            while j < n and text[j] != '"':
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == "{":
                    out.append((False, text[seg:j + 1]))
                    depth, k = 1, j + 1
                    while k < n and depth:
                        depth += {"{": 1, "}": -1}.get(text[k], 0)
                        k += 1
                    out.append((True, text[j + 1:k - 1]))
                    seg = j = k - 1
                    continue
                j += 1
            out.append((False, text[seg:j + 1]))
            i = start = j + 1
            continue
        if text.startswith("--", i):
            out.append((True, text[start:i]))
            j = text.find("\n", i)
            j = n if j < 0 else j
            out.append((False, text[i:j]))
            i = start = j
            continue
        i += 1
    out.append((True, text[start:]))
    return out


def r6_rename(files, rng):
    """Renames every function a module declares, in every file, outside strings and
    comments, with a suffix no generator uses."""
    names = set()
    for rel, text in files.items():
        if rel.endswith(".una"):
            names.update(m.group(1) for m in DECL_NAME.finditer(text))
    names -= KEYWORDS
    if not names:
        return None
    suffix = "_r" + format(rng.randrange(1 << 16), "x")
    pat = re.compile(r"\b(" + "|".join(sorted(map(re.escape, names))) + r")\b")
    out = {}
    for rel, text in files.items():
        if not rel.endswith(".una"):
            out[rel] = text
            continue
        out[rel] = "".join(pat.sub(lambda m: m.group(1) + suffix, piece) if code else piece
                           for code, piece in _split_code(text))
    return out


def _blocks(text):
    """The module's header (comments, `module`, imports) and its top-level declarations."""
    lines = text.split("\n")
    head, i = [], 0
    while i < len(lines) and (not lines[i] or lines[i].startswith("--") or HEAD.match(lines[i])):
        head.append(lines[i])
        i += 1
    decls, cur = [], []
    for line in lines[i:]:
        starts = bool(line) and not line[0].isspace() and line != "="
        only_comments = all(not x or x.startswith("--") for x in cur)
        if starts and cur and not only_comments:
            decls.append(cur)
            cur = []
        cur.append(line)
    if cur:
        decls.append(cur)
    decls = ["\n".join(d).strip("\n") for d in decls if "\n".join(d).strip()]
    return "\n".join(head).rstrip("\n"), decls


def r9_reorder(files, rng):
    """Reorders the top-level declarations of one module with at least two."""
    candidates = []
    for rel, text in sorted(files.items()):
        if rel.endswith(".una"):
            head, decls = _blocks(text)
            if len(decls) >= 2:
                candidates.append((rel, head, decls))
    if not candidates:
        return None
    rel, head, decls = rng.choice(candidates)
    order = list(range(len(decls)))
    while order == list(range(len(decls))):
        rng.shuffle(order)
    out = dict(files)
    out[rel] = head + "\n\n" + "\n\n".join(decls[k] for k in order) + "\n"
    return out


REFACTORS = {"R6": r6_rename, "R9": r9_reorder}
