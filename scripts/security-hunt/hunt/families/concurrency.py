"""Family C: structured concurrency, `together` (`spec/19 §3`, edition 0.2).

A `together for` runs its body under the top label of its list, or the labels of its range bounds,
since the number of tasks depends on them; a `together` block's branches run under the block's
program counter; a branch keeps its value's labels; and which tasks a failure cancels may depend
only on failures under a public program counter. The model has no `together`
(docs/reviews/security-hunt/coverage.md), so these rules rest on the checker alone.

In edition 0.1, `together` is reserved: every program of this family is U0103 there, which checks
the edition gate (`spec/19`, Editions).
"""

from ..program import MUST_REJECT, OPEN, Program, rng_for
from ..una import header, join_decls, package

MOD = "app.main"
RULE = "spec/19 §3"

LOG = "pub capability Log\n  op write(line: Text) -> Unit"
OOPS = "pub kind Oops = Bad | Worse"
SHUT = "pub kind Shut = | Closed"
LOG_ONE = 'log_one() -> Int\n  uses @log: Log[write]\n=\n  @log.write("branch")\n  1'


def _make(op, edition, seed, cls, text, expect=(), variant="", note=""):
    if edition == "0.1":
        cls, expect, note = MUST_REJECT, ["U0103"], "edition 0.1 reserves `together`"
    files = package("hunt", edition, {MOD: text})
    return Program(op=op, rule=RULE, edition=edition, seed=seed, cls=cls, files=files,
                   expect=list(expect), note=note, variant=variant)


# C1. The number of tasks decided by a secret: a `together for` over a secret list or a range with
# a secret bound, whose body has a sink, a failure, or whose result's length leaves as public.
# The twin iterates a public list of secret elements: the count is public, and so is the length.
def c1(seed, edition):
    rng = rng_for(seed)
    source = rng.choice(["list", "range"])
    act = rng.choice(["sink", "fail", "length"])
    twin = rng.random() < 0.25
    if source == "list":
        param = "pins: List[secret Int]" if twin else "pins: secret List[Int]"
        head = "together for p in pins"
    else:
        param = "limit: Int" if twin else "limit: secret Int"
        head = "together for p in 0..<limit"
    if act == "sink":
        fn = (f"pub run({param}) -> List[Unit]\n  uses @log: Log[write]\n=\n"
              f'  {head}\n    @log.write("one")')
        expect = ["U4003"]
    elif act == "fail":
        fn = (f"pub run({param}) -> List[Int]\n  fails Oops\n=\n"
              f"  {head}\n    if 1 > 0\n      fail Bad\n    0")
        expect = ["U4002"]
    else:
        fn = f"pub run({param}) -> Int\n=\n  let done = {head}\n    1\n  done.len"
        expect = ["U4001"]
    text = join_decls(header(MOD), OOPS, LOG, fn)
    return _make("C1", edition, seed, OPEN if twin else MUST_REJECT, text,
                 [] if twin else expect, variant=f"{source}-{act}" + ("-twin" if twin else ""))


# C2. A task's own data: an element of a public list with secret elements decides a failure or a
# sink inside its task, or a branch's binding keeps its secret label after the block.
def c2(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["element-fail", "element-sink", "branch-keeps", "branch-sink"])
    if form == "element-fail":
        fn = ("pub run(pins: List[secret Int]) -> List[Int]\n  fails Oops\n=\n"
              "  together for p in pins\n    if p > 3\n      fail Bad\n    p")
        expect = ["U4002"]
    elif form == "element-sink":
        fn = ("pub run(pins: List[secret Int]) -> List[Unit]\n  uses @log: Log[write]\n=\n"
              '  together for p in pins\n    @log.write("{p}")')
        expect = ["U4003"]
    elif form == "branch-keeps":
        fn = ("pub run(pin: secret Int, n: Int) -> Int\n=\n"
              "  together\n    let x = pin + 1\n    let y = n + 1\n  x")
        expect = ["U4001"]
    else:
        fn = ("pub run(pin: secret Int, n: Int) -> Unit\n  uses @log: Log[write]\n=\n"
              '  together\n    let x = pin + 1\n    let y = n + 1\n  @log.write("{x}")')
        expect = ["U4003"]
    text = join_decls(header(MOD), OOPS, LOG, fn)
    return _make("C2", edition, seed, MUST_REJECT, text, expect, variant=form)


# C3. A block under a secret program counter: its branches inherit it, so an effect in a branch,
# directly or in a called function, or a nested `together for` with a sink, is rejected.
def c3(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["call-effect", "nested-for", "twin-public-pc"])
    decls = [header(MOD), OOPS, LOG, LOG_ONE]
    if form == "call-effect":
        fn = ("pub run(flag: secret Bool) -> Int\n  uses @log: Log[write]\n=\n"
              "  if flag\n    together\n      let x = 1\n      let y = log_one()\n    x + y\n"
              "  else\n    0")
        # A call of a function with effects under a raised program counter (spec/07 §7 rule 4).
        cls, expect = MUST_REJECT, ["U4002"]
    elif form == "nested-for":
        fn = ("pub run(flag: secret Bool, ids: List[Int]) -> Int\n  uses @log: Log[write]\n=\n"
              "  if flag\n    together\n      let x = 1\n      let ys = together for i in ids\n"
              '        @log.write("one")\n    x\n  else\n    0')
        cls, expect = MUST_REJECT, ["U4003"]
    else:
        fn = ("pub run(flag: Bool) -> Int\n  uses @log: Log[write]\n=\n"
              "  if flag\n    together\n      let x = 1\n      let y = log_one()\n    x + y\n"
              "  else\n    0")
        cls, expect = OPEN, []
    decls.append(fn)
    return _make("C3", edition, seed, cls, join_decls(*decls), expect, variant=form)


# C4. Cancellation seen by a sibling: one task's failure is decided by a value above `public`,
# and a sibling task, or code after the block, has a public effect that the failure cancels or
# skips. spec/19 §3 rule 2 keeps cancellation public by requiring that failures happen under
# public labels, so passing such a failure out of its task is U4002; `attempt` keeps it in the
# task, and the block cannot fail (open twin). The failure comes from `Http.get` with an
# `internal` URL, whose failure every argument labels (spec/07 §7 rule 4, S-15). A user
# capability cannot give one: its handler is checked at the operation's `accepts`, so it cannot
# fail on its argument (campaign triage of C4's first draft).
def c4(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["sibling", "after-block", "for-sibling", "attempt-twin"])
    uses = ("  uses @http: Http[get]\n  uses @log: Log[write]\n  fails HttpError\n")
    get = "@http.get({u}, headers: no_headers(), key_name: None)"
    if form in ("sibling", "after-block"):
        tail = '  @log.write("done")\n  b' if form == "after-block" else "  b"
        fn = ("pub run(url: internal Text) -> Int\n" + uses + "=\n"
              f"  together\n    let a = {get.format(u='url')}\n      if error pass\n"
              "    let b = log_one()\n" + tail)
        cls, expect = MUST_REJECT, ["U4002"]
    elif form == "for-sibling":
        fn = ("pub run(urls: List[internal Text]) -> List[Unit]\n" + uses + "=\n"
              f"  together for u in urls\n    let r = {get.format(u='u')}\n"
              '      if error pass\n    @log.write("one")')
        cls, expect = MUST_REJECT, ["U4002"]
    else:
        fn = ("pub run(url: internal Text) -> Int\n" + uses + "=\n"
              f"  together\n    let a = attempt {get.format(u='url')}\n    let b = log_one()\n"
              '  @log.write("done")\n  b')
        cls, expect = OPEN, []
    headers = "no_headers() -> List[Header] = []"
    text = join_decls("module app.main\n\nimport std.host (Http)", OOPS, LOG, LOG_ONE, headers, fn)
    note = ("a leak candidate if accepted: an internal-decided failure cancels a public effect"
            if cls == MUST_REJECT else "")
    return _make("C4", edition, seed, cls, text, expect, variant=form, note=note)


# C5. Probes of C4's triage (a user operation's failure cannot depend on a secret argument): the
# handler itself reads its argument through a host operation and fails or not on the result, or
# the handler that always fails or never fails is chosen by `with` under a secret condition, around
# a `together` block whose sibling has a public effect. Both must be rejected.
DOOR_HTTP = ("capability Door\n  op open(code: Text) -> Int\n    fails Shut\n    accepts internal\n\n"
             "handler probing() for Door\n  uses @http: Http[get]\n  op open(code) =\n"
             "    let r = attempt @http.get(code, headers: no_headers(), key_name: None)\n"
             "    when r\n      is Ok -> 1\n      is Err -> fail Closed")
DOOR_FILES = ("capability Door\n  op open(code: Text) -> Int\n    fails Shut\n    accepts internal\n\n"
              "handler probing() for Door\n  uses @files: Files[read]\n  op open(code) =\n"
              "    let r = attempt @files.read(code)\n"
              "    when r\n      is Ok -> 1\n      is Err -> fail Closed")
DOOR_PAIR = ("capability Door\n  op open(code: Int) -> Int\n    fails Shut\n    accepts secret\n\n"
             "handler failing() for Door\n  op open(code) = fail Closed\n\n"
             "handler fine() for Door\n  op open(code) = 1")


def c5(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["handler-http", "handler-files", "with-under-pc"])
    work = ("work(arg: {t}) -> Int\n  uses @door: Door[open]\n  uses @log: Log[write]\n"
            "  fails Shut\n=\n  together\n    let a = @door.open(arg)\n      if error pass\n"
            "    let b = log_one()\n  b")
    decls = ["module app.main\n\nimport std.host (Http)", OOPS, SHUT, LOG, LOG_ONE,
             "no_headers() -> List[Header] = []"]
    if form in ("handler-http", "handler-files"):
        decls += [DOOR_HTTP if form == "handler-http" else DOOR_FILES, work.format(t="internal Text")]
        uses = "  uses @http: Http[get]\n" if form == "handler-http" else "  uses @files: Files[read]\n"
        fn = ("pub run(url: internal Text) -> Int\n" + uses + "  uses @log: Log[write]\n"
              "  fails Shut\n=\n  work(url) with @door = probing()\n    if error pass")
        # The handler branches on what its argument decided: U4002 at its failure (Http), or
        # U4003 at the read, whose path accepts only `public` (Files).
        expect = ["U4002"] if form == "handler-http" else ["U4003"]
    else:
        decls += [DOOR_PAIR, work.format(t="Int")]
        fn = ("pub run(flag: secret Bool) -> Int\n  uses @log: Log[write]\n  fails Shut\n=\n"
              "  if flag\n    work(1) with @door = failing()\n      if error pass\n"
              "  else\n    work(1) with @door = fine()\n      if error pass")
        expect = ["U4002"]
    decls.append(fn)
    return _make("C5", edition, seed, MUST_REJECT, join_decls(*decls), expect, variant=form,
                 note="a leak candidate if accepted: probes C4's triage")


OPERATORS = {"C1": c1, "C2": c2, "C3": c3, "C4": c4, "C5": c5}
