"""The `hunt` release gate (docs/release-gates.md): a fixed, seeded run of the generators through
the must-reject and must-keep drivers, the two-run oracle and the comparison with the model's
re-check (`differ`), with canaries.

It fails closed. The gate passes only when all of these hold:
- no must-reject program is accepted, no refactor turns a rejected program into an accepted one,
  and the oracle sees no difference an observer below the varied literal could see;
- every check and every run ends in a known way (no timeout of `una check`, no internal error,
  no unreadable output, no nondeterministic run);
- the re-check rejects no declaration the checks accept; its `unknown` outcomes are exactly the
  pinned ones (`PINNED_UNKNOWN`); what it does not read is exactly the pinned counts per reason
  (`PINNED_FALLBACK`, `PINNED_NOT_RECHECKED`); its residual comparisons are the pinned count and
  each is clean; the programs it compares and the declarations it accepts meet their floors;
- each driver judged exactly the cases the fixed run generates, and none of those counts is zero;
  the edition 0.3 sample has a seed for every form of the fixed run (`programs` raises otherwise);
  the oracle's complete-against-complete comparisons, the programs that have one, and the programs
  whose variants differ at the literal's own level (liveness) meet their floors, and so do those
  of families B, C and Q (`MIN_FAMILY_COMPARED`);
- a program the oracle sets aside because the checks reject it is a closed call or a must-reject
  program, rejected with one of its expected codes, and their count per family is the pinned one;
- the solver decides every run and build of the oracle and the native sample (U5002 and U5003
  are errors), and a run killed at its time limit counts as an abort only when its full check,
  run on its own, decides (`native.undecided`); and no such run is killed, its check deciding
  (`MAX_TIMEOUTS_DECIDED`, 0), since each is a comparison not made;
- the whole gate ends within its platform's time budget (`TIME_BUDGETS`), a guard against a gate
  slowed as a whole;
- every canary is caught (`CANARIES`): a must-reject program that is in fact accepted; a program
  whose printed public literal the manifest marks as secret; an operation planted as accepting
  `internal`, reached under a secret branch, which only the `internal` observer must see; an
  operation planted as accepting `public`, sent a secret, whose redacted argument the oracle must
  report by itself; the same leak through a `together` branch; a must-reject program of edition
  0.3 expecting U3021 that is in fact accepted; Q1's ceiling probe with a plain ceiling, which
  the checks must accept, so that only the probe's `where` can reject it; a literal marked secret
  that decides, by the branch around it, whether a conditioned public operation runs, which the
  oracle must see in the trace alone, with the solver on the run's path; a must-reject program
  that does not parse, which must be a generator's error; planted Core documents the re-check
  must reject for their reason, with a control it must read clean (`DIFFER_CANARIES`,
  `DIFFER_CONTROL`); a native executable built
  from another variant than the one the interpreter ran, which the native comparison must report
  as a divergence; and three pairs of a program and a changed copy (an event's argument,
  `together` tasks, an abort at `--max-depth`), each of which must agree with itself and differ
  from its copy (`native_pairs`); and the sandbox's own canary (`sandbox_canary`, #793): a run
  writes in its directory but not in a sibling run's or in `/tmp`, and connects to the mock's
  loopback port but to no other;
- the native sample (`native.py`, #792): its size is the pinned one, its compared programs meet
  their floor, the interpreter and the native executable agree on every variant compared, and the
  programs the native backend does not compile are exactly those pinned by name.
"""

import atexit
import io
import json
import os
import socket
import subprocess
import sys
import time
from collections import Counter
from concurrent.futures import ThreadPoolExecutor

from . import drivers, native, oracle, sandbox
from .families import (DEFAULT_EDITIONS, OPERATOR_EDITIONS, OPERATORS, SAMPLE_COUNT,
                       SAMPLE_EDITION, editions_for, family_of, sample_forms)
from .families import conditions
from .program import MUST_ACCEPT, MUST_REJECT, OPEN, RESIDUAL, Program, derive_seed, rng_for
from .program import write_manifest
from .refactor import REFACTORS
from .una import Varied, header, join_decls, package

# The fixed run: changing any of these changes the gate, and docs/release-gates.md with it.
SEED = 767
COUNT = 50
# The editions of every operator but those of edition 0.3's rules, which run 0.2, as the edition
# gate's probe, and 0.3 (`OPERATOR_EDITIONS`, #813).
EDITIONS = DEFAULT_EDITIONS
REFACTOR_IDS = ("R6", "R9")
# The fixed run's sizes, pinned: a change of the generators that changes them fails the gate
# until this table and docs/release-gates.md say so.
PINNED = {"programs": 6288, "reject": 6288, "keep": 10851, "oracle": 637, "differ": 6288}
# The re-check comparison's expected `unknown` outcomes, by program, each with its reason: any
# other `unknown` is an error, and so is a pinned one that is no longer unknown (#791).
PINNED_UNKNOWN = {
    "F8-0.2-17ae8812afd2af8b-wrapping-secret-result": "out of F1 (expected): the primitive `fixed`",
    "F8-0.2-7c392c87affef562-saturating-secret-result": "out of F1 (expected): the primitive `fixed`",
    "F8-0.2-8fcd886c54b655f1-saturating-secret-result": "out of F1 (expected): the primitive `fixed`",
    "F8-0.2-f3c36fb985e9ab24-bit-secret-branch": "out of F1 (expected): the primitive `fixed`",
    "F8-0.3-17ae8812afd2af8b-wrapping-secret-result": "out of F1 (expected): the primitive `fixed`",
    "F8-0.3-8fcd886c54b655f1-saturating-secret-result": "out of F1 (expected): the primitive `fixed`",
    "F8-0.3-f3c36fb985e9ab24-bit-secret-branch": "out of F1 (expected): the primitive `fixed`",
}
# The residual programs the re-check compares (a step a secret may abort, which the full check's
# obligations guard): their count is pinned, so a silent drop fails: 13 of F8 and 50 of Q2.
PINNED_RESIDUAL = 63
# What the re-check compares, as the measured run found it. A program is compared when at least
# one of its own (`app.*`) functions is accepted by the re-check; the floors are about 90% of the
# measured run's 1,114 compared programs and 1,776 accepted declarations (#818).
MIN_DIFFER_COMPARED = 1002
MIN_ACCEPTED_DECLARATIONS = 1598
# Documents the re-check reads as a whole fallback (every line `not re-checked` for one reason):
# nothing in them is compared, so their count is pinned per reason, exactly.
PINNED_FALLBACK = {
    "a `case` on `app.main.Either`": 4,
    "a `case` on `app.main.Maybe`": 3,
    "the type `app.main.Box` with type arguments": 32,
    "the type `app.main.Chain` with type arguments": 7,
    "the type `app.main.Outer` with type arguments": 9,
    "the type `app.main.Sink` with type arguments": 18,
}
# The program's own declarations the re-check does not re-check, outside those documents, per
# reason, exactly: a change in what it reads is visible.
PINNED_NOT_RECHECKED = {
    "B1 restriction (impure via): `app.keys.masked`": 38,
    "F1 restriction (`div` on operands of mixed or unknown integer types)": 2,
    "a `case` on `app.main.Either`": 6,
    "a `case` on `app.main.Maybe`": 5,
    "a `handler` declaration": 340,
    "a `via` function, which no label rule checks": 490,
    "a `with` of a source other than a handler": 28,
    "a call of `app.edge.stash`, whose handler of `Vault.note` no `with` scope installs here": 8,
    "a call of `prelude.sort`": 14,
    "a conditioned operation (ADR 0092)": 76,
    "reaches `app.main.block`, which is not re-checked (the form `together`)": 5,
    "reaches `app.main.both`, which is not re-checked (the form `together`)": 9,
    "reaches `app.main.fan`, which is not re-checked (the form `together_for`)": 7,
    "the constraint `integer`": 16,
    "the constraint `ord`": 6,
    "the form `together_for`": 105,
    "the form `together`": 126,
    "the type `app.main.Bad`": 19,
    "the type `app.main.Box` with type arguments": 54,
    "the type `app.main.Chain` with type arguments": 16,
    "the type `app.main.Maybe` with type arguments": 7,
    "the type `app.main.Outer` with type arguments": 20,
    "the type `app.main.Sink` with type arguments": 54,
}
ACCEPTED = ("accepted", "accepted, conditional on A3")
# The planted Core documents of the re-check comparison (docs/reviews/formal-m2-replay/core-s27):
# v0.3.14's exports of S-27's two cases, each of which the re-check must reject at
# `app.main.main` for its reason, and the fix's export of the first, which it must read clean.
REPLAY = "docs/reviews/formal-m2-replay/core-s27"
DIFFER_CANARIES = {
    "S-27.interpolated_record.v0.3.14.json":
        "the text of a value, `secret[PII] Text`, does not flow into `public Text`",
    "S-27.when_literal_range.v0.3.14.json":
        "the result of `==`, `secret Bool`, does not flow into `public Bool`",
}
DIFFER_CONTROL = "S-27.interpolated_record.fixed.json"
# Floors on what the oracle compares, against a run that sets most programs aside or compares
# literals that decide nothing: about 90% of the measured run's 481 programs and 1,889
# complete-against-complete comparisons, and of its 117 live programs (#818, docs/release-gates.md).
MIN_ORACLE_COMPARED = 432
MIN_VARIANTS_COMPARED = 1700
MIN_LIVE = 105
# The same floors per family for the families whose oracle programs are few beside the others'
# (B, C and Q), about 90% of the measured run's compared programs and comparisons: 36 and 180 of
# B, 39 and 195 of C, 38 and 190 of Q (#818). Above the global floors, these families could lose
# most of their comparisons unseen.
MIN_FAMILY_COMPARED = {"B": (32, 162), "C": (35, 175), "Q": (34, 171)}
# What the oracle compares and finds live, by operator and form, pinned exactly: losing a whole
# form, or every live program of one, fails even above the floors; and the programs it sets aside
# because the checks reject them, among them the closed calls of B2, C6 and Q3 that a correct
# checker rejects (docs/release-gates.md).
ORACLE_FORMS = (
    "B2:function-value", "B2:grant-handler", "B2:narrowed", "C6:branch-value", "C6:for-elements",
    "C6:secret-fan-out", "C6:secret-sink", "C6:under-public-pc", "H3:install-under-pc",
    "H3:reach-under-pc", "K4:list", "K4:record", "K4:text", "P1:list", "P1:option", "P1:scalar",
    "Q3:guarded-dry", "Q3:guarded-live", "T8:effect-public", "T8:via-calls-policy",
)
LIVE_FORMS = ("C6:secret-fan-out", "H3:install-under-pc", "H3:reach-under-pc")
PINNED_SET_ASIDE_REJECTED = {"B": 14, "C": 20, "Q": 12, "T": 29}
# The native sample (#792): its size, pinned, the families it compares, pinned, and a floor on the
# programs it compares, about 90% of the measured run's (docs/release-gates.md).
NATIVE_SAMPLE = 45
NATIVE_FAMILIES = ("B", "C", "H", "K", "P", "Q", "T")
MIN_NATIVE_COMPARED = 36
MIN_NATIVE_VARIANTS = 165
CHECK_TIMEOUT = 60
RUN_TIMEOUT = 20
WORKERS = 4
# The whole gate's time budget, in seconds, never a limit on coverage. Every check and run has a
# time limit of its own, so a hang ends there; the budget guards against slowness that adds up
# over the run, from a regression or a machine too loaded to measure on. The fixed run of 6,288
# programs, the edition 0.3 sample included, took 226 to 253 seconds in four runs (2026-10-16),
# the slower ones under more load (load average at the start and the end): 228 (1.9, 3.8), 226
# (2.2, 4.4), 247 (3.5, 6.3) and 253 (4.9, 8.3), the same programs and the same compiler, so
# 270 seconds leaves 17 seconds above the slowest, about 6%. A run over it is resolved by
# measuring again, on a quieter machine, or by raising the budget on a measurement, never by
# shrinking the run or its sample. With each run in a directory of its own (#793) and the gate's
# shared proof-cache key, the gate took 252 and 253 seconds at a load of about 5 (2026-10-17).
TIME_BUDGET_MACOS = 270
# The budget of the gate on Linux (#793), in the hardened container of `scripts/hunt-linux.sh`,
# where every run also starts the Landlock launcher: set from measured quiet runs on that
# container, not from the macOS number. On Docker Desktop's VM on the same Apple M1 Max, the gate
# took 362 and 359 seconds at a load of about 4 to 5 (2026-10-17), so 390 seconds is the slower
# plus 7%, rounded up to 5 seconds.
TIME_BUDGET_LINUX = 390
# The budget by sandbox backend. A backend with no budget here has none pinned, which is an error
# of the gate: no platform runs unbudgeted.
TIME_BUDGETS = {"sandbox-exec": TIME_BUDGET_MACOS, "landlock": TIME_BUDGET_LINUX}
# The most runs killed at their time limit, their full check deciding, that the gate allows: the
# measured run had none, and each such kill is a comparison the oracle does not make (#818).
MAX_TIMEOUTS_DECIDED = 0

MOD = "app.main"
CONSOLE_MAIN = "pub main() -> Unit\n  uses @console: Console[print]\n=\n"


def programs():
    """Every program of the fixed run, in a fixed order. Raises when the edition 0.3 sample has no
    seed for a form of the fixed run (`sample_forms`), so the gate fails closed."""
    out = []
    for op in sorted(OPERATORS):
        def seed_of(i, op=op):
            return derive_seed(SEED, op, i)
        sample, missing = sample_forms(op, seed_of, COUNT)
        if missing:
            raise ValueError(f"the edition 0.3 sample of {op} has no seed for the forms "
                             f"{', '.join(sorted(missing))}")
        for i in range(COUNT):
            for edition in editions_for(op, i, sample):
                out.append(OPERATORS[op](seed_of(i), edition))
        # The sample's seeds past the run's, for the forms only another edition had: edition 0.3
        # only.
        for i in sorted(j for j in sample if j >= COUNT):
            out.append(OPERATORS[op](seed_of(i), SAMPLE_EDITION))
    return out


def canary_reject():
    """A must-reject row whose program the checks accept: the must-reject driver must call it a
    finding."""
    text = join_decls(header(MOD), CONSOLE_MAIN + '  @console.print("canary")')
    files = package("canary", "0.2", {MOD: text}, entry=MOD, grants=["console"])
    return Program(op="canary-reject", rule="the gate's own canary", edition="0.2", seed=0,
                   cls=MUST_REJECT, files=files, expect=["U4001"])


def canary_oracle():
    """An accepted program that prints a public literal, which its manifest marks as a secret: its
    variants differ in public output, and the oracle must call it a leak."""
    v = Varied()
    lit = v.lit("Int", "secret", "5")
    text = join_decls(header(MOD), CONSOLE_MAIN + f"  let x: Int = {lit}\n"
                      '  @console.print("{x}")')
    files, vary = v.place(package("canary", "0.2", {MOD: text}, entry=MOD, grants=["console"]))
    return Program(op="canary-oracle", rule="the gate's own canary", edition="0.2", seed=0,
                   cls=OPEN, files=files, vary=vary)


def canary_oracle_tasks():
    """An accepted program whose literal, public in the code and marked secret by its manifest,
    decides how many tasks of a `together for` call an operation an observer at `public` sees;
    it prints a constant. Its variants differ only in those tasks' events, so the oracle must call
    it a leak at `public` through the tasks' trace alone (#807)."""
    v = Varied()
    lit = v.lit("Int", "secret", "3")
    text = join_decls(
        header(MOD), NOTE,
        "fan(n: Int) -> Unit\n  uses @note: Note[note]\n=\n  let k = if n > 4 then 4 else n\n"
        "  let notes = together for p in 0..<k\n    @note.note(p)\n  unit",
        CONSOLE_MAIN + f"  let x: Int = {lit}\n  fan(x) with @note = quiet()\n"
        '  @console.print("done")')
    files, vary = v.place(package("canary", "0.2", {MOD: text}, entry=MOD, grants=["console"]))
    return Program(op="canary-oracle-tasks", rule="the gate's own canary", edition="0.2", seed=0,
                   cls=OPEN, files=files, vary=vary)


def residual_verdicts(r):
    """What a residual row that tier 0 accepts gives the gate, as (errors, findings, notes): an
    invalid full check is an error; a refuted row the full check accepts (`missed`) and a public
    print after the step that it accepts are findings; one that tier 0 accepts is a note."""
    errors, findings, notes = [], [], []
    if r.get("invalid"):
        errors.append(f"differ {r['name']}: residual row invalid ({r.get('full_kind')})")
    if r.get("missed"):
        findings.append(f"differ {r['name']}: the full check accepts it, and it must "
                        f"reject it with {', '.join(r['expect'])}")
    if r.get("triage"):
        findings.append(f"differ {r['name']}: the full check accepts a public print after "
                        "the step")
    if r.get("triage_tier0"):
        notes.append(f"differ {r['name']}: tier 0 accepts a public print after the step, "
                     "and the full check decides it")
    return errors, findings, notes


def oracle_family_errors(oracle_rows):
    """The oracle's per-family checks: the programs set aside because the checks reject them,
    counted per family against `PINNED_SET_ASIDE_REJECTED`, and the compared programs and
    comparisons of each family of `MIN_FAMILY_COMPARED` against its floors."""
    errors = []
    rejected = Counter(family_of(r["op"]) for r in oracle_rows
                       if str(r.get("set_aside", "")).startswith("rejected by the checks"))
    for family in sorted(set(rejected) | set(PINNED_SET_ASIDE_REJECTED)):
        if rejected.get(family, 0) != PINNED_SET_ASIDE_REJECTED.get(family, 0):
            errors.append(f"oracle: {rejected.get(family, 0)} programs of family {family} set "
                          "aside because the checks reject them, and the gate pins "
                          f"{PINNED_SET_ASIDE_REJECTED.get(family, 0)}")
    for family, (min_programs, min_variants) in sorted(MIN_FAMILY_COMPARED.items()):
        rows_of = [r for r in oracle_rows
                   if family_of(r["op"]) == family and r.get("compared", 0) > 0]
        variants_of = sum(r["compared"] for r in rows_of)
        if len(rows_of) < min_programs or variants_of < min_variants:
            errors.append(f"oracle: family {family} has {len(rows_of)} programs and "
                          f"{variants_of} variants compared, under its floors of {min_programs} "
                          f"and {min_variants}")
    return errors


def timeouts_decided(oracle_rows):
    """The runs killed at their time limit whose full check, run on its own, decided: the oracle's
    base runs and variants (`base_timed_out`, `timed_out`) and the native pass's own runs
    (`pass_timed_out`). A sampled program's native comparison reuses the oracle's run, so its
    `skipped` entry is not counted again."""
    return sum(int(bool(r.get("base_timed_out")))
               + sum(1 for v in r.get("variants", []) if v.get("timed_out"))
               + r.get("native", {}).get("pass_timed_out", 0) for r in oracle_rows)


def budget_error(elapsed, backend=None):
    """The error of a gate that took `elapsed` seconds on `backend` (this platform's by default),
    over its budget in `TIME_BUDGETS` or with none pinned, or None."""
    backend = sandbox.backend() if backend is None else backend
    budget = TIME_BUDGETS.get(backend)
    if budget is None:
        return (f"the gate took {elapsed:.0f} seconds, and no time budget is pinned for the "
                f"{backend} sandbox")
    if elapsed > budget:
        return (f"the gate took {elapsed:.0f} seconds, over its budget of {budget} for the "
                f"{backend} sandbox: a slower toolchain, or a machine too loaded to measure on")
    return None


def canary_reject_boundary():
    """A must-reject row of edition 0.3 expecting U3021 whose program the checks accept: a call
    into a boundary granted within its ceiling. The must-reject driver must call it a finding on
    the edition 0.3 path too (#813)."""
    store = join_decls(header("app.store"), "pub capability Store\n  op read(key: Text) -> Text")
    edge = join_decls(header("app.edge", imports=[("app.store", ["Store"])]),
                      "pub fetch(key: Text) -> Text\n  uses @store: Store[read]\n=\n"
                      "  @store.read(key)")
    main = join_decls(header(MOD, imports=[("app.edge", ["fetch"]), ("app.store", ["Store"])]),
                      "pub preview(key: Text) -> Text\n  uses @store: Store[read]\n=\n"
                      "  fetch(key) with @store = @store[read]")
    files = package("canary", "0.3", {"app.store": store, "app.edge": edge, MOD: main},
                    tables=[("boundaries", [("app.edge", ["Store[read]"])])])
    return Program(op="canary-reject-boundary", rule="the gate's own canary", edition="0.3",
                   seed=0, cls=MUST_REJECT, files=files, expect=["U3021"])


def canary_ceiling_control():
    """Q1's `ceiling` probe with a plain ceiling, `Log[write]`, as a must-reject row expecting
    U7002: the checks accept it, so the driver must call it a finding. It shows that in the probe's
    package everything but the ceiling's `where` resolves, so the probe's U7002 can come only from
    that condition (#813)."""
    modules, tables = conditions.ceiling_package("dry", "write")
    files = package("canary", "0.3", modules, tables=tables)
    return Program(op="canary-ceiling-control", rule="the gate's own canary", edition="0.3",
                   seed=0, cls=MUST_REJECT, files=files, expect=["U7002"])


def canary_oracle_condition():
    """An accepted program of edition 0.3 whose literal, public in the code and marked secret by
    its manifest, decides by the branch around it whether a conditioned public operation,
    `journal`, runs. It prints a constant, so its variants differ only in that operation's events,
    and the oracle must call it a leak at `public` through the trace alone (#813). The leak is the
    branch's, not the condition's: what the canary shows is that the oracle sees a difference of
    events alone in edition 0.3, and that its runs have the solver, without which the conditioned
    operation's obligation would make the canary an error (U5003)."""
    v = Varied()
    lit = v.lit("Int", "secret", "5")
    text = join_decls(
        header(MOD),
        "capability Catalog\n  op journal(n: Int) -> Unit\n\n"
        "handler book() for Catalog\n  op journal(n) = unit",
        "work(n: Int, dry: Bool) -> Unit\n  uses @cat: Catalog[journal where not dry]\n=\n"
        "  if not dry\n    @cat.journal(n)\n  unit",
        CONSOLE_MAIN + f"  let x: Int = {lit}\n  work(1, dry: x > 3) with @cat = book()\n"
        '  @console.print("done")')
    files, vary = v.place(package("canary", "0.3", {MOD: text}, entry=MOD, grants=["console"]))
    return Program(op="canary-oracle-condition", rule="the gate's own canary", edition="0.3",
                   seed=0, cls=OPEN, files=files, vary=vary)



VAULT = ("capability Vault\n  op note(x: Int) -> Unit\n    accepts secret\n\n"
         "handler quiet() for Vault\n  op note(x) = unit")


def canary_internal():
    """An accepted program that calls an operation accepting `secret` under a secret branch; the
    gate plants that operation as accepting `internal`. The `public` observer must see the same in
    every variant, and the `internal` observer the event come and go: a leak at `internal` only."""
    v = Varied()
    lit = v.lit("Bool", "secret", "true")
    text = join_decls(
        header(MOD), VAULT,
        "work(n: Int) -> Unit\n  uses @vault: Vault[note]\n=\n  @vault.note(n)",
        "guarded(flag: secret Bool) -> Unit\n=\n"
        "  if flag\n    work(1) with @vault = quiet()\n  else\n    unit",
        CONSOLE_MAIN + f"  let s: secret Bool = {lit}\n  guarded(s)\n"
        '  @console.print("done")')
    files, vary = v.place(package("canary", "0.2", {MOD: text}, entry=MOD, grants=["console"]))
    return Program(op="canary-internal", rule="the gate's own canary", edition="0.2", seed=0,
                   cls=OPEN, files=files, vary=vary)


def canary_redacted():
    """An accepted program that sends a secret to an operation accepting `secret`; the gate plants
    that operation as accepting `public`. Every variant shows the same `<redacted:secret>`
    argument, so only the redacted-sink rule can report it."""
    v = Varied()
    lit = v.lit("Int", "secret", "5")
    text = join_decls(
        header(MOD), VAULT,
        "send(n: secret Int) -> Unit\n  uses @vault: Vault[note]\n=\n  @vault.note(n)",
        CONSOLE_MAIN + f"  let s: secret Int = {lit}\n  send(s) with @vault = quiet()\n"
        '  @console.print("done")')
    files, vary = v.place(package("canary", "0.2", {MOD: text}, entry=MOD, grants=["console"]))
    return Program(op="canary-redacted", rule="the gate's own canary", edition="0.2", seed=0,
                   cls=OPEN, files=files, vary=vary)


def planted(level):
    """The canaries' planted fault: `Vault.note` accepts `level` in the oracle's table."""
    def plant(table):
        table = dict(table)
        table[("Vault", "note")] = {level}
        return table
    return plant


def oracle_canary(prog, una, work, plant=None):
    """Runs one program through the oracle, and returns its result row."""
    buf = io.StringIO()
    oracle.oracle([dict(prog.manifest(), digest=prog.digest())], una, work, RUN_TIMEOUT, buf,
                  lambda row: prog, plant=plant)
    return json.loads(buf.getvalue())


def _unexpected_syntax(codes, expect):
    return sorted(c for c in {c[0] for c in codes} if c.startswith("U0") and c not in expect)


def generator_errors(rows):
    """The must-reject driver's rows that show a generator's error rather than a verdict on the
    program: a lexical or syntax code (U0xxx) the program does not expect, which no generated
    program should have; a must-reject program with no expected code, which any rejection would
    satisfy; or a must-reject program rejected without any of its expected codes. The codes are
    matched by code, not by place."""
    out = []
    for r in rows:
        codes = {c[0] for c in r.get("codes", [])}
        unexpected = _unexpected_syntax(r.get("codes", []), r.get("expect", []))
        if unexpected:
            out.append(f"reject {r['name']}: a generator error, {', '.join(unexpected)} on a "
                       "generated program")
        elif r.get("class") == MUST_REJECT and not r.get("expect"):
            out.append(f"reject {r['name']}: a must-reject program with no expected code")
        elif (r.get("class") == MUST_REJECT or (r.get("class") == OPEN and r.get("expect"))) \
                and r.get("verdict") == "rejected" and r.get("expect_seen") is False:
            out.append(f"reject {r['name']}: rejected, but by {', '.join(sorted(codes))}, not its "
                       f"expected {', '.join(r.get('expect', []))}")
    return out


def keep_generator_errors(rows):
    """The must-keep driver's pairs whose base or refactored twin holds a lexical or syntax code
    the program does not expect: a refactor that breaks parsing would make a vacuous pair."""
    out = []
    for r in rows:
        for side in ("base", "twin"):
            unexpected = _unexpected_syntax(r[side].get("codes", []), r.get("expect", []))
            if unexpected:
                out.append(f"keep {r['name']} {r['refactor']} {side}: a generator error, "
                           f"{', '.join(unexpected)}")
    return out


def canary_syntax():
    """An open row, expecting nothing, whose program does not parse: the gate must call it a
    generator error by its code alone, never a rejection that counts or a program set aside."""
    text = join_decls(header(MOD), CONSOLE_MAIN + "  let x = = 1\n  @console.print(\"x\")")
    files = package("canary", "0.2", {MOD: text}, entry=MOD, grants=["console"])
    return Program(op="canary-syntax", rule="the gate's own canary", edition="0.2", seed=0,
                   cls=OPEN, files=files)


def caught(name, row):
    """Whether a canary's result is the fault it plants, and only that."""
    if row.get("error") or row.get("set_aside") or not row.get("finding"):
        return False
    if name == "native":
        # Divergences on the variants built from the base's files, and none on the base itself,
        # whose executable is its own: the comparison tells the two apart.
        record = row.get("native", {})
        diverged = [d["variant"] for d in record.get("divergences", [])]
        return (bool(diverged) and not record.get("not_compiled")
                and not any(v.endswith("-base") for v in diverged))
    leaks = sorted({o for v in row["variants"] for o in v.get("observers", [])})
    if name in ("oracle", "oracle-tasks", "oracle-condition"):
        return "public" in leaks
    if name == "internal":
        return leaks == ["internal"] and not row["violations"]
    if name == "redacted":
        return bool(row["violations"]) and not leaks
    return False


def native_strata(rows):
    """The oracle's results that join the native sample as their family's stratum: for every family
    with a compared program, the compared program whose name has the lowest SHA-256, when the
    hashed part of the sample does not hold it already; in family order."""
    lowest = {}
    for r in rows:
        if r.get("compared", 0) > 0 and not r.get("error"):
            family = family_of(r["op"])
            if family not in lowest or native.key(r["name"]) < native.key(lowest[family]["name"]):
                lowest[family] = r
    return [r for _, r in sorted(lowest.items()) if not r["native"]["sampled"]]


NOTE = ("capability Note\n  op note(x: Int) -> Unit\n\n"
        "handler quiet() for Note\n  op note(x) = unit")
SHOW = ("show(line: Text) -> Int\n  uses @console: Console[print]\n=\n"
        "  @console.print(line)\n  1")
SHOW_TWICE = ("show(line: Text) -> Int\n  uses @console: Console[print]\n=\n"
              "  @console.print(line)\n  @console.print(line)\n  1")
DOWN = "down(n: Int) -> Int = if n == 0 then 0 else down(n - 1) + 1"


# The sandbox canary's probe (#793): run under the sandbox, it writes in its own directory, in a
# sibling run's and in the system's temporary directory, reads a file of the sibling's, connects
# to and binds a loopback port, signals its parent and connects to the parent's abstract Unix
# socket; it prints what each attempt did.
SANDBOX_PROBE = r'''
import errno, json, os, socket, sys
own, sibling, secret, outside, port, abstract, key = sys.argv[1:8]
def name(e):
    return errno.errorcode.get(e.errno, type(e).__name__)
def write(path):
    try:
        with open(path, "w") as f:
            f.write("x")
        return "written"
    except OSError as e:
        return name(e)
def read(path):
    try:
        with open(path, "rb") as f:
            f.read()
        return "read"
    except OSError as e:
        return name(e)
def connect(p):
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.settimeout(5)
    try:
        s.connect(("127.0.0.1", int(p)))
        return "connected"
    except OSError as e:
        return name(e)
    finally:
        s.close()
def bind():
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        s.bind(("127.0.0.1", 0))
        return "bound"
    except OSError as e:
        return name(e)
    finally:
        s.close()
def signal_parent():
    try:
        os.kill(os.getppid(), 0)
        return "sent"
    except OSError as e:
        return name(e)
def unix(path):
    if path == "-":
        return "unsupported"
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        s.connect("\0" + path)
        return "connected"
    except OSError as e:
        return name(e)
    finally:
        s.close()
print(json.dumps({"own": write(own), "sibling": write(sibling), "sibling_read": read(secret),
                  "key": write(key) if key != "-" else "none",
                  "key_read": read(key) if key != "-" else "none",
                  "outside": write(outside), "socket": connect(port), "bind": bind(),
                  "signal": signal_parent(), "abstract": unix(abstract)}))
'''
# What each attempt of the probe must give, by backend, when the run has no mock port. Landlock
# denies files and TCP with EACCES and scopes signals and abstract sockets with EPERM;
# sandbox-exec denies writes, the network and signals to other processes with EPERM, and does
# not confine reads (abstract sockets do not exist on macOS).
SANDBOX_EXPECTED = {
    "landlock": {"own": "written", "sibling": "EACCES", "sibling_read": "EACCES",
                 "key": "EACCES", "key_read": "read",
                 "outside": "EACCES", "socket": "EACCES", "bind": "EACCES", "signal": "EPERM",
                 "abstract": "EPERM"},
    "sandbox-exec": {"own": "written", "sibling": "EPERM", "sibling_read": "read",
                     "key": "EPERM", "key_read": "read",
                     "outside": "EPERM", "socket": "EPERM", "bind": "EPERM", "signal": "EPERM",
                     "abstract": "unsupported"},
}


def sandbox_canary(work):
    """The sandbox canary (#793), on every platform: a run writes in its own directory, and gets
    the sandbox's refusal for every attempt of `SANDBOX_EXPECTED` on its backend: a write in a
    sibling run's directory or in the system's temporary one, a read of the sibling's file, a
    connection to a loopback port and a bind, and on Linux a signal to its parent and a
    connection to the parent's abstract Unix socket. A listener outside the sandbox holds the
    port open, so a refused connection is the sandbox's doing, not a closed port's; a second run,
    given that port as its mock's, must connect. Returns (caught, row)."""
    base = sandbox.fresh_dir(os.path.join(work, "sandbox-canary"), "c-")
    own, sibling = sandbox.fresh_dir(base, "own-"), sandbox.fresh_dir(base, "sibling-")
    secret = os.path.join(sibling, "secret")
    with open(secret, "w", encoding="utf-8") as f:
        f.write("the sibling's\n")
    outside = os.path.join("/tmp", f"una-hunt-canary-{os.getpid()}-{os.path.basename(base)}")
    kind = sandbox.backend()
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.bind(("127.0.0.1", 0))
    listener.listen(4)
    port = listener.getsockname()[1]
    abstract, unix = "-", None
    if kind == "landlock":
        abstract = f"una-hunt-canary-{os.getpid()}-{os.path.basename(base)}"
        unix = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        unix.bind("\0" + abstract)
        unix.listen(4)
    try:
        def probe(mock_port):
            r = sandbox.run([sys.executable, "-I", "-c", SANDBOX_PROBE,
                             os.path.join(own, "file"), os.path.join(sibling, "file"), secret,
                             outside, str(port), abstract,
                             sandbox.PROOF_KEY[0] if sandbox.PROOF_KEY else "-"],
                            cwd=own, writable=own,
                            timeout=CHECK_TIMEOUT, mock_port=mock_port)
            try:
                return json.loads(r.stdout)
            except ValueError:
                return {"unreadable": (r.stdout + r.stderr)[-2000:], "exit": r.code}
        denied, allowed = probe(None), probe(port)
    finally:
        listener.close()
        if unix is not None:
            unix.close()
    leaked = os.path.exists(outside) or os.path.exists(os.path.join(sibling, "file"))
    if os.path.exists(outside):
        os.remove(outside)
    expected = SANDBOX_EXPECTED.get(kind)
    caught = (expected is not None and denied == expected
              and allowed.get("socket") == "connected" and not leaked)
    return caught, {"backend": kind, "denied": denied, "expected": expected, "allowed": allowed,
                    "leaked": leaked}


def native_pairs():
    """The native comparison's own canaries: for each, the files of a program the interpreter runs
    and those of the one the native executable is built from. Each program against itself must
    agree, and against the other must be a divergence:
    - `native-trace`: the two differ in the argument of an event only, never in output;
    - `native-tasks`: a block of `together` whose tasks print lines with braces and quotes, which
      the trace escapes (`una_core::quote`); the other prints another line in one task;
    - `native-task-extra`: an ordered block of `together`; the other's tasks each print their
      line once more, an extra effect that an ordered run may not have;
    - `native-abort`: a recursion past `--max-depth` (U6004); the other prints another line
      before it."""
    def files(body, *decls):
        text = join_decls(header(MOD), *decls, CONSOLE_MAIN + body)
        return package("canary", "0.2", {MOD: text}, entry=MOD, grants=["console"])

    def trace(n):
        return files(f"  work({n}) with @note = quiet()\n  @console.print(\"done\")", NOTE,
                     "work(n: Int) -> Unit\n  uses @note: Note[note]\n=\n  @note.note(n)")

    def tasks(line):
        return files("  together\n    let a = show(\"left \\{1\\} \\\"q\\\"\")\n"
                     f"    let b = show(\"{line}\")\n  @console.print(\"{{a + b}}\")", SHOW)

    def extra(twice):
        body = SHOW_TWICE if twice else SHOW
        return files("  together\n    let a = show(\"left\")\n    let b = show(\"right\")\n"
                     '  @console.print("{a + b}")', body)

    def abort(line):
        return files(f"  @console.print(\"{line}\")\n  let d = down(5000)\n"
                     "  @console.print(\"{d}\")", DOWN)

    return {"native-trace": (trace(5), trace(6)), "native-tasks": (tasks("right"), tasks("other")),
            "native-task-extra": (extra(False), extra(True)),
            "native-abort": (abort("before"), abort("other"))}


def expected_keep(progs):
    """How many (program, refactor) pairs the must-keep driver judges: the refactors that apply."""
    n = 0
    for p in progs:
        for rid in REFACTOR_IDS:
            if REFACTORS[rid](p.files, rng_for(derive_seed(p.seed, rid, 0))) is not None:
                n += 1
    return n


def _sharded(fn, rows, workers):
    """Runs `fn(rows_part, out, k)` on `workers` contiguous parts of `rows` at once, part `k` in a
    directory of its own, and returns the joined output lines in row order and the summed tally."""
    size = max(1, -(-len(rows) // workers))
    parts = [rows[i:i + size] for i in range(0, len(rows), size)]

    def one(k):
        buf = io.StringIO()
        return fn(parts[k], buf, k), buf.getvalue()

    with ThreadPoolExecutor(max_workers=workers) as pool:
        done = list(pool.map(one, range(len(parts))))
    tally = Counter()
    lines = []
    for t, text in done:
        tally.update(t)
        lines += [json.loads(line) for line in text.splitlines() if line.strip()]
    return tally, lines


def reading(row):
    """What the re-check did with a program it read: (`fallback`, reason) when every line is
    `not re-checked` for one reason, (`compared`, None) when at least one of the program's own
    (`app.*`) functions is accepted, else (`unread`, None)."""
    lines = row.get("recheck") or []
    if lines and all(x["outcome"] == "not re-checked" for x in lines) \
            and len({x["note"] for x in lines}) == 1:
        return "fallback", lines[0]["note"]
    if any(x["decl"].startswith("app.") and x["outcome"] in ACCEPTED for x in lines):
        return "compared", None
    return "unread", None


def differ_canary(core_check, work):
    """Whether the re-check comparison sees planted faults, and only them: each document of
    DIFFER_CANARIES, read as a program's export through `judge_differ`, must be a finding whose
    rejected declaration is `app.main.main` for its reason, and DIFFER_CONTROL must read clean,
    with `app.main.main` accepted."""
    repo = os.path.realpath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))

    def judged(name):
        path = os.path.join(repo, REPLAY, name)
        try:
            with open(path, encoding="utf-8") as f:
                text = f.read()
        except OSError:
            return None
        # The document is copied into the run's own directory: the run reads nothing of the
        # checkout's (#793).
        own = sandbox.fresh_dir(work, "differ-canary-")
        try:
            copy = os.path.join(own, name)
            with open(copy, "w", encoding="utf-8") as f:
                f.write(text)
            rec = sandbox.run([core_check, copy], cwd=own, writable=own, timeout=CHECK_TIMEOUT,
                              env_extra=drivers.RECHECK_ENV)
        finally:
            sandbox.remove_dir(own, work)
        return drivers.judge_differ(sandbox.Run(0, text, "", False, 0.0), text, rec)

    for name, note in DIFFER_CANARIES.items():
        j = judged(name)
        if j is None or j["kind"] != "finding":
            return False
        rejected = [x for x in j["recheck"] if x["outcome"] == "rejected"]
        if [(x["decl"], x["note"]) for x in rejected] != [("app.main.main", note)]:
            return False
    control = judged(DIFFER_CONTROL)
    return control is not None and control["kind"] == "clean" and any(
        x["decl"] == "app.main.main" and x["outcome"] in ACCEPTED for x in control["recheck"])


def _version(una):
    try:
        p = subprocess.run([una, "--version"], capture_output=True, text=True, timeout=30)
        return p.stdout.strip()
    except (OSError, subprocess.SubprocessError) as e:
        return f"unknown ({e})"


def run(una, out_dir, tip, core_check, z3):
    """Runs the gate. Writes OUT/manifest.jsonl, reject.jsonl, keep.jsonl, oracle.jsonl,
    canaries.jsonl and summary.json, prints the summary, and returns 0 (PASS) or 1 (FAIL)."""
    if os.path.exists(out_dir) and os.listdir(out_dir):
        raise SystemExit(f"hunt gate: {out_dir} is not empty; the gate writes a fresh directory")
    started = time.monotonic()
    # No run starts before the sandbox is verified, and on Linux each run may read the tools'
    # directories besides its own.
    sandbox.require()
    sandbox.READABLE[:] = [os.path.dirname(os.path.realpath(p)) for p in (una, core_check, z3)
                           if p]
    sandbox.OUT_ROOT[:] = [os.path.realpath(out_dir)]
    key_home = sandbox.make_proof_key()
    atexit.register(sandbox.drop_proof_key, key_home)
    root = drivers.campaign_root(out_dir)
    # `una run` and `una build` do the full check: a conditioned operation's obligations need the
    # pinned solver (#813).
    native.RUN_ENV.clear()
    if z3:
        native.RUN_ENV.update(drivers.solver_env(z3))
    progs = programs()
    by_name = {p.name: p for p in progs}
    if len(by_name) != len(progs):
        raise SystemExit("two programs of the fixed run share a name")
    write_manifest(os.path.join(root, "manifest.jsonl"), progs)
    rows = drivers.read_manifest(os.path.join(root, "manifest.jsonl"))
    work = os.path.join(root, "work")
    os.makedirs(work, exist_ok=True)

    def shard(k):
        """The writable directory of part `k`: concurrent runs never share a home or a package."""
        path = os.path.join(work, f"part{k}")
        os.makedirs(path, exist_ok=True)
        return path
    errors, findings, notes = [], [], []

    # Must-reject.
    judged = [r for r in rows if r["class"] in (MUST_REJECT, MUST_ACCEPT, OPEN, RESIDUAL)]
    reject_tally, reject_rows = _sharded(
        lambda part, buf, k: drivers.must_reject(part, una, shard(k), CHECK_TIMEOUT, buf), judged,
        WORKERS)
    errors.extend(generator_errors(reject_rows))
    for r in reject_rows:
        if r["verdict"] not in ("accepted", "rejected"):
            errors.append(f"reject {r['name']}: {r['verdict']} {r.get('detail', '')[:200]}")
        if r.get("finding"):
            findings.append(f"reject {r['name']}: " + ("a must-reject program is accepted"
                            if r.get("class") == MUST_REJECT else
                            "a closed call, which a correct checker rejects, is accepted"))
        if r.get("over_strict"):
            notes.append(f"reject {r['name']}: a must-accept program is rejected")

    # Must-keep.
    keep_tally, keep_rows = _sharded(
        lambda part, buf, k: drivers.must_keep(part, una, shard(k), CHECK_TIMEOUT, buf,
                                                  REFACTOR_IDS),
        rows, WORKERS)
    errors.extend(keep_generator_errors(keep_rows))
    for r in keep_rows:
        for side in ("base", "twin"):
            if r[side]["verdict"] not in ("accepted", "rejected"):
                errors.append(f"keep {r['name']} {r['refactor']} {side}: {r[side]['verdict']}")
        if r.get("direction") == "rejected-to-accepted":
            findings.append(f"keep {r['name']} {r['refactor']}: rejected to accepted")
        elif r.get("direction") == "accepted-to-rejected":
            notes.append(f"keep {r['name']} {r['refactor']}: accepted to rejected")

    # The oracle.
    varied = [r for r in rows if r.get("vary")]
    oracle_tally, oracle_rows = _sharded(
        lambda part, buf, k: oracle.oracle(part, una, shard(k), RUN_TIMEOUT, buf,
                                           drivers.rebuild),
        varied, WORKERS)
    # The native sample's strata: for every family with a compared program, its compared program
    # of lowest hash joins the sample, when the hashed part does not hold it already.
    by_name = {r["name"]: r for r in varied}
    for r in native_strata(oracle_rows):
        oracle.native_pass(drivers.rebuild(by_name[r["name"]]), una, shard(0), RUN_TIMEOUT, r)
        if r["native"]["divergences"]:
            r["finding"] = True
    termination = []
    # Runs killed at their time limit whose full check, run on its own, decided
    # (`native.undecided`): each removes a comparison, so their count has a ceiling.
    killed = timeouts_decided(oracle_rows)
    if killed > MAX_TIMEOUTS_DECIDED:
        errors.append(f"oracle: {killed} runs killed at their time limit, their full check "
                      f"deciding, and the gate allows {MAX_TIMEOUTS_DECIDED}")
    for r in oracle_rows:
        if r.get("error"):
            errors.append(f"oracle {r['name']}: {r['error']}")
        if r.get("finding"):
            seen = sorted({o for v in r["variants"] for o in v.get("observers", [])})
            if seen:
                findings.append(f"oracle {r['name']}: an observer at {', '.join(seen)} sees the "
                                "varied literal")
            for v in r.get("violations", []):
                findings.append(f"oracle {r['name']}: {v}")
        for t in r.get("termination", []):
            termination.append(dict(t, name=r["name"]))
    # The native sample: divergences are findings; a program not compiled must be pinned by name.
    pinned_unsupported = native.unsupported()
    native_rows = [r for r in oracle_rows if r.get("native", {}).get("sampled")]
    native_compared = sum(1 for r in native_rows if r["native"]["compared"] > 0)
    native_families = sorted({family_of(r["op"]) for r in native_rows
                              if r["native"]["compared"] > 0})
    native_variants = sum(r["native"]["compared"] for r in native_rows)
    not_compiled = {r["name"]: r["native"]["not_compiled"] for r in native_rows
                    if r["native"]["not_compiled"]}
    for r in native_rows:
        for d in r["native"]["divergences"]:
            findings.append(f"native {r['name']} {d['variant']}: backend divergence: "
                            + "; ".join(d["differences"]))
    for name in sorted(set(not_compiled) - set(pinned_unsupported)):
        errors.append(f"native {name}: the native backend does not compile it, and "
                      "scripts/security-hunt/native-unsupported.txt does not pin it: "
                      f"{not_compiled[name][:200]}")
    # A sampled program that is not compared says why: the oracle set it aside (a policy that
    # releases to public, or a program the checks reject, so nothing runs), or it is pinned as not
    # compiled. Any other is an error: a sample that silently compares less.
    native_not_compared = []
    for r in native_rows:
        if r["native"]["compared"] > 0:
            continue
        why = (r.get("set_aside") or
               (f"not compiled: {r['native']['not_compiled'][:120]}" if r["native"]["not_compiled"]
                else None))
        if why is None:
            errors.append(f"native {r['name']}: sampled, but no variant was compared natively"
                          + (f" ({r['error'][:200]})" if r.get("error") else ""))
        native_not_compared.append({"name": r["name"], "why": why or "not compared"})
    for name in sorted(set(pinned_unsupported) - set(not_compiled)):
        errors.append(f"native {name}: pinned as not compiled, but it compiles or is not sampled; "
                      "remove it from scripts/security-hunt/native-unsupported.txt")
    # Only complete-against-complete comparisons count toward the floors.
    compared = sum(r.get("compared", 0) for r in oracle_rows)
    programs_compared = sum(1 for r in oracle_rows if r.get("compared", 0) > 0)
    live = sum(1 for r in oracle_rows if r.get("live"))

    # The re-check comparison (part (b) of docs/plan/security-hunt.md, #791): every program the
    # checks accept is exported and re-checked by the model's checker.
    differ_tally, differ_rows = _sharded(
        lambda part, buf, k: drivers.differ(part, una, core_check, shard(k), CHECK_TIMEOUT, buf,
                                            z3=z3),
        rows, WORKERS)
    unknown_seen = {}
    for r in differ_rows:
        if r["verdict"] not in ("accepted", "rejected"):
            errors.append(f"differ {r['name']}: {r['verdict']} {str(r.get('detail', ''))[:200]}")
        if r.get("finding"):
            findings.append(f"differ {r['name']}: {r.get('differ_detail') or 'the re-check rejects'}")
        if r.get("differ") == "unknown":
            unknown_seen[r["name"]] = r.get("differ_detail") or ""
        if r["class"] == RESIDUAL and r["verdict"] == "accepted":
            row_errors, row_findings, row_notes = residual_verdicts(r)
            errors += row_errors
            findings += row_findings
            notes += row_notes
    for name in sorted(set(unknown_seen) - set(PINNED_UNKNOWN)):
        errors.append(f"differ {name}: an unknown that is not pinned: {unknown_seen[name]}")
    for name in sorted(set(PINNED_UNKNOWN) - set(unknown_seen)):
        errors.append(f"differ {name}: a pinned unknown is no longer unknown")
    for name in sorted(set(PINNED_UNKNOWN) & set(unknown_seen)):
        if unknown_seen[name] != PINNED_UNKNOWN[name]:
            errors.append(f"differ {name}: unknown for another reason: {unknown_seen[name]}")
    differ_clean = sum(1 for r in differ_rows if r.get("differ") == "clean")
    differ_residual = sum(1 for r in differ_rows if r.get("differ") == "residual")
    # A residual row's own comparison must be clean: an export or re-check failure there is an
    # error, never hidden by the row's class.
    for r in differ_rows:
        if r.get("differ") == "residual" and r.get("differ_raw") != "clean":
            errors.append(f"differ {r['name']}: the residual row's comparison is "
                          f"{r.get('differ_raw')!r}: {r.get('differ_detail') or ''}")
    # What was compared: a clean row counts only when the re-check accepted one of the program's
    # own functions; whole-document fallbacks and declarations not re-checked are pinned.
    compared_rows, fallbacks, not_rechecked, accepted_decls = 0, Counter(), Counter(), 0
    for r in differ_rows:
        if r.get("differ") != "clean":
            continue
        how, why = reading(r)
        if how == "fallback":
            fallbacks[why] += 1
            continue
        compared_rows += how == "compared"
        for x in r.get("recheck") or []:
            if not x["decl"].startswith("app."):
                continue
            if x["outcome"] in ACCEPTED:
                accepted_decls += 1
            elif x["outcome"] == "not re-checked":
                not_rechecked[x["note"]] += 1
    if compared_rows < MIN_DIFFER_COMPARED:
        errors.append(f"differ: {compared_rows} programs compared, under the floor of "
                      f"{MIN_DIFFER_COMPARED}")
    if accepted_decls < MIN_ACCEPTED_DECLARATIONS:
        errors.append(f"differ: {accepted_decls} declarations accepted by the re-check, under the "
                      f"floor of {MIN_ACCEPTED_DECLARATIONS}")
    for name, pinned in (("whole-document fallbacks", PINNED_FALLBACK),
                         ("declarations not re-checked", PINNED_NOT_RECHECKED)):
        got = fallbacks if pinned is PINNED_FALLBACK else not_rechecked
        for why in sorted(set(got) | set(pinned)):
            if got.get(why, 0) != pinned.get(why, 0):
                errors.append(f"differ: {got.get(why, 0)} {name} for {why!r}, and the gate pins "
                              f"{pinned.get(why, 0)}")
    if differ_residual != PINNED_RESIDUAL:
        errors.append(f"differ: {differ_residual} residual programs compared, and the gate pins "
                      f"{PINNED_RESIDUAL}")
    # The two drivers' tier-0 verdicts must agree, program by program.
    tier0 = {r["name"]: r["verdict"] for r in reject_rows}
    for r in differ_rows:
        if r["name"] in tier0 and tier0[r["name"]] != r["verdict"]:
            errors.append(f"differ {r['name']}: tier 0 said {tier0[r['name']]!r} to must-reject "
                          f"and {r['verdict']!r} here")

    # Canaries.
    canaries = {}
    canaries["differ"] = differ_canary(core_check, work)
    crej = canary_reject()
    buf = io.StringIO()
    drivers.must_reject([dict(crej.manifest(), digest=crej.digest())], una, work, CHECK_TIMEOUT,
                        buf, build=lambda row: crej)
    canaries["reject"] = bool(json.loads(buf.getvalue()).get("finding"))
    cbound = canary_reject_boundary()
    buf = io.StringIO()
    drivers.must_reject([dict(cbound.manifest(), digest=cbound.digest())], una, work,
                        CHECK_TIMEOUT, buf, build=lambda row: cbound)
    canaries["reject-boundary"] = bool(json.loads(buf.getvalue()).get("finding"))
    cceil = canary_ceiling_control()
    buf = io.StringIO()
    drivers.must_reject([dict(cceil.manifest(), digest=cceil.digest())], una, work,
                        CHECK_TIMEOUT, buf, build=lambda row: cceil)
    canaries["ceiling-control"] = bool(json.loads(buf.getvalue()).get("finding"))
    csyn = canary_syntax()
    buf = io.StringIO()
    drivers.must_reject([dict(csyn.manifest(), digest=csyn.digest())], una, work, CHECK_TIMEOUT,
                        buf, build=lambda row: csyn)
    canaries["syntax"] = bool(generator_errors([json.loads(buf.getvalue())]))
    for name, (a, b) in native_pairs().items():
        same = native.pair(una, work, RUN_TIMEOUT, oracle.RUN_LIMITS, a, a, f"{name}-same")
        other = native.pair(una, work, RUN_TIMEOUT, oracle.RUN_LIMITS, a, b, f"{name}-other")
        canaries[name] = same == [] and bool(other)
    native_canary = canary_oracle()
    buf = io.StringIO()
    oracle.oracle([dict(native_canary.manifest(), digest=native_canary.digest())], una, work,
                  RUN_TIMEOUT, buf, lambda row: native_canary,
                  native_plant=lambda files: native_canary.files)
    canary_rows = {
        "native": json.loads(buf.getvalue()),
        "oracle": oracle_canary(canary_oracle(), una, work),
        "oracle-tasks": oracle_canary(canary_oracle_tasks(), una, work),
        "oracle-condition": oracle_canary(canary_oracle_condition(), una, work),
        "internal": oracle_canary(canary_internal(), una, work, planted("internal")),
        "redacted": oracle_canary(canary_redacted(), una, work, planted("public")),
    }
    for name, row in canary_rows.items():
        canaries[name] = caught(name, row)
    canaries["sandbox"], canary_rows["sandbox"] = sandbox_canary(work)
    try:
        sandbox.check_proof_key(full=True)
    except sandbox.SandboxUnavailable as e:
        errors.append(f"sandbox: {e}")
    with open(os.path.join(root, "canaries.jsonl"), "w", encoding="utf-8") as f:
        f.write(json.dumps({"caught": canaries, "rows": canary_rows}, sort_keys=True) + "\n")

    for name, lines in (("reject", reject_rows), ("keep", keep_rows), ("oracle", oracle_rows),
                        ("differ", differ_rows)):
        with open(os.path.join(root, f"{name}.jsonl"), "w", encoding="utf-8") as f:
            for line in lines:
                f.write(json.dumps(line, sort_keys=True) + "\n")

    def forms(pick):
        return sorted({f"{r['op']}:{r['name'].split('-', 3)[3] if r['name'].count('-') >= 3 else ''}"
                       for r in oracle_rows if pick(r)})
    compared_forms = forms(lambda r: r.get("compared", 0) > 0)
    live_forms = forms(lambda r: r.get("live"))
    for label, got, pinned in (("compares", compared_forms, ORACLE_FORMS),
                               ("finds live", live_forms, LIVE_FORMS)):
        if tuple(got) != tuple(pinned):
            errors.append(f"oracle: the forms it {label} differ from the pinned ones: missing "
                          f"{sorted(set(pinned) - set(got))}, new {sorted(set(got) - set(pinned))}")
    set_aside_rejected = sum(1 for r in oracle_rows
                             if str(r.get("set_aside", "")).startswith("rejected by the checks"))
    errors += oracle_family_errors(oracle_rows)
    sizes = {
        "programs": len(progs),
        "reject": {"expected": len(judged), "judged": len(reject_rows)},
        "keep": {"expected": expected_keep(progs), "judged": len(keep_rows)},
        "oracle": {"expected": len(varied), "judged": len(oracle_rows),
                   "set_aside": sum(1 for r in oracle_rows if r.get("set_aside")),
                   "programs_compared": programs_compared, "variants_compared": compared,
                   "live": live, "compared_forms": compared_forms, "live_forms": live_forms,
                   "set_aside_rejected": set_aside_rejected,
                   "timeouts_decided": killed},
        "differ": {"expected": len(rows), "judged": len(differ_rows), "clean": differ_clean,
                   "compared": compared_rows, "accepted_declarations": accepted_decls,
                   "fallbacks": dict(sorted(fallbacks.items())),
                   "not_rechecked": dict(sorted(not_rechecked.items())),
                   "unknown": len(unknown_seen), "residual": differ_residual},
        "native": {"expected": NATIVE_SAMPLE, "sampled": len(native_rows),
                   "programs_compared": native_compared, "variants_compared": native_variants,
                   "families": native_families,
                   "not_compiled": sorted(not_compiled),
                   "not_compared": native_not_compared,
                   "skipped": [dict(s, name=r["name"]) for r in native_rows
                               for s in r["native"]["skipped"]]},
    }
    for key in ("reject", "keep", "oracle", "differ"):
        s = sizes[key]
        if s["judged"] != s["expected"]:
            errors.append(f"{key}: judged {s['judged']} of {s['expected']} cases")
        if s["judged"] == 0:
            errors.append(f"{key}: no case judged")
    if compared == 0:
        errors.append("oracle: no variant compared")
    if compared < MIN_VARIANTS_COMPARED:
        errors.append(f"oracle: {compared} variants compared, under the floor of "
                      f"{MIN_VARIANTS_COMPARED}")
    if live < MIN_LIVE:
        errors.append(f"oracle: {live} programs whose variants differ at the literal's level, "
                      f"under the floor of {MIN_LIVE}")
    if programs_compared < MIN_ORACLE_COMPARED:
        errors.append(f"oracle: {programs_compared} programs compared, under the floor of "
                      f"{MIN_ORACLE_COMPARED}")
    if len(native_rows) != NATIVE_SAMPLE:
        errors.append(f"native: the sample has {len(native_rows)} programs, and the gate pins "
                      f"{NATIVE_SAMPLE}")
    if tuple(native_families) != tuple(NATIVE_FAMILIES):
        errors.append(f"native: the families compared natively are {', '.join(native_families)}, "
                      f"and the gate pins {', '.join(NATIVE_FAMILIES)}")
    if native_variants < MIN_NATIVE_VARIANTS:
        errors.append(f"native: {native_variants} variants compared natively, under the floor of "
                      f"{MIN_NATIVE_VARIANTS}")
    if native_compared < MIN_NATIVE_COMPARED:
        errors.append(f"native: {native_compared} programs compared natively, under the floor of "
                      f"{MIN_NATIVE_COMPARED}")
    for key, want in PINNED.items():
        got = sizes[key] if key == "programs" else sizes[key]["expected"]
        if got != want:
            errors.append(f"{key}: the fixed run has {got} cases, and the gate pins {want}")
    for name, seen in sorted(canaries.items()):
        if not seen:
            errors.append(f"canary {name}: not caught, so the gate cannot see that fault")

    elapsed = time.monotonic() - started
    if budget_error(elapsed):
        errors.append(budget_error(elapsed))
    verdict = "PASS" if not errors and not findings else "FAIL"
    summary = {
        "verdict": verdict, "seed": SEED, "count_per_operator": COUNT,
        "editions": list(EDITIONS),
        "operator_editions": {op: list(e) for op, e in sorted(OPERATOR_EDITIONS.items())},
        "sample": {"edition": SAMPLE_EDITION, "first_per_operator": SAMPLE_COUNT,
                   "and": "the first program of each further form of its 0.2 programs, and of "
                          "each form only its programs of another edition have"},
        "refactors": list(REFACTOR_IDS), "tip": tip,
        "una": _version(una), "sizes": sizes, "canaries": canaries,
        "findings": findings, "errors": errors,
        "termination_pairs": {"count": len(termination), "pairs": termination},
        "notes": notes,
        "tallies": {
            "reject": {" ".join(map(str, k)): n for k, n in sorted(reject_tally.items())},
            "keep": {" ".join(map(str, k)): n for k, n in sorted(keep_tally.items())},
            "oracle": dict(sorted(oracle_tally.items())),
            "differ": {" ".join(map(str, k)): n for k, n in sorted(differ_tally.items())},
        },
    }
    with open(os.path.join(root, "summary.json"), "w", encoding="utf-8") as f:
        json.dump(summary, f, indent=1, sort_keys=True)
        f.write("\n")
    print(f"hunt gate: seed {SEED}, {len(progs)} programs, una tip {tip}")
    print(f"  time: {elapsed:.0f} seconds, budget {TIME_BUDGETS.get(sandbox.backend())} "
          f"({sandbox.backend()})")
    for key in ("reject", "keep", "oracle", "differ"):
        print(f"  {key}: {sizes[key]['judged']} of {sizes[key]['expected']} cases judged")
    print(f"  oracle: {programs_compared} programs and {compared} variants compared (complete "
          f"against complete), {live} live, {sizes['oracle']['set_aside']} programs set aside")
    print(f"  native: {native_compared} of {len(native_rows)} sampled programs and "
          f"{native_variants} variants compared, {len(not_compiled)} not compiled (pinned)")
    for item in native_not_compared:
        print(f"    not compared: {item['name']}: {item['why']}")
    for item in sizes["native"]["skipped"]:
        print(f"    variant not compared: {item['variant']}: {item['why']}")
    print(f"  canaries: " + ", ".join(f"{k} {'caught' if v else 'MISSED'}"
                                      for k, v in sorted(canaries.items())))
    print(f"  termination pairs (outside T6, recorded): {len(termination)}")
    print(f"  runs killed at their time limit, their full check deciding: {killed}")
    for t in termination:
        print(f"    {t['name']} value {t['value']}: {t['base']} vs {t['variant']}")
    print(f"  notes (over-strict, not failures): {len(notes)}")
    for line in findings:
        print("  FINDING " + line)
    for line in errors:
        print("  ERROR " + line)
    print(f"hunt gate: {verdict}")
    return 0 if verdict == "PASS" else 1
