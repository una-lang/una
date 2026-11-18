"""Family O: `Ord`, `sort`, `min`, `max` (ADR 0066, `spec/07 §7` rule 8, `spec/20 §4`).

A comparison on a constrained type parameter carries `ℓ*_T`; `min` and `max` join every label of
their arguments; `sort` labels every position of its result, the list itself included, with the
join of all its elements and of the list. The model reads no `Ord` constraint
(docs/reviews/security-hunt/coverage.md), so these rules rest on the checker alone. Each
must-reject shape has a twin on public data.
"""

from ..program import MUST_REJECT, OPEN, Program, rng_for
from ..una import header, join_decls, package

MOD = "app.main"
RULE = "spec/07 §7 rule 8 (ADR 0066), spec/20 §4"
CONSOLE = "  uses @console: Console[print]\n"


# Edition 0.1 has no `sort` (U1001) and no constrained type parameters (U0101): there, O1 and O3
# are must-reject with that code, which checks the edition gate.
GATE_0_1 = {"O1": "U1001", "O3": "U0101"}


def _make(op, edition, seed, cls, text, expect=(), variant="", note=""):
    if edition == "0.1" and op in GATE_0_1:
        cls, expect, note = MUST_REJECT, [GATE_0_1[op]], "edition 0.1 lacks this construct"
    files = package("hunt", edition, {MOD: text})
    return Program(op=op, rule=RULE, edition=edition, seed=seed, cls=cls, files=files,
                   expect=list(expect), note=note, variant=variant)


# O1. `sort` of a list of secrets, then its first element, an equality on it, or its length, into
# a public position. The length is the conservative case (the rule raises the list itself), the
# others are real channels: which value comes first depends on the secrets' order.
def o1(seed, edition):
    rng = rng_for(seed)
    use = rng.choice(["first", "equal", "length", "print-first"])
    twin = rng.random() < 0.25
    elems = "List[Int]" if twin else "List[secret Int]"
    if use == "first":
        fn = (f"pub lowest(xs: {elems}) -> Int\n=\n  let s = sort(xs)\n"
              "  when s.first()\n    is Some(x) -> x\n    is None -> 0")
        expect = ["U4001"]
    elif use == "equal":
        fn = f"pub same(xs: {elems}) -> Bool = sort(xs) == [1, 2]"
        expect = ["U4001"]
    elif use == "length":
        fn = f"pub count(xs: {elems}) -> Int = sort(xs).len"
        expect = ["U4001"]
    else:
        fn = (f"pub show(xs: {elems}) -> Unit\n" + CONSOLE + "=\n  let s = sort(xs)\n"
              "  when s.first()\n    is Some(x) -> @console.print(\"{x}\")\n    is None -> unit")
        expect = ["U4003"]
    note = "conservative: the sorted list's length is public in fact" if use == "length" else ""
    text = join_decls(header(MOD), fn)
    return _make("O1", edition, seed, OPEN if twin else MUST_REJECT, text,
                 [] if twin else expect, variant=use + ("-twin" if twin else ""),
                 note="" if twin else note)


# O2. `min` and `max` of a public and a secret value, into a public result or a print.
def o2(seed, edition):
    rng = rng_for(seed)
    op = rng.choice(["min", "max"])
    sink = rng.choice(["return", "print", "compare"])
    twin = rng.random() < 0.25
    pin = "pin: Int" if twin else "pin: secret Int"
    call = f"{op}(pin, b: n)"
    if sink == "return":
        fn = f"pub pick({pin}, n: Int) -> Int = {call}"
        expect = ["U4001"]
    elif sink == "print":
        fn = f"pub pick({pin}, n: Int) -> Unit\n" + CONSOLE + f'=\n  @console.print("{{{call}}}")'
        expect = ["U4003"]
    else:
        # Whether the public value won tells whether the secret is above or below it.
        fn = f"pub pick({pin}, n: Int) -> Bool = {call} == n"
        expect = ["U4001"]
    text = join_decls(header(MOD), fn)
    return _make("O2", edition, seed, OPEN if twin else MUST_REJECT, text,
                 [] if twin else expect, variant=f"{op}-{sink}" + ("-twin" if twin else ""))


# O3. A generic function constrained by `Ord` whose result a comparison chooses, or whose effect a
# comparison decides, called with a secret and a public value.
def o3(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["choose", "decide-effect", "to-int"])
    twin = rng.random() < 0.25
    pin = "pin: Int" if twin else "pin: secret Int"
    if form == "choose":
        gen = "lower[T: Ord](a: T, b: T) -> T = if a < b then a else b"
        fn = f"pub pick({pin}, n: Int) -> Int = lower(pin, b: n)"
        expect = ["U4001"]
    elif form == "decide-effect":
        gen = ("note[T: Ord](a: T, b: T) -> Unit\n" + CONSOLE +
               '=\n  if a < b\n    @console.print("less")\n  else\n    unit')
        # The generic body itself runs a public effect under `ℓ*_T` (U4002 at its print).
        fn = f"pub pick({pin}, n: Int) -> Unit\n" + CONSOLE + "=\n  note(pin, b: n)"
        expect = ["U4002"]
    else:
        # `to_int()` needs `Integer` (U2026 under `Ord` alone).
        gen = "rank[T: Integer](a: T) -> Int = a.to_int()"
        fn = f"pub pick({pin}) -> Int = rank(pin)"
        expect = ["U4001"]
    if form == "decide-effect":
        # Rejected at the generic's body whatever the call: no twin is open.
        cls = MUST_REJECT
    else:
        cls = OPEN if twin else MUST_REJECT
    text = join_decls(header(MOD), gen, fn)
    return _make("O3", edition, seed, cls, text, [] if cls == OPEN else expect,
                 variant=form + ("-twin" if twin and cls == OPEN else ""))


OPERATORS = {"O1": o1, "O2": o2, "O3": o3}
