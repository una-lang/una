#!/usr/bin/env python3
"""The public summary of a release's validation record (#717, docs/release-gates.md §3).

Reads a validation record that scripts/release-gates.sh wrote (record.md) and writes its
summary from a fixed template: the commit, the run's date, OS and architecture, the toolchain,
the gate table without log digests, the archives with their SHA-256, the strip that removed
the debug sections of each embedded runtime, with its version and its binary's digest (#1359),
the dependency audit's tool and advisory database (#770) and the verdict. Nothing
else of the record is copied: no subject, no notes, no log digest, no docs bundle, no Linux
toolchain or musl lines (#1971).

Every field is checked against its shape before it is written, and a field that does not match
stops the summary: no free text of the record can reach it. The privacy pattern comes on the
file descriptor that UNA_PRIVACY_PATTERN_FD names, as scripts/release-gates.sh passes it, else
from the environment variable UNA_PRIVACY_PATTERN, for a run by hand; the finished summary must
not match it (case-insensitive extended regular expression). Without it, or when
UNA_SUMMARY_NEUTRALITY=unchecked says that the run checked no page against the neutrality
pattern, the summary opens with a line that says it is unchecked, a warning says so, and
docs/release-gates.md §4 does not publish an unchecked summary.

Usage: scripts/release-summary.py VERSION RECORD OUT
Exit code: 0 written, 1 a field out of shape or a privacy match, 2 a usage error.
"""
import os
import re
import subprocess
import sys

# A release, or a pre-release such as 0.4.0-alpha.1 (SemVer: dot-separated alphanumerics).
VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?")
COMMIT = re.compile(r"- Commit: `([0-9a-f]{40})`")
# The run line of record.md: date, `uname -sm`, `rustc --version`, `docker --version`.
RUN = re.compile(
    r"- Run: ([0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z), "
    r"([A-Za-z]+ [A-Za-z0-9_]+), "
    r"(rustc [0-9]+\.[0-9]+\.[0-9]+ \([0-9a-f]+ [0-9]{4}-[0-9]{2}-[0-9]{2}\)|rustc unknown), "
    r"(Docker version [0-9]+\.[0-9]+\.[0-9]+, build [0-9a-f]+|Docker unavailable)"
)
# The gates of scripts/release-gates.sh, in its order (its ALL list); a test of
# crates/una-cli/tests/release_gates.rs keeps the two equal. A row of the record's gate table
# naming any other gate is refused.
GATES = ["clone", "build", "deps", "mac", "linux", "linux-arm64", "release", "reproducible",
         "determinism", "native", "timing-channels", "owner-replay", "sanitize", "docs", "formal", "hunt",
         "hunt-linux-arm64", "solver-identity", "limits", "limits-linux-arm64", "limits-linux",
         "toolchain-free"]
# Earlier names of a gate, each mapped to its name now (docs/release-gates.md §3). The record of a
# run made before #2081 names the documentation gate `site`, which then also built the website;
# its row is accepted and copied as it was written, never renamed. A record that names a gate by
# both names is refused: no run of one script writes both.
EARLIER = {"site": "docs"}
GATE_ROW = re.compile(r"\| ([a-z0-9-]+) \| (PASS|FAIL) \| ([0-9]+) \| `[0-9a-f]{64}` \|")
ARCHIVE_ROW = re.compile(r"\| `(una-[0-9A-Za-z._-]+\.tar\.gz)` \| `([0-9a-f]{64})` \|")
# The target part of an archive's name, after `una-<version>-`.
TARGET = re.compile(r"[a-z0-9_]+(?:-[a-z0-9_]+)*")
# The dependency audit's lines (#770): cargo-deny's version, and the advisory database's commit
# and its date as git prints it (%cI).
DEPS_TOOL = re.compile(r"- Tool: (cargo-deny [0-9]+\.[0-9]+\.[0-9]+)")
DEPS_DB = re.compile(
    r"- Advisory database: `([0-9a-f]{40})` "
    r"\(([0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(?:Z|[+-][0-9]{2}:[0-9]{2}))\)"
)
DEPS_NONE = "None: the deps gate did not run or did not fetch the advisory database."
# The strip of each embedded runtime (#1359), as scripts/release.sh prints it: on macOS the Xcode
# toolchain's, named by its Xcode release or Command Line Tools version; on Linux GNU strip's
# version line, in Ubuntu's form (`GNU strip (GNU Binutils for Ubuntu) 2.42`) or AlmaLinux 8's
# gcc-toolset form (`GNU strip version 2.40-21.el8`, #2105); each with the SHA-256 of the strip
# binary.
STRIP_ROW = re.compile(
    r"- ([a-z0-9_]+(?:-[a-z0-9_]+)*): "
    r"(xcrun strip -S \((?:Xcode [0-9.]+ build [0-9A-Za-z]+|Command Line Tools [0-9.]+)\)"
    r"|GNU strip \([A-Za-z0-9 .]+\) [0-9][0-9.]* --strip-debug -D"
    r"|GNU strip version [0-9]+\.[0-9]+(?:\.[0-9]+)?-[0-9]+(?:\.[0-9]+)*\.el[0-9]+(?:_[0-9]+)* --strip-debug -D)"
    r", sha256 [0-9a-f]{64}"
)
STRIP_NONE = "None: the release gate did not run or did not record the strip tool."
# Every character but "\n" at which str.splitlines() ends a line.
OTHER_BREAKS = re.compile("[\r\x0b\x0c\x1c\x1d\x1e\x85\u2028\u2029]")
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
    # Lines end at "\n" only (#1971): a record is read with no newline translation, and one that
    # holds any other character that str.splitlines() would end a line at, a carriage return
    # in a line copied from a log included, is refused, so that no copied line can pose as a
    # heading or a field of its own.
    with open(record, encoding="utf-8", newline="") as f:
        text = f.read()
    if OTHER_BREAKS.search(text):
        fail("the record holds a line break other than \\n")
    lines = text.split("\n")

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
        if m.group(1) not in GATES and m.group(1) not in EARLIER:
            fail(f"an unknown gate {m.group(1)!r}")
        gates.append(m.groups())
    if not gates:
        fail("the gate table is empty")
    named = {g for g, _, _ in gates}
    for earlier, now in EARLIER.items():
        if earlier in named and now in named:
            fail(f"the gate {now!r} is named both {earlier!r} and {now!r}")

    archives = []
    for line in section(lines, "## Archives"):
        if line.startswith("| `"):
            m = ARCHIVE_ROW.fullmatch(line)
            if not m:
                fail(f"an archive row out of shape: {line[:40]!r}")
            prefix, suffix = f"una-{version}-", ".tar.gz"
            if not m.group(1).startswith(prefix) or not TARGET.fullmatch(m.group(1)[len(prefix):-len(suffix)]):
                fail(f"{m.group(1)} is not an archive of {version}")
            archives.append(m.groups())

    # The strip of each embedded runtime: every line in shape, or none.
    strips = [line for line in section(lines, "## Runtime strip")
              if line.startswith("- ") or line.startswith("None")]
    if strips and strips != [STRIP_NONE]:
        for line in strips:
            if not STRIP_ROW.fullmatch(line):
                fail(f"a runtime strip line out of shape: {line[:40]!r}")
    else:
        strips = []

    # The dependency audit: its two lines in shape, or none when the gate did not reach them.
    deps = [line for line in section(lines, "## Dependency audit")
            if line.startswith("- ") or line.startswith("None")]
    if deps and deps != [DEPS_NONE]:
        if len(deps) != 2 or not DEPS_TOOL.fullmatch(deps[0]) or not DEPS_DB.fullmatch(deps[1]):
            fail("the dependency audit is out of shape")
    else:
        deps = []

    fd = os.environ.get("UNA_PRIVACY_PATTERN_FD", "")
    if fd:
        with os.fdopen(int(fd), "r", encoding="utf-8", errors="strict") as handle:
            pattern = handle.read().rstrip("\n")
    else:
        pattern = os.environ.get("UNA_PRIVACY_PATTERN", "")
    neutrality_unchecked = os.environ.get("UNA_SUMMARY_NEUTRALITY", "") == "unchecked"
    text = [f"# Una v{version}: release validation", ""]
    # An unchecked summary says so in its own text, so that it cannot pass for a published one.
    if not pattern:
        text += ["**Unchecked: written without the privacy pattern; not for publication.**", ""]
    if neutrality_unchecked:
        text += ["**Unchecked: the pages were not checked against the neutrality pattern; not for publication.**", ""]
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
    text += ["", "## Runtime strip", ""]
    text += strips if strips else [STRIP_NONE]
    text += ["", "## Dependency audit", ""]
    text += deps if deps else [DEPS_NONE]
    text += ["", f"Verdict: {verdict}", "",
             "The gates are defined in `docs/release-gates.md` at the commit above.", ""]
    summary = "\n".join(text)

    if len(pattern) > 60000:
        # More than a pipe holds at once: the write below would wait for a reader forever.
        fail("the privacy pattern is longer than 60,000 characters")
    if pattern:
        # grep -E, so the pattern means what it means in the other checks (site-bundle.sh). The
        # pattern goes to grep through a pipe (-f), never on a command line, where any process
        # of the machine could read it.
        read, write = os.pipe()
        os.write(write, pattern.encode() + b"\n")
        os.close(write)
        # Nor in grep's environment: it needs only the pipe.
        env = {k: v for k, v in os.environ.items() if k != "UNA_PRIVACY_PATTERN"}
        found = subprocess.run(["grep", "-E", "-i", "-q", "-f", f"/dev/fd/{read}"],
                               input=summary.encode(), pass_fds=(read,), env=env)
        os.close(read)
        if found.returncode == 0:
            fail("the summary matches UNA_PRIVACY_PATTERN")
        if found.returncode != 1:
            fail("grep could not apply UNA_PRIVACY_PATTERN")
    else:
        print("release-summary: warning: no privacy pattern, so the summary is unchecked "
              "and is not published (docs/release-gates.md §4)", file=sys.stderr)
    if neutrality_unchecked:
        print("release-summary: warning: the run checked no page against the neutrality pattern, "
              "so the summary is unchecked and is not published (docs/release-gates.md §4)", file=sys.stderr)
    with open(out, "w", encoding="utf-8") as f:
        f.write(summary)
    print(f"summary: {out} ({'privacy checked' if pattern else 'privacy unchecked'})")


if __name__ == "__main__":
    main()
