"""Builders for Una packages, in the canonical layout of `spec/15`.

The generators assemble programs from these pieces, so that each one parses by
construction. Varied literals are written through `Varied`, which records where each one
lands, for the two-run oracle (docs/plan/security-hunt-oracle.md §2).
"""

import json

from .program import LOCK

# Labels a policy's `from` and `to` take, lowest first.
LEVELS = ["public", "internal", "confidential", "secret", "restricted"]


def label_prefix(level, cats=()):
    """The label written before a type: `secret[PII] `, `internal `, or nothing for public."""
    if level == "public" and not cats:
        return ""
    cat = f"[{', '.join(cats)}]" if cats else ""
    return f"{level}{cat} "


def module_file(module):
    return "src/" + module.replace(".", "/") + ".una"


def toml(name, edition, categories=(), trusted=None, entry=None, grants=()):
    """`una.toml`. `trusted` is the raw list of module names written under `[trusted]`,
    so that operator T3 can write any spelling; `None` leaves the section out."""
    lines = ["[package]", f'name = "{name}"', 'version = "0.1.0"', f'edition = "{edition}"']
    if categories:
        lines += ["", "[labels]", "categories = " + json.dumps(list(categories))]
    if trusted is not None:
        lines += ["", "[trusted]", "modules = " + json.dumps(list(trusted), ensure_ascii=False)]
    if entry is not None:
        lines += ["", "[run]", f'entry = "{entry}"']
        if grants:
            lines.append("grants = " + json.dumps(list(grants)))
    return "\n".join(lines) + "\n"


def policy(name, frm, to, via, purpose="hunt", owner="hunt", pub=True):
    head = "pub policy" if pub else "policy"
    return (
        f"{head} {name}\n  from {frm}\n  to {to}\n  via {via}\n"
        f'  purpose "{purpose}"\n  owner {owner}\n'
    )


def header(module, trusted=False, imports=()):
    lines = [("trusted module " if trusted else "module ") + module]
    if imports:
        lines.append("")
        lines += [f"import {m} ({', '.join(names)})" for m, names in imports]
    return "\n".join(lines) + "\n"


def join_decls(*decls):
    """One module's text: its header and declarations, separated by blank lines."""
    return "\n".join(d.rstrip("\n") + "\n" for d in decls if d)


class Varied:
    """Collects the literals the oracle may vary, and where they land in each file.

    A generator writes `v.lit("Text", "secret[PII]", '"abc"')` where a literal goes and
    gets back a marker; `place(files)` swaps every marker for its literal and returns the
    positions: file, 1-based line and column, the literal's text, its type and label.
    """

    def __init__(self):
        self.items = []

    def lit(self, ty, label, text):
        marker = f"⟦V{len(self.items)}⟧"
        self.items.append({"marker": marker, "type": ty, "label": label, "literal": text})
        return marker

    def place(self, files):
        out = {}
        positions = []
        for rel, text in files.items():
            for item in self.items:
                while item["marker"] in text:
                    i = text.index(item["marker"])
                    line = text.count("\n", 0, i) + 1
                    col = i - (text.rfind("\n", 0, i) + 1) + 1
                    positions.append(
                        {"file": rel, "line": line, "col": col, "literal": item["literal"],
                         "type": item["type"], "label": item["label"]}
                    )
                    text = text[:i] + item["literal"] + text[i + len(item["marker"]):]
            out[rel] = text
        return out, positions


def package(name, edition, modules, categories=(), trusted=None, entry=None, grants=()):
    """The files of a package: `una.toml`, `verification.lock` and each module."""
    files = {
        "una.toml": toml(name, edition, categories, trusted, entry, grants),
        "verification.lock": LOCK,
    }
    for module, text in modules.items():
        files[module_file(module)] = text
    return files
