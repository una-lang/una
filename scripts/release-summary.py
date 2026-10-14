#!/usr/bin/env python3
"""The public summary of a release's validation record (#717, docs/release-gates.md §3).

Reads a validation record that scripts/release-gates.sh wrote (record.md) and writes its
summary from a fixed template: the commit, the run's date, OS and architecture, the toolchain,
the gate table without log digests, the archives with their SHA-256 and the verdict. Nothing
else of the record is copied: no subject, no notes, no log digest, no docs bundle.

Every field is checked against its shape before it is written, and a field that does not match
stops the summary: no free text of the record can reach it. When the environment variable
UNA_PRIVACY_PATTERN is set, the finished summary must not match it (case-insensitive extended
regular expression); when it is unset, the summary is written and a warning says that it is
unchecked, and docs/release-gates.md §4 does not publish an unchecked summary.

Usage: scripts/release-summary.py VERSION RECORD OUT
Exit code: 0 written, 1 a field out of shape or a privacy match, 2 a usage error.
"""
import os
import re
import subprocess
import sys

VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+")
COMMIT = re.compile(r"- Commit: `([0-9a-f]{40})`")
# The run line of record.md: date, `uname -sm`, `rustc --version`, `docker --version`.
RUN = re.compile(
    r"- Run: ([0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z), "
    r"([A-Za-z]+ [A-Za-z0-9_]+), "
    r"(rustc [0-9]+\.[0-9]+\.[0-9]+ \([0-9a-f]+ [0-9]{4}-[0-9]{2}-[0-9]{2}\)|rustc unknown), "
    r"(Docker version [0-9]+\.[0-9]+\.[0-9]+, build [0-9a-f]+|Docker unavailable)"
)
GATES = ["clone", "build", "mac", "linux", "linux-arm64", "release", "reproducible",
         "determinism", "native", "site", "formal"]
GATE_ROW = re.compile(r"\| ([a-z0-9-]+) \| (PASS|FAIL) \| ([0-9]+) \| `[0-9a-f]{64}` \|")
ARCHIVE_ROW = re.compile(r"\| `(una-[0-9.]+-[a-z0-9_]+(?:-[a-z0-9_]+)*\.tar\.gz)` \| `([0-9a-f]{64})` \|")
VERDICT = re.compile(r"Verdict: (PASS \(every gate\)|FAIL \([a-z0-9, -]+\)|INCOMPLETE \(not run: [a-z0-9, -]+\))")


def fail(message):
    print(f"release-summary: FAIL ({message})", file=sys.stderr)
    sys.exit(1)


def one(pattern, lines, what):
    found = [m for m in map(pattern.fullmatch, lines) if m]
    if len(found) != 1:
        fail(f"the record has {len(found)} {what} lines in the expected shape, not 1")
    return found[0]


def section(lines, heading):
    """The lines under `heading`, up to the next `## ` heading."""
    if heading not in lines:
        return []
    start = lines.index(heading) + 1
    end = next((i for i in range(start, len(lines)) if lines[i].startswith("## ")), len(lines))
    return lines[start:end]


def main():
    if len(sys.argv) != 4:
        print("usage: scripts/release-summary.py VERSION RECORD OUT", file=sys.stderr)
        sys.exit(2)
    version, record, out = sys.argv[1:]
    if not VERSION.fullmatch(version):
        fail(f"{version!r} is not a version")
    with open(record, encoding="utf-8") as f:
        lines = f.read().splitlines()

    commit = one(COMMIT, lines, "commit").group(1)
    run = one(RUN, lines, "run")
    verdict = one(VERDICT, lines, "verdict").group(1)

    # The gate table: every row of the record's first table, each in shape.
    header = "| Gate | Result | Seconds | Log sha256 |"
    if header not in lines:
        fail("the record has no gate table")
    gates = []
    for line in lines[lines.index(header) + 2:]:
        if not line.startswith("|"):
            break
        m = GATE_ROW.fullmatch(line)
        if not m:
            fail(f"a gate row out of shape: {line[:40]!r}")
        if m.group(1) not in GATES:
            fail(f"an unknown gate {m.group(1)!r}")
        gates.append(m.groups())
    if not gates:
        fail("the gate table is empty")

    archives = []
    for line in section(lines, "## Archives"):
        if line.startswith("| `"):
            m = ARCHIVE_ROW.fullmatch(line)
            if not m:
                fail(f"an archive row out of shape: {line[:40]!r}")
            if not m.group(1).startswith(f"una-{version}-"):
                fail(f"{m.group(1)} is not an archive of {version}")
            archives.append(m.groups())

    text = [f"# Una v{version}: release validation", ""]
    text += [f"- Commit: `{commit}`", f"- Run: {run.group(1)}, {run.group(2)}",
             f"- Toolchain: {run.group(3)}; {run.group(4)}", ""]
    text += ["| Gate | Result | Seconds |", "| --- | --- | --- |"]
    text += [f"| {g} | {r} | {s} |" for g, r, s in gates]
    text += ["", "## Archives", ""]
    if archives:
        text += ["| Archive | sha256 |", "| --- | --- |"]
        text += [f"| `{name}` | `{digest}` |" for name, digest in archives]
    else:
        text += ["None: the release gate did not run or did not pack them."]
    text += ["", f"Verdict: {verdict}", "",
             "The gates are defined in `docs/release-gates.md` at the commit above.", ""]
    summary = "\n".join(text)

    pattern = os.environ.get("UNA_PRIVACY_PATTERN", "")
    if pattern:
        # grep -E, so the pattern means what it means in the other checks (site-bundle.sh).
        found = subprocess.run(["grep", "-E", "-i", "-q", "--", pattern], input=summary.encode())
        if found.returncode == 0:
            fail("the summary matches UNA_PRIVACY_PATTERN")
        if found.returncode != 1:
            fail("grep could not apply UNA_PRIVACY_PATTERN")
    else:
        print("release-summary: warning: UNA_PRIVACY_PATTERN is unset, so the summary is unchecked "
              "and is not published (docs/release-gates.md §4)", file=sys.stderr)
    with open(out, "w", encoding="utf-8") as f:
        f.write(summary)
    print(f"summary: {out} ({'privacy checked' if pattern else 'privacy unchecked'})")


if __name__ == "__main__":
    main()
