#!/usr/bin/env python3
"""Diagnostic-code coverage report for the Una v0.1 repository.

Usage: python3 diag-coverage.py [REPO_ROOT]

For every code registered in docs/spec/13-diagnostics-and-abi.md section 4 it counts:
  fail   conformance cases expecting `error <code>`
  pos    positive conformance cases (ok, tier0-ok, run-output, test-pass, no error
         expectation) whose `-- spec:` cites a section cited by one of the code's failing
         cases, or whose `-- test:` text names the code; a second column counts positives
         citing only another section of the same spec file
  crate  crate test files (crates/*/tests/**, including snapshots) mentioning the code,
         with the total number of mentions
  explain  whether crates/una-diag/explain/<code>.md exists, and whether it has a
         failing and a fixed example (checked by crates/una-cli/tests/explain_examples.rs)
Output is a Markdown table followed by gap lists. Deterministic: sorted everywhere.
"""
import os
import re
import sys
from collections import defaultdict

ROOT = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "../../../.."))
SPEC13 = os.path.join(ROOT, "docs/spec/13-diagnostics-and-abi.md")
CONF = os.path.join(ROOT, "tests/conformance")
EXPLAIN = os.path.join(ROOT, "crates/una-diag/explain")
DIRECTIVE = re.compile(r"^-- (test|spec|expect|grant|pending): (.*)$")


def registry():
    text = open(SPEC13, encoding="utf-8").read()
    sec = text.split("\n## 4.")[1].split("\n## 5.")[0]
    codes = []
    for m in re.finditer(r"^\| (U\d{4}) \| (\w+) \| (.*?) \|$", sec, re.M):
        codes.append((m.group(1), m.group(2), m.group(3)))
    return codes


def directives(path):
    out = []
    with open(path, encoding="utf-8-sig", errors="replace") as f:
        for line in f:
            m = DIRECTIVE.match(line.rstrip("\r\n"))
            if not m:
                break
            out.append((m.group(1), m.group(2)))
    return out


def cases():
    """Yield (case_name, directives) for every conformance case."""
    for area in sorted(os.listdir(CONF)):
        adir = os.path.join(CONF, area)
        if area.startswith(".") or not os.path.isdir(adir):
            continue
        for entry in sorted(os.listdir(adir)):
            if entry.startswith("."):
                continue
            p = os.path.join(adir, entry)
            if entry.endswith(".una") and os.path.isfile(p):
                yield f"{area}/{entry}", directives(p)
            elif os.path.isdir(p) and os.path.exists(os.path.join(p, "una.toml")):
                found = []
                for dp, dn, fn in os.walk(os.path.join(p, "src")):
                    dn.sort()
                    for n in sorted(fn):
                        if n.endswith(".una"):
                            d = directives(os.path.join(dp, n))
                            if d:
                                found.append(d)
                yield f"{area}/{entry}/", found[0] if found else []


def sections(spec_value):
    """`spec/10 §3, §4` -> {'spec/10 §3', 'spec/10 §4'}; also handles `spec/05 §6, spec/10 §9`."""
    out, cur = set(), None
    for part in [p.strip() for p in spec_value.split(",")]:
        m = re.match(r"(spec/\d\d)\s*(§\S+)?", part)
        if m:
            cur = m.group(1)
            if m.group(2):
                out.add(f"{cur} {m.group(2)}")
        else:
            m = re.match(r"(§\S+)", part)
            if m and cur:
                out.add(f"{cur} {m.group(1)}")
    return out


def crate_mentions():
    files, counts = defaultdict(set), defaultdict(int)
    base = os.path.join(ROOT, "crates")
    for crate in sorted(os.listdir(base)):
        tdir = os.path.join(base, crate, "tests")
        if not os.path.isdir(tdir):
            continue
        for dp, dn, fn in os.walk(tdir):
            for n in fn:
                p = os.path.join(dp, n)
                try:
                    t = open(p, encoding="utf-8").read()
                except (UnicodeDecodeError, OSError):
                    continue
                for c in re.findall(r"U\d{4}", t):
                    files[c].add(os.path.relpath(p, ROOT))
                    counts[c] += 1
    return files, counts


def main():
    codes = registry()
    fail, fail_secs, pos = defaultdict(list), defaultdict(set), defaultdict(list)
    positives = []
    for name, d in cases():
        spec = set().union(*[sections(v) for k, v in d if k == "spec"]) if d else set()
        test = " ".join(v for k, v in d if k == "test")
        errs = {re.match(r"error (U\d{4})", v).group(1) for k, v in d if k == "expect" and v.startswith("error ")}
        for c in errs:
            fail[c].append(name)
            fail_secs[c] |= spec
        if not errs and d:
            positives.append((name, spec, test))
    pos_file = defaultdict(list)
    for c, _, _ in codes:
        files = {s.split()[0] for s in fail_secs[c]}
        for name, spec, test in positives:
            if spec & fail_secs[c] or c in test:
                pos[c].append(name)
            elif {s.split()[0] for s in spec} & files:
                pos_file[c].append(name)
    cfiles, ccount = crate_mentions()
    print("| Code | Domain | Fail cases | Positive cases (same §) | Positive (same spec file only) | Crate test files / mentions | Explain | Failing-case spec sections |")
    print("| --- | --- | --- | --- | --- | --- | --- | --- |")
    no_fail, no_pos, no_crate, no_explain = [], [], [], []
    for c, dom, meaning in codes:
        ex = os.path.join(EXPLAIN, f"{c}.md")
        if os.path.exists(ex):
            t = open(ex, encoding="utf-8").read()
            exs = "yes+examples" if ("## Failing example" in t and "## Fixed example" in t) else "yes (no examples)"
        else:
            exs = "MISSING"
            no_explain.append(c)
        print(f"| {c} | {dom} | {len(fail[c])} | {len(pos[c])} | {len(pos_file[c])} | {len(cfiles[c])} / {ccount[c]} | {exs} | {', '.join(sorted(fail_secs[c])) or '-'} |")
        if not fail[c]:
            no_fail.append(c)
        if not pos[c]:
            no_pos.append(c)
        if not cfiles[c]:
            no_crate.append(c)
    registered = {c for c, _, _ in codes}
    stray_conf = sorted(set(fail) - registered)
    stray_crate = sorted(set(cfiles) - registered)
    print()
    print(f"Registered codes: {len(codes)}; conformance cases: {len(positives) + len({n for v in fail.values() for n in v})} (approx, positive + failing)")
    print(f"Codes with no failing conformance case: {', '.join(no_fail) or 'none'}")
    print(f"Codes with no positive counterpart (same section): {', '.join(no_pos) or 'none'}")
    print(f"  of which also none in the same spec file: {', '.join(c for c in no_pos if not pos_file[c]) or 'none'}")
    print("  (every explain file's fixed example is also a positive twin, checked by crates/una-cli/tests/explain_examples.rs)")
    print(f"Codes not mentioned in any crate test: {', '.join(no_crate) or 'none'}")
    print(f"Codes without an explain file: {', '.join(no_explain) or 'none'}")
    print(f"Unregistered codes in conformance expectations: {', '.join(stray_conf) or 'none'}")
    print(f"Unregistered codes mentioned in crate tests: {', '.join(stray_crate) or 'none'}")
    for c in stray_crate:
        print(f"  {c}: {', '.join(sorted(cfiles[c]))}")


if __name__ == "__main__":
    main()
