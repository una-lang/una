"""Family W: tasks and time over code that reads secrets (ADR 0117 S8, `spec/27`, #973).

`within`, `first of`, `tasks` and the limits of `std.limit`, edition 0.4, on code whose work a
secret decides. Each program has a `main` and one secret literal the two-run oracle varies. The
accepted forms keep every timing outcome at the secret's label: the time-out of a `within` and the
value of a `first of` carry `τ` (`spec/27 §5` rule 8, §6 rule 8), a started task's check points
are not decided by the secret (§8 rule 5), and the statement a `limiter` scopes over is timed by
data at most `internal` (§10 rule 4). The closed forms, open programs with an expected code, are
the same programs with that label missed: a correct checker rejects them, and one that accepted
them would let the oracle see the secret.

`hunt timing` runs them through the oracle under the cost clock (`una run --virtual-time
--cost-clock`, `spec/27 §8` rule 6): each check point moves the virtual clock by one millisecond, so
the secret's work decides whether a deadline passes or a token is left, and a missing timing
label turns into a difference of values. Under the plain virtual clock no deadline would pass and
these programs would test nothing.

The family is not part of the release gate's fixed run (seed 767): its operators are in
`TIMING_OPERATORS`, apart from `OPERATORS`, which the gate runs; `hunt gen --ops W` and
`hunt repro --op W1` reach them.
Every loop on the secret is bounded (`count` clamps to `CLAMP`), so a varied value as large as
2^53 never makes a run long.
"""

from ..itaty import lock_text
from ..packages import package_files
from ..program import OPEN, Program, rng_for
from ..una import Varied, header, join_decls

MOD = "main"
RULE = "spec/27 §5, §6, §7, §10"
EDITION = "0.4"
EDITIONS = (EDITION,)

# The most iterations `count` makes: above every deadline, so a secret can still decide a
# time-out, and small enough that a run outside a `within` stays short.
CLAMP = 200

COUNT = (f"count(n: secret Int) -> secret Int\n=\n  let m = if n > {CLAMP} then {CLAMP} else n\n"
         "  let edit acc: secret Int = 0\n  for i in 0..<m\n    acc = acc + 1\n  acc")
# The same loop on public data.
TALLY = (f"tally(n: Int) -> Int\n=\n  let m = if n > {CLAMP} then {CLAMP} else n\n"
         "  let edit acc = 0\n  for i in 0..<m\n    acc = acc + 1\n  acc")
TIME = "std.time", ["millisecond"]
MAIN = "pub main() -> Unit\n  uses @console: Console[print]\n=\n"
AUDIT = ("pub capability Audit\n  op note(ok: internal Bool) -> Unit\n    accepts internal\n\n"
         "handler kept() for Audit\n  op note(ok) = unit")


def _make(op, seed, edition, form, decls, main, v, imports=(TIME,), expect=(), note=""):
    """A program of the family: module `main` with `decls` and `main`'s body, the secret literal
    of `v` placed. Only edition 0.4 has these constructs; any other edition is refused."""
    if edition not in EDITIONS:
        raise ValueError(f"family W runs edition {EDITION} only, not {edition}")
    text = join_decls(header(MOD, imports=imports), *decls, MAIN + main)
    files = package_files("hunt", edition, {MOD: text}, entry=MOD, grants=["console"])
    files["ita.lock"] = lock_text()
    files, vary = v.place(files)
    if not note:
        note = (f"a closed call: a correct checker rejects it ({', '.join(expect)}); one that "
                "accepted it would let the oracle see the secret under the cost clock"
                if expect else
                "expected accepted; under the cost clock what an observer below the secret sees "
                "must not depend on it")
    return Program(op=op, rule=RULE, edition=edition, seed=seed, cls=OPEN, files=files,
                   expect=list(expect), note=note, variant=form, vary=vary)


def _secret(rng, v):
    """The varied secret literal and its binding in `main`."""
    lit = v.lit("Int", "secret", str(rng.choice([3, 30, 100])))
    return f"  let s: secret Int = {lit}\n"


def _deadline(rng):
    return rng.choice([5, 20, 50])


# W1. `e within d` over work a secret decides: the time-out is labeled `τ(e)`, so its fallback is
# secret (accepted when only a secret place keeps it), and a public use of it is refused; the
# secret may come from the caller, from a literal of a callee (#1685) or from a handler's operation
# (#1436). Work on public data inside the `within`, with a secret beside it, stays public.
def w1(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["fallback-kept", "fallback-shown", "callee-literal", "handler-literal",
                       "attempt-kept", "attempt-shown", "public-beside"])
    v = Varied()
    d = _deadline(rng)
    decls = [COUNT]
    within = f"within {d} * millisecond"
    if form == "fallback-kept":
        main = _secret(rng, v) + (f"  let r = count(s) {within}\n    if error use -1\n"
                                  '  @console.print("done")')
        expect = []
    elif form == "fallback-shown":
        main = _secret(rng, v) + (f"  let r = count(s) {within}\n    if error use -1\n"
                                  "  let shown = if r == -1 then 0 else 1\n"
                                  '  @console.print("{shown}")')
        expect = ["U4003"]
    elif form == "callee-literal":
        lit = v.lit("Int", "secret", str(rng.choice([3, 30, 100])))
        decls.append(f"work() -> secret Int\n=\n  let k: secret Int = {lit}\n  count(k)")
        main = (f"  let r = work() {within}\n    if error use -1\n  let ok = r > 0\n"
                '  @console.print("{ok}")')
        expect = ["U4003"]
    elif form == "handler-literal":
        lit = v.lit("Int", "secret", str(rng.choice([3, 30, 100])))
        decls += ["capability Vault\n  op check() -> Bool",
                  "spin(n: secret Int) -> Bool\n=\n  let k = count(n)\n  true",
                  f"handler sealed() for Vault\n  op check() = spin({lit})"]
        main = (f"  let ok = @vault.check() with @vault = sealed() {within}\n"
                '    if error use false\n  @console.print("{ok}")')
        expect = ["U4003"]
    elif form == "attempt-kept":
        main = _secret(rng, v) + (f"  let res = attempt count(s) {within}\n"
                                  "  let ok = res is Ok\n"
                                  '  @console.print("done")')
        expect = []
    elif form == "attempt-shown":
        main = _secret(rng, v) + (f"  let res = attempt count(s) {within}\n"
                                  "  let ok = res is Ok\n"
                                  '  @console.print("{ok}")')
        expect = ["U4003"]
    else:
        decls.append(TALLY)
        main = _secret(rng, v) + (f"  let hidden = count(s)\n"
                                  f"  let r = tally({rng.choice([3, 10, 60])}) {within}\n"
                                  '    if error use -1\n  @console.print("{r}")')
        expect = []
    return _make("W1", seed, edition, form, decls, main, v, expect=expect)


# W2. `first of` over work a secret decides: its value carries `t`, the timing of every branch, so
# a race inside a `within` is kept secret, and a branch whose public effect a sibling's win could
# cut is refused (the timing floor, §6 rule 8).
def w2(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["race-kept", "race-shown", "branch-effect"])
    v = Varied()
    d = _deadline(rng)
    decls = [COUNT]
    race = ("race(s: secret Int) -> secret Int\n=\n  let r = first of\n    count(s)\n"
            f"    count({rng.choice([5, 25])})\n  r")
    if form == "race-kept":
        decls.append(race)
        main = _secret(rng, v) + (f"  let r = race(s) within {d} * millisecond\n"
                                  '    if error use -1\n  @console.print("done")')
        expect = []
    elif form == "race-shown":
        decls.append(race)
        main = _secret(rng, v) + (f"  let r = race(s) within {d} * millisecond\n"
                                  "    if error use -1\n  let won = r > 0\n"
                                  '  @console.print("{won}")')
        expect = ["U4003"]
    else:
        decls.append("loud(n: secret Int) -> Int\n  uses @console: Console[print]\n=\n"
                     '  let k = count(n)\n  @console.print("first")\n  1')
        main = _secret(rng, v) + ("  let r = first of\n    loud(s)\n    loud(s)\n"
                                  '  @console.print("done")')
        expect = ["U4002"]
    return _make("W2", seed, edition, form, decls, main, v, expect=expect)


# W3. `tasks` and `start` beside a secret: a started task whose check points the secret decides
# would move the switch after 1024 of them and so order the other tasks' public prints (§8 rule
# 5, U4002); one that only carries the secret, beside tasks that print, keeps the order public.
def w3(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["secret-job", "carried", "bounded-carried"])
    v = Varied()
    say = "say(line: Text) -> Unit\n  uses @console: Console[print]\n=\n  @console.print(line)"
    bound = " at most 2 at once" if form == "bounded-carried" else ""
    lines = rng.choice([["a", "b"], ["a", "b", "c"]])
    starts = "".join(f'    start say("{x}")\n' for x in lines)
    if form == "secret-job":
        decls = [COUNT, "job(n: secret Int) -> Unit\n=\n  let k = count(n)\n  unit", say]
        main = _secret(rng, v) + f"  tasks\n    start job(s)\n{starts}" + '  @console.print("done")'
        expect = ["U4002"]
    else:
        decls = ["keep(n: secret Int) -> Unit = unit", say]
        main = _secret(rng, v) + (f"  let t = s + 1\n  tasks{bound}\n    start keep(t)\n{starts}"
                                  '  @console.print("done")')
        expect = []
    return _make("W3", seed, edition, form, decls, main, v, imports=(), expect=expect)


# W4. The limits of `std.limit` beside a secret: whether `acquire` finds a token depends on the
# timing of the statement the `limiter` scopes over, which must be at most `internal` (§10 rule 4,
# U4001 at the binding); secret work before that statement leaves its bucket as it was made.
def w4(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["work-inside", "work-before"])
    v = Varied()
    per = rng.choice([10, 20, 50])
    inside = form == "work-inside"
    param = "n: secret Int" if inside else ""
    work = "  let k = count(n)\n" if inside else ""
    decls = [COUNT, AUDIT,
             f"twice({param}) -> Unit\n  uses @lim: Limit[acquire]\n  uses @audit: Audit[note]\n"
             f"=\n  let a = attempt @lim.acquire()\n{work}  let b = attempt @lim.acquire()\n"
             "  let ok = b is Ok\n  @audit.note(ok)"]
    install = (f"with @lim = limiter(at_most: 1, per: {per} * millisecond, waiting: 0), "
               "@audit = kept()")
    if inside:
        main = _secret(rng, v) + f"  twice(s) {install}\n" + '  @console.print("done")'
        expect = ["U4001"]
    else:
        main = _secret(rng, v) + (f"  let k = count(s)\n  twice() {install}\n"
                                  '  @console.print("done")')
        expect = []
    return _make("W4", seed, edition, form, decls, main, v,
                 imports=(("std.limit", ["Limit", "limiter"]), TIME), expect=expect)


OPERATORS = {"W1": w1, "W2": w2, "W3": w3, "W4": w4}
