"""Family S: user sum types, and `is` / `in` (`spec/07 §3` rule 2, §7 rules 3 and 4).

`when` on a sum value raises the program counter by the scrutinee's top label; a pattern binding
takes the variant field's declared type raised by it; `is` and `in` take the top label of their
operand. The model reads no user sum type with fields (docs/reviews/security-hunt/coverage.md), so
these rules rest on the checker alone. Each must-reject shape has a twin on public data.
"""

from ..program import MUST_REJECT, OPEN, Program, rng_for
from ..una import header, join_decls, package

MOD = "app.main"
RULE = "spec/07 §3 rule 2, §7 rules 3 and 4"

GUESS = "pub kind Guess = Hit(pin: Int) | Miss"
SECRET_GUESS = "pub kind Vault = Open(code: secret Int) | Shut"
OOPS = "pub kind Oops = Bad | Worse"
CONSOLE = "  uses @console: Console[print]\n"


def _make(op, edition, seed, cls, text, expect=(), variant=""):
    files = package("hunt", edition, {MOD: text})
    return Program(op=op, rule=RULE, edition=edition, seed=seed, cls=cls, files=files,
                   expect=list(expect), variant=variant)


# S1. `when` on a secret sum value with an effect or a failure in an arm.
def s1(seed, edition):
    rng = rng_for(seed)
    act = rng.choice(["print", "fail", "value"])
    twin = rng.random() < 0.25
    arg = "g: Guess" if twin else "g: secret Guess"
    if act == "print":
        fn = (f"pub show({arg}) -> Unit\n" + CONSOLE + "=\n  when g\n"
              '    is Hit(pin) -> @console.print("hit")\n    is Miss -> unit')
        expect = ["U4003"]
    elif act == "fail":
        fn = (f"pub check({arg}) -> Int\n  fails Oops\n=\n  when g\n"
              "    is Hit(pin) -> fail Bad\n    is Miss -> 0")
        expect = ["U4002"]
    else:
        fn = f"pub pick({arg}) -> Int\n=\n  when g\n    is Hit(pin) -> 1\n    is Miss -> 0"
        expect = ["U4001"]
    text = join_decls(header(MOD), GUESS, OOPS, fn)
    return _make("S1", edition, seed, OPEN if twin else MUST_REJECT, text,
                 [] if twin else expect, variant=act + ("-twin" if twin else ""))


# S2. `is` on a secret sum value and `in` on a secret integer, into a public result or a print.
# The function is `probe`: `test` is the keyword of the test declaration from edition 0.3 on
# (ADR 0105, `spec/01`), so a function of that name is U0101 there, a generator error (#1911).
def s2(seed, edition):
    rng = rng_for(seed)
    test = rng.choice(["is", "in-range", "in-open-range"])
    sink = rng.choice(["return", "print", "branch"])
    twin = rng.random() < 0.25
    lab = "" if twin else "secret "
    if test == "is":
        param, expr = f"g: {lab}Guess", "g is Miss"
    elif test == "in-range":
        param, expr = f"n: {lab}Int", "n in 0..10"
    else:
        # `in` takes a range only (a list is U0101).
        param, expr = f"n: {lab}Int", "n in 0..<5"
    if sink == "return":
        fn = f"pub probe({param}) -> Bool = {expr}"
        expect = ["U4001"]
    elif sink == "print":
        fn = f"pub probe({param}) -> Unit\n" + CONSOLE + f'=\n  @console.print("{{{expr}}}")'
        expect = ["U4003"]
    else:
        fn = (f"pub probe({param}) -> Unit\n" + CONSOLE + f"=\n  if {expr}\n"
              '    @console.print("yes")\n  else\n    unit')
        expect = ["U4003"]
    text = join_decls(header(MOD), GUESS, fn)
    return _make("S2", edition, seed, OPEN if twin else MUST_REJECT, text,
                 [] if twin else expect, variant=f"{test}-{sink}" + ("-twin" if twin else ""))


# S3. A variant field declared `secret`, bound by a pattern of a public value and sent to a sink
# through each arm; the twin's field is public.
def s3(seed, edition):
    rng = rng_for(seed)
    sink = rng.choice(["return", "print", "compare"])
    twin = rng.random() < 0.25
    kinds = GUESS if twin else SECRET_GUESS
    ty, ctor, other = ("Guess", "Hit(pin)", "Miss") if twin else ("Vault", "Open(code)", "Shut")
    field = "pin" if twin else "code"
    if sink == "return":
        fn = (f"pub open(v: {ty}) -> Int\n=\n  when v\n    is {ctor} -> {field}\n"
              f"    is {other} -> 0")
        expect = ["U4001"]
    elif sink == "print":
        fn = (f"pub open(v: {ty}) -> Unit\n" + CONSOLE + f"=\n  when v\n"
              f'    is {ctor} -> @console.print("{{{field}}}")\n    is {other} -> unit')
        expect = ["U4003"]
    else:
        fn = (f"pub open(v: {ty}) -> Bool\n=\n  when v\n    is {ctor} -> {field} > 3\n"
              f"    is {other} -> false")
        expect = ["U4001"]
    text = join_decls(header(MOD), kinds, fn)
    return _make("S3", edition, seed, OPEN if twin else MUST_REJECT, text,
                 [] if twin else expect, variant=sink + ("-twin" if twin else ""))


OPERATORS = {"S1": s1, "S2": s2, "S3": s3}
