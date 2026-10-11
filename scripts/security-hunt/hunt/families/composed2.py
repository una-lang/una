"""More composed families (the coordinator's order after seed 8, by risk).

- TC, policies inside `together` (`spec/07 §5` with `spec/19 §3`): a policy called by a task under
  a secret program counter, a secret task count or a secret element; a failing `via` (S-18, ADR
  0090) whose failure, labeled by the policy's `to`, cancels a sibling task with a public effect;
  and, open, a release after the block of what a task computed. The relative order of tasks is
  timing, outside the model (`spec/07 §1`, `spec/19 §3` rule 3), so no shape depends on it.
- CP, payload failures in tasks (`spec/19 §2` rule 5, §3): a task fails with a payload, decided by
  its secret element or carrying a secret field, while a sibling or code after the block has a
  public effect, and the block's failure is recovered with its payload.
- GS, generic sum kinds (`spec/07 §7` rules 2 and 3): a `Maybe[T]` or `Either[A, B]` at a secret
  instance, matched into a public sink, and an invariant parameter inside a variant.

Edition 0.1 reserves `together`: there, every TC and CP program is must-reject U0103.
"""

from ..program import MUST_REJECT, OPEN, Program, rng_for
from ..una import header, join_decls, package, policy
from .trusted import CALLER, KEYS, Setup, caller_module, keys_module, make

LOG = "pub capability Log\n  op write(line: Text) -> Unit"
LOG_ONE = 'log_one() -> Int\n  uses @log: Log[write]\n=\n  @log.write("branch")\n  1'
USES_LOG = "  uses @log: Log[write]\n"


def _gate(op, edition, cls, expect, note=""):
    if edition == "0.1":
        return MUST_REJECT, ["U0103"], "edition 0.1 reserves `together`"
    return cls, expect, note


# TC1. A policy called by a task under a secret program counter: a secret element decides the call,
# a secret count decides how many tasks call it, or a branch calls it under a secret flag (T10's
# shapes inside `together`). U4002: a policy call can fail, and is an effect, under that pc.
def tc1(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["element-flag", "count", "branch-flag"])
    twin = rng.random() < 0.25
    s = Setup("Int", "secret", (), "internal")
    F, d = s.frm, s.default()
    lab = "" if twin else "secret "
    if form == "element-flag":
        fn = (f"pub run(flags: List[{lab}Bool], t: {F}) -> List[internal Int]\n=\n"
              f"  together for f in flags\n    if f then mask(t) else {d}")
    elif form == "count":
        fn = (f"pub run(n: {lab}Int, t: {F}) -> List[internal Int]\n=\n"
              "  together for i in 0..<n\n    mask(t)")
    else:
        fn = (f"pub run(flag: {lab}Bool, t: {F}) -> internal Int\n=\n"
              f"  together\n    let x = if flag then mask(t) else {d}\n    let y = 1\n  x")
    cls, expect, note = _gate("TC1", edition, OPEN if twin else MUST_REJECT,
                              [] if twin else ["U4002"])
    return make("TC1", "spec/07 §5 rule 2, spec/19 §3", edition, seed, cls,
                {KEYS: keys_module(s), CALLER: caller_module(s, fn)}, s, expect, note=note,
                variant=form + ("-twin" if twin else ""))


# TC2. A failing `via` in one task, whose failure is labeled by the policy's `to` (S-18): passed out
# of the task, it cancels a sibling task, or skips code after the block, with a public effect. Above
# `public`, U4002, in both forms. With `to public` the failure is public and only the after-block
# form is open: how far a cancelled sibling got is timing (`spec/19 §3` rule 3), so an open sibling
# form would make the two-run oracle compare scheduling, not labels (Una-2's review of #533).
def tc2(seed, edition):
    rng = rng_for(seed)
    to_level = rng.choice(["public", "internal", "secret"])
    form = rng.choice(["sibling", "after-block"])
    if to_level == "public":
        form = "after-block"
    s = Setup("Int", "secret", (), to_level)
    keys = join_decls(header(KEYS, trusted=True), "pub kind Gate = | TooBig",
                      policy("checkpin", "secret Int", s.to, "gate"),
                      f"gate(x: secret Int) -> {s.to}\n  fails Gate\n=\n"
                      "  if x > 3 then fail TooBig else x")
    tail = '  @log.write("done")\n  b' if form == "after-block" else "  b"
    fn = ("pub run(n: secret Int) -> Int\n" + USES_LOG + "  fails Gate\n=\n"
          "  together\n    let a = checkpin(n)\n      if error pass\n    let b = log_one()\n" + tail)
    caller = caller_module(s, LOG, LOG_ONE, fn, imports=[(KEYS, ["Gate", "checkpin"])])
    public = to_level == "public"
    cls, expect, note = _gate("TC2", edition, OPEN if public else MUST_REJECT,
                              [] if public else ["U4002"],
                              "a public failure label: expected accepted" if public else "")
    return make("TC2", "spec/07 §5 rule 6 (ADR 0090), spec/19 §3", edition, seed, cls,
                {KEYS: keys, CALLER: caller}, s, expect, note=note, variant=f"to-{to_level}-{form}")


# TC3. Open: what a task computed from a secret is released after the block, at a public program
# counter, as any value is: the join keeps its label, and the policy's checks apply as usual.
def tc3(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["branch", "for-first"])
    s = Setup("Int", "secret", (), "internal")
    F = s.frm
    if form == "branch":
        fn = (f"pub run(t: {F}, n: Int) -> internal Int\n=\n"
              "  together\n    let x = t + 1\n    let y = n + 1\n  mask(x)")
    else:
        fn = (f"pub run(ts: List[{F}]) -> internal Int\n=\n"
              "  let xs = together for t in ts\n    t * 2\n"
              "  when xs.first_where(keep: fn(v) -> true)\n    is Some(v) -> mask(v)\n"
              "    is None -> 0")
    cls, expect, note = _gate("TC3", edition, OPEN, [],
                              "expected accepted: a release after the block, at a public pc")
    return make("TC3", "spec/07 §5, spec/19 §3", edition, seed, cls,
                {KEYS: keys_module(s), CALLER: caller_module(s, fn)}, s, expect, note=note,
                variant=form)


def _plain(op, rule, edition, seed, cls, text, expect, variant, note=""):
    files = package("hunt", edition, {"app.main": text})
    return Program(op=op, rule=rule, edition=edition, seed=seed, cls=cls, files=files,
                   expect=list(expect), variant=variant, note=note)


# CP1. A task fails with a payload: the failure decided by the task's secret element (U4002), or
# a public failure carrying a secret field recovered after the block into a public sink (U4001,
# U4003), while a sibling has a public effect. Twins on public data.
def cp1(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["decided-by-element", "secret-field-recovered"])
    twin = rng.random() < 0.25
    lab = "" if twin else "secret "
    head = header("app.main")
    if form == "decided-by-element":
        kinds = "pub kind Stop = | Halt(code: Int)"
        fn = (f"pub run(xs: List[{lab}Int]) -> List[Int]\n" + USES_LOG + "  fails Stop\n=\n"
              "  together for x in xs\n    if x > 3\n      fail Halt(code: 1)\n"
              '    @log.write("ok")\n    x')
        expect = ["U4002"]
        text = join_decls(head, kinds, LOG, fn)
    else:
        kinds = f"pub kind Stop = | Halt(code: {lab}Int)"
        work = (f"work(n: {lab}Int) -> Int\n  fails Stop\n=\n  fail Halt(code: n)")
        fn = (f"pub run(n: {lab}Int) -> Int\n" + USES_LOG + "=\n"
              "  let r = attempt block(n)\n  let c = when r\n    is Ok(v) -> v\n"
              "    is Err(e) ->\n      when e\n        is Halt(code) -> code\n"
              '  @log.write("{c}")\n  0')
        block = (f"block(n: {lab}Int) -> Int\n" + USES_LOG + "  fails Stop\n=\n"
                 "  together\n    let a = work(n)\n      if error pass\n    let b = log_one()\n  b")
        expect = ["U4003"]
        text = join_decls(head, kinds, LOG, LOG_ONE, work, block, fn)
    cls, expect, note = _gate("CP1", edition, OPEN if twin else MUST_REJECT,
                              [] if twin else expect)
    return _plain("CP1", "spec/19 §2 rule 5, §3; spec/07 §7", edition, seed, cls, text, expect,
                  form + ("-twin" if twin else ""), note)


GS_KINDS = ("pub kind Maybe[T] = Just(value: T) | Nothing\n\n"
            "pub kind Either[A, B] = Left(a: A) | Right(b: B)\n\n"
            "pub kind Sink[T]\n  eat: fn(T) -> Int")


# GS1. Generic sums at secret instances, matched into a public result or print, and an invariant
# parameter inside a variant taken at a higher instance.
def gs1(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["maybe-return", "either-print", "match-decides", "sink-in-variant"])
    twin = rng.random() < 0.25
    lab = "" if twin else "secret "
    console = "  uses @console: Console[print]\n"
    if form == "maybe-return":
        fn = (f"pub run(m: Maybe[{lab}Int]) -> Int\n=\n  when m\n    is Just(value) -> value\n"
              "    is Nothing -> 0")
        expect = ["U4001"]
    elif form == "either-print":
        fn = (f"pub run(e: Either[Int, {lab}Int]) -> Unit\n" + console + "=\n  when e\n"
              '    is Left(a) -> @console.print("{a}")\n    is Right(b) -> @console.print("{b}")')
        expect = ["U4003"]
    elif form == "match-decides":
        fn = (f"pub run(m: {lab}Maybe[Int]) -> Unit\n" + console + "=\n  when m\n"
              '    is Just(value) -> @console.print("some")\n    is Nothing -> unit')
        expect = ["U4003"]
    else:
        target = "Maybe[Sink[Int]]" if twin else "Maybe[Sink[secret Int]]"
        pin = "pin: Int" if twin else "pin: secret Int"
        fn = (f"made() -> Maybe[Sink[Int]] = Just(value: Sink(eat: fn(x: Int) -> x))\n\n"
              f"pub run({pin}) -> Int\n=\n  let m: {target} = made()\n  when m\n"
              "    is Just(value) -> value.eat(pin)\n    is Nothing -> 0")
        expect = ["U4001"]
    text = join_decls(header("app.main"), GS_KINDS, fn)
    return _plain("GS1", "spec/07 §7 rules 2 and 3 (ADR 0046)", edition, seed,
                  OPEN if twin else MUST_REJECT, text, [] if twin else expect,
                  form + ("-twin" if twin else ""))


OPERATORS = {"TC1": tc1, "TC2": tc2, "TC3": tc3, "CP1": cp1, "GS1": gs1}
