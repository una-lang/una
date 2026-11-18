"""Family P: failure payloads (`spec/07 §1`, §3 rule 3, §7; `spec/04 §8`).

A failure's payload is a value like any other: its fields keep their labels through `attempt` and
`when`, a value put into a field flows into the field's declared type, and which failure is raised
is decided by the program counter. The importer of the formal model reads no payload
(docs/reviews/security-hunt/coverage.md, "Failures: payloads"), so these rules rest on the checker
alone.
"""

from ..program import MUST_REJECT, OPEN, Program, rng_for
from ..una import Varied, header, join_decls, package

MOD = "app.main"
RULE = "spec/07 §7 rules 2 and 3, §3 rule 3"


def _make(op, edition, seed, cls, text, expect=(), variant="", note="", entry=False,
          varied=None):
    files = package("hunt", edition, {MOD: text}, entry=MOD if entry else None,
                    grants=["console"] if entry else ())
    vary = []
    if varied is not None:
        files, vary = varied.place(files)
    return Program(op=op, rule=RULE, edition=edition, seed=seed, cls=cls, files=files,
                   expect=list(expect), note=note, variant=variant, vary=vary)


def _field(level, shape):
    """The declared type of the payload field `value`, holding an `Int` at `level`."""
    base = f"{level} Int" if level != "public" else "Int"
    return {"scalar": base, "list": f"List[{base}]", "option": f"Option[{base}]"}[shape]


def _put(shape, v):
    return {"scalar": v, "list": f"[{v}]", "option": f"Some({v})"}[shape]


def _take(shape):
    """Statements that turn the field `value` into an `Int` named `n` (inside a `when` arm)."""
    if shape == "scalar":
        return "value"
    if shape == "list":
        # An element, not `.len`: the list's own label is public, so its length may be
        # (campaign triage, seed 11768755190238204873); P2's `len-const` is that open twin.
        return "when at(value, index: 0)\n            is Some(x) -> x\n            is None -> 0"
    return "when value\n            is Some(x) -> x\n            is None -> 0"


# P1. A payload with a public and a secret field reaches `main`'s failure; the runtime prints only
# the public fields (U6001). Open: the two-run oracle varies the secret.
def p1(seed, edition):
    rng = rng_for(seed)
    shape = rng.choice(["scalar", "list", "option"])
    v = Varied()
    lit = v.lit("Int", "secret", "4242")
    kinds = f"pub kind Stop = | Halt(code: Int, value: {_field('secret', shape)})"
    main = ("pub main() -> Unit\n  uses @console: Console[print]\n  fails Stop\n=\n"
            f"  let s: secret Int = {lit}\n"
            '  @console.print("start")\n'
            f"  fail Halt(code: 7, value: {_put(shape, 's')})")
    text = join_decls(header(MOD), kinds, main)
    return _make("P1", edition, seed, OPEN, text, variant=shape,
                 note="expected accepted; main's failure output must not depend on the secret",
                 entry=True, varied=v)


# P2. A secret payload field read back after `attempt`, into a public sink; and a secret value put
# into a field declared public. The twin keeps the payload and the sink, with public data.
# `len-const` reads only the length of a list of secrets whose length is constant: open, since
# `len` takes the list's own label (campaign triage, seed 11768755190238204873). `len-dep` makes
# that length depend on the secret, with the field's list labeled `public` (rejected where the
# list is put in) or `secret` (rejected where its length is returned).
def p2(seed, edition):
    rng = rng_for(seed)
    shape = rng.choice(["scalar", "list", "option", "len-const", "len-dep", "len-dep-top"])
    sink = rng.choice(["return", "print", "put-public"])
    if shape.startswith("len"):
        return _p2_len(seed, edition, shape, sink)
    twin = rng.random() < 0.25
    level = "public" if twin else "secret"
    kinds = f"pub kind PinError = | Bad(value: {_field(level, shape)})"
    arg = "pin: Int" if twin else "pin: secret Int"
    if sink == "put-public":
        # The field is declared public; only the value is secret.
        kinds = f"pub kind PinError = | Bad(value: {_field('public', shape)})"
        fn = (f"pub check({arg}) -> Int\n  fails PinError\n=\n"
              f"  fail Bad(value: {_put(shape, 'pin')})")
        text = join_decls(header(MOD), kinds, fn)
        return _make("P2", edition, seed, OPEN if twin else MUST_REJECT, text,
                     [] if twin else ["U4001"],
                     variant=f"{sink}-{shape}" + ("-twin" if twin else ""))
    inner = (f"inner({arg}) -> Int\n  fails PinError\n=\n"
             f"  fail Bad(value: {_put(shape, 'pin')})")
    fn, expect = _recover(arg, _take(shape), sink)
    text = join_decls(header(MOD), kinds, inner, fn)
    return _make("P2", edition, seed, OPEN if twin else MUST_REJECT, text,
                 [] if twin else expect, variant=f"{sink}-{shape}" + ("-twin" if twin else ""))


def _recover(arg, arm, sink):
    """`outer`, which recovers the payload's `value` as `n` and sends it to `sink`."""
    if "\n" in arm:
        arm_line = f"        is Bad(value) ->\n          {arm}\n"
    else:
        arm_line = f"        is Bad(value) -> {arm}\n"
    recover = ("  let r = attempt inner(pin)\n  let n = when r\n    is Ok(k) -> k\n"
               "    is Err(failure) ->\n      when failure\n" + arm_line)
    if sink == "print":
        return (f"pub outer({arg}) -> Unit\n  uses @console: Console[print]\n=\n" + recover +
                '  @console.print("{n}")'), ["U4003"]
    return f"pub outer({arg}) -> Int\n=\n" + recover + "  n", ["U4001"]


def _p2_len(seed, edition, shape, sink):
    sink = "print" if sink == "print" else "return"
    if shape == "len-const":
        kinds = "pub kind PinError = | Bad(value: List[secret Int])"
        build = "  fail Bad(value: [pin])"
        cls, expect = OPEN, []
    else:
        top = "secret " if shape == "len-dep-top" else ""
        kinds = f"pub kind PinError = | Bad(value: {top}List[secret Int])"
        build = "  let xs = if pin > 0 then [pin] else []\n  fail Bad(value: xs)"
        cls = MUST_REJECT
        # With a public list type the secret length is caught where the list is put in; with a
        # secret one, where its length reaches the sink.
        expect = ["U4001"] if shape == "len-dep" else _recover("", "", sink)[1]
    inner = "inner(pin: secret Int) -> Int\n  fails PinError\n=\n" + build
    fn, _ = _recover("pin: secret Int", "value.len", sink)
    text = join_decls(header(MOD), kinds, inner, fn)
    note = ("a leak candidate if accepted: the payload list's length depends on the secret"
            if cls == MUST_REJECT else "")
    return _make("P2", edition, seed, cls, text, expect, variant=f"{sink}-{shape}", note=note)


# P3. Which failure is raised, or which payload it carries, is decided by a secret: a failure under
# a raised program counter (U4002). The twin decides it by a public value.
def p3(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["which-payload", "which-variant", "fail-or-not"])
    twin = rng.random() < 0.25
    cond = "flag: Bool" if twin else "flag: secret Bool"
    kinds = "pub kind Stop = Halt(code: Int) | Quit"
    if form == "which-payload":
        body = "  if flag\n    fail Halt(code: 1)\n  else\n    fail Halt(code: 2)"
    elif form == "which-variant":
        body = "  if flag\n    fail Halt(code: 1)\n  else\n    fail Quit"
    else:
        body = "  if flag\n    fail Halt(code: 1)\n  else\n    0"
    fn = f"pub decide({cond}) -> Int\n  fails Stop\n=\n" + body
    text = join_decls(header(MOD), kinds, fn)
    return _make("P3", edition, seed, OPEN if twin else MUST_REJECT, text,
                 [] if twin else ["U4002"], variant=form + ("-twin" if twin else ""))


OPERATORS = {"P1": p1, "P2": p2, "P3": p3}
