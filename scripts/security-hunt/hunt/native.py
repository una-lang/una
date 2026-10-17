"""The native sample of the two-run oracle (docs/plan/security-hunt-oracle.md §4 steps 3 and 5,
#792): a fixed sample of the oracle's programs also runs as a native executable, every variant
the interpreter ran, and the two engines must agree on everything a run shows (`spec/16 §5`).

The comparison is the one of the native equivalence test (crates/una-cli/tests/native.rs):
- the exit code;
- the standard output, line for line, against the run document's `output`;
- the trace, against the document's `trace`, every event and field, the result of each
  `Clock.now` aside (two runs read the clock at different times); a redacted argument is
  compared as it is written, its label included;
- the standard error: empty on a run that ends `ok`; otherwise its first line is the error,
  `[<code>] Error: <message>`, equal to the document's first error, and every other line is a
  place, `  at <file>:<line>:<column>`, or the note that names the code's explanation,
  `  note: run `una explain <code>` for an explanation`;
- a run that starts tasks (`spec/19 §8` rule 4): the output is what the native trace says each
  task printed, each task's lines in its own order (a redacted line is not matched, and which
  task prints first is outside the model); on an ordered run, one that neither fails nor cancels
  a task, the trace equals the interpreter's and the output is the document's; on a run that is
  not ordered, the trace holds the interpreter's in order with the extra events those of tasks,
  and the output the document's plus only lines those extra events printed (`task_differences`);
- a run the interpreter ends with a resource abort, U6003, U6004 or U6005: where each engine runs
  out is not exact (ADR 0061), so the executable must end the same way, the same exit code and
  error, and the same standard output, as
  `a_copy_between_tasks_over_the_budget_is_u6003_in_both_backends` checks; and one engine's
  trace, its `seq` aside, must be a prefix of the other's, so that an effect one engine makes
  before its abort and the other never makes is a divergence. The place of the abort is not
  compared.
A difference is a backend divergence, a finding that fails the gate.

A program the native backend does not compile is not a free count: the programs that do not
compile are pinned by name, with their reason, in UNSUPPORTED (scripts/security-hunt/
native-unsupported.txt), and any other build failure, or a pinned program that compiles, is an
error.
"""

import hashlib
import json
import os
import re
import stat

from . import sandbox
from .program import Program

# The environment of every `una run` and `una build` of the oracle and the native sample: `una run`
# and `una build` do the full check, and a program with a conditioned operation (family Q, #813)
# has obligations for the solver. The gate sets it to the pinned solver's `PATH`
# (`drivers.solver_env`) before it runs anything; a program without obligations never calls it.
RUN_ENV = {}
# The solver's wall clock per query in those runs and builds, well below the run's own limit
# (`gate.RUN_TIMEOUT`, 20 seconds): a query the solver does not decide in 10 seconds ends as U5002,
# a gate error, before the run's limit can kill the run and make it look like an abort. A run
# killed at its limit anyway, its check slower in total, has its full check run on its own
# (`undecided`): only a check that decides every obligation lets the kill count as an abort.
SOLVER_WALLCLOCK = 10
SOLVER_FLAGS = ["--solver-wallclock", str(SOLVER_WALLCLOCK)]
FULL_CHECK_TIMEOUT = 60
SOLVER_CODES = {"U5002", "U5003"}


def undecided(una, files, name, root):
    """Why the full check of the variant `files` does not show that the solver decided it, or None
    when the check accepts it: a timeout, an output that is not a check document, a rejection, or
    U5002 or U5003 among its diagnostics. For a run killed at its time limit, whose check may have
    been what took the time."""
    rundir = sandbox.fresh_dir(root, f"{name}-c-")
    pkg = Program(op="oracle", rule="", edition="", seed=0, cls="open", files=files,
                  variant=name).write(rundir)
    r = sandbox.run([una, "--json", "check", ".", *SOLVER_FLAGS], cwd=pkg, writable=rundir,
                    timeout=FULL_CHECK_TIMEOUT, env_extra=RUN_ENV)
    if r.timed_out:
        return f"{name}: the run reached its time limit, and so did its full check"
    try:
        doc = json.loads(r.stdout)
    except ValueError:
        doc = None
    diags = doc.get("diagnostics") if isinstance(doc, dict) else None
    if (not isinstance(doc, dict) or doc.get("schema") != "una.diagnostics/1"
            or not isinstance(diags, list) or not all(isinstance(d, dict) for d in diags)):
        return f"{name}: the run reached its time limit, and its full check is not a document"
    codes = sorted({str(d.get("code")) for d in diags})
    if set(codes) & SOLVER_CODES:
        return (f"{name}: the run reached its time limit, and its full check did not decide: "
                + ", ".join(sorted(set(codes) & SOLVER_CODES)))
    if r.code != 0 or any(d.get("severity") == "error" for d in diags):
        return (f"{name}: the run reached its time limit, and its full check rejects it: "
                + ", ".join(codes))
    return None

# The sample is stratified, a fixed function of the fixed run's names:
# - the oracle's programs whose name's SHA-256 starts with a byte below SAMPLE_BELOW, 20 of 256,
#   about 8% of its programs and 5% or more of those it compares;
# - and, for every family (`families.family_of`) with a compared program, the compared program
#   whose name has the lowest SHA-256, so that no family the oracle compares is left without a
#   native comparison by chance.
SAMPLE_BELOW = 20
ABORT_CODES = ("U6003", "U6004", "U6005")
UNSUPPORTED = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                           "native-unsupported.txt")
NOT_COMPILED = "the native backend does not compile"
ERROR_LINE = re.compile(r"^\[(U\d{4})\] Error: (.*)$")
PLACE_LINE = re.compile(r"^  at .+:\d+:\d+$")


def key(name):
    """The order of the sample: the name's SHA-256."""
    return hashlib.sha256(name.encode("utf-8")).digest()


def sampled(name):
    """Whether the program `name` is in the hashed part of the native sample."""
    return key(name)[0] < SAMPLE_BELOW


def unsupported():
    """The programs pinned as not compiled by the native backend: {name: reason}."""
    out = {}
    try:
        with open(UNSUPPORTED, encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line or line.startswith("#"):
                    continue
                name, reason = (line.split("\t", 1) + [""])[:2]
                out[name.strip()] = reason.strip()
    except FileNotFoundError:
        pass
    return out


def timeless(trace):
    """`trace` with the result of each `Clock.now` replaced, as the native test does."""
    if not isinstance(trace, list):
        return trace
    out = []
    for event in trace:
        if (isinstance(event, dict) and event.get("capability") == "Clock"
                and event.get("op") == "now" and isinstance(event.get("outcome"), dict)):
            event = dict(event, outcome=dict(event["outcome"], value="<now>"))
        out.append(event)
    return out


def _starts_tasks(trace):
    return isinstance(trace, list) and any(isinstance(e, dict) and e.get("task") is not None
                                           for e in trace)


def _subsequence(part, whole):
    rest = iter(whole)
    return all(any(other == item for other in rest) for item in part)


def quote(text):
    """A text literal in Una syntax, with the escapes of `spec/01 §6`: `una_core::quote`, which
    the trace uses for a printed line and the native test for each line of standard output."""
    out = ['"']
    for c in text:
        o = ord(c)
        if c == '"':
            out.append('\\"')
        elif c == "\\":
            out.append("\\\\")
        elif c == "{":
            out.append("\\{")
        elif c == "}":
            out.append("\\}")
        elif c == "\n":
            out.append("\\n")
        elif c == "\t":
            out.append("\\t")
        elif o < 0x20 or o == 0x7F or 0x202A <= o <= 0x202E or 0x2066 <= o <= 0x2069:
            out.append("\\u{%x}" % o)
        else:
            out.append(c)
    out.append('"')
    return "".join(out)


def _task(event):
    """The task of an event, the root's "" (`task` absent or null)."""
    task = event.get("task")
    return task if isinstance(task, str) else ""


def _unnumbered(trace):
    return [{k: v for k, v in e.items() if k != "seq"} if isinstance(e, dict) else e
            for e in timeless(trace if isinstance(trace, list) else [])]


def _printed(events):
    return [(_task(e), e["args"]["line"]) for e in events
            if isinstance(e, dict) and e.get("capability") == "Console" and e.get("op") == "print"
            and isinstance(e.get("args"), dict) and isinstance(e["args"].get("line"), str)]


def _cancels(trace):
    """Whether a trace holds a task's cancellation: an event with no operation (`spec/19 §7`)."""
    return isinstance(trace, list) and any(
        isinstance(e, dict) and "capability" not in e and "op" not in e for e in trace)


def task_differences(stdout, interpreted, native, output, ordered):
    """How a native run that starts tasks departs from `spec/19 §8` rule 4: the rule of native.rs,
    and the document's `output` against the native standard output. On an ordered run, one that
    neither fails nor cancels a task, both are exact: the trace equals the interpreter's and the
    output is the document's, as a multiset of lines; only a run that is not ordered may hold the
    extra events and lines of tasks that started after a failed one. Lines whose printed form is
    redacted in the trace are not matched against it, and the interleaving of tasks' lines is
    compared only as each task's own order (`spec/19 §6` rule 7: which task prints first is
    outside the model)."""
    from collections import Counter

    differences = []
    events = native if isinstance(native, list) else []
    printed = _printed(events)
    lines = [quote(line) for line in stdout.splitlines()]
    redacted = any(line.startswith("<redacted") for _, line in printed)
    if not redacted and sorted(lines) != sorted(line for _, line in printed):
        differences.append("the output is not what the native trace printed")
    for task in sorted({t for t, _ in printed}):
        own = [line for t, line in printed if t == task]
        if not _subsequence(own, lines):
            differences.append(f"the output does not keep the order of the lines of task {task!r}")
    # The document's output, as a multiset: the native run may hold more lines only where its
    # trace holds more print events than the interpreter's, on a run that is not ordered.
    native_lines, doc_lines = Counter(stdout.splitlines()), Counter(output or [])
    if ordered:
        if native_lines != doc_lines:
            differences.append("the output is not the document's")
        if timeless(native) != timeless(interpreted):
            differences.append("the trace differs on an ordered run")
        return differences
    if native_lines != doc_lines:
        missing = doc_lines - native_lines
        extra = Counter(quote(line) for line in (native_lines - doc_lines).elements())
        extra_printed = (Counter(line for _, line in _printed(_unnumbered(native)))
                         - Counter(line for _, line in _printed(_unnumbered(interpreted))))
        if missing or extra - extra_printed:
            differences.append("the output is not the document's")
    if timeless(native) != timeless(interpreted):
        a, b = _unnumbered(interpreted), _unnumbered(native)
        roots = (lambda t: sum(1 for e in t if not (isinstance(e, dict) and e.get("task") is not None)))
        if not _subsequence(a, b) or roots(a) != roots(b):
            differences.append("the trace is neither the interpreter's nor one of a run that is not "
                               "ordered")
    return differences


def stderr_differences(stderr, doc):
    """The standard error against the document: empty on `ok`; otherwise the first line is the
    document's first error, `[<code>] Error: <message>`, and every other line a place."""
    lines = stderr.splitlines()
    if doc.get("status") == "ok":
        return [] if stderr == "" else ["standard error is not empty on a run that ends ok"]
    first = ERROR_LINE.match(lines[0]) if lines else None
    if first is None or (first.group(1), first.group(2)) != doc_error(doc):
        return [f"the error: interpreter {doc_error(doc)}, native "
                f"{(first.group(1), first.group(2)) if first else None}"]
    note = f"  note: run `una explain {first.group(1)}` for an explanation"
    if not all(PLACE_LINE.match(line) or line == note for line in lines[1:]):
        return ["standard error holds more than the error, its places and its note"]
    return []


def doc_error(doc):
    """The error of a run document as (code, message): its first error diagnostic, or None."""
    for d in doc.get("diagnostics", []):
        if d.get("severity") == "error":
            return d.get("code"), d.get("message")
    return None


def _prefix(a, b):
    return len(a) <= len(b) and b[:len(a)] == a


def compare(interp, doc, native, native_trace):
    """The differences between the interpreter's run (`interp`, its document `doc`) and the
    native run (`native`, its trace `native_trace`): an empty list when they agree."""
    if native.timed_out:
        return ["the native run timed out where the interpreter's did not"]
    differences = []
    if native.code != interp.code:
        differences.append(f"exit code: interpreter {interp.code}, native {native.code}")
    differences += stderr_differences(native.stderr, doc)
    expected = "".join(f"{line}\n" for line in doc.get("output", []))
    status = doc.get("status")
    codes = [d.get("code") for d in doc.get("diagnostics", []) if d.get("severity") == "error"]
    trace = doc.get("trace", [])
    if status == "aborted" and codes and all(c in ABORT_CODES for c in codes):
        # A resource abort: the same ending and output; one trace a prefix of the other.
        if native.stdout != expected:
            differences.append("standard output differs")
        a, b = _unnumbered(trace), _unnumbered(native_trace)
        if not (_prefix(a, b) or _prefix(b, a)):
            differences.append("neither trace is a prefix of the other")
        return differences
    if _starts_tasks(trace) or _starts_tasks(native_trace):
        # Ordered or not by the interpreter's run alone: a cancellation, or a failure, that only
        # the native run has is a divergence, never a reason to compare it more loosely.
        ordered = status != "failed" and not _cancels(trace)
        return differences + task_differences(native.stdout, trace, native_trace,
                                              doc.get("output"), ordered)
    if native.stdout != expected:
        differences.append("standard output differs")
    if timeless(native_trace) != timeless(trace):
        differences.append("the trace differs")
    return differences


def build_and_run(una, files, name, root, timeout, limits):
    """Builds the variant `files` natively in a fresh directory and runs it with `limits`:
    ("not compiled", stderr), ("build failed", stderr), or ("ran", (run, trace))."""
    rundir = sandbox.fresh_dir(root, f"{name}-n-")
    p = Program(op="oracle", rule="", edition="", seed=0, cls="open", files=files, variant=name)
    pkg = p.write(rundir)
    exe = os.path.join(rundir, "native-exe")
    built = sandbox.run([una, "--json", "build", ".", "--out", exe, *SOLVER_FLAGS], cwd=pkg,
                        writable=rundir, timeout=timeout * 6, env_extra=RUN_ENV)
    if built.timed_out or built.code != 0:
        why = "timed out" if built.timed_out else built.stderr.strip()[-300:]
        return ("not compiled" if NOT_COMPILED in built.stderr else "build failed", why)
    trace_file = os.path.join(rundir, "trace.json")
    ran = sandbox.run([exe, "--trace", trace_file, *limits], cwd=pkg, writable=rundir,
                      timeout=timeout)
    # The run could have put a link in its place: the trace is read only as a regular file of its
    # own directory, never through a link.
    try:
        fd = os.open(trace_file, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        with os.fdopen(fd, encoding="utf-8") as f:
            trace = json.load(f) if stat.S_ISREG(os.fstat(f.fileno()).st_mode) else None
    except (OSError, ValueError):
        trace = None
    return ("ran", (ran, trace))


def pair(una, root, timeout, limits, interpreted_files, native_files, name):
    """The gate's native canaries: runs `interpreted_files` on the interpreter and `native_files`
    natively, and returns the differences, or None when a side could not run as it should."""
    rundir = sandbox.fresh_dir(root, f"{name}-i-")
    p = Program(op="canary", rule="", edition="", seed=0, cls="open", files=interpreted_files,
                variant=name)
    pkg = p.write(rundir)
    interp = sandbox.run([una, "--json", "run", ".", *limits, *SOLVER_FLAGS], cwd=pkg,
                         writable=rundir, timeout=timeout, env_extra=RUN_ENV)
    try:
        doc = json.loads(interp.stdout)
    except ValueError:
        return None
    if interp.timed_out or doc.get("status") not in ("ok", "failed", "aborted"):
        return None
    kind, got = build_and_run(una, native_files, name, root, timeout, limits)
    if kind != "ran":
        return None
    return compare(interp, doc, got[0], got[1])
