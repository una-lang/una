"""Family G: kinds with type parameters, and invariance (`spec/07 §7` rule 2, ADR 0046).

A kind parameter that reaches a function parameter is contravariant there, so the kind is
invariant in it: `Sink[Int]` does not flow into `Sink[secret Int]`, whose holder would hand it a
secret and read the result back as public. Records with type parameters are outside the formal
model (docs/reviews/security-hunt/coverage.md, rule 2), so these rules rest on the checker alone.
The conformance sweeps (`tests/conformance/security/sweep_variance_*`) hold one wrapper per file;
G composes wrappers and flow sites, and adds recursive kinds.
"""

from ..program import MUST_REJECT, OPEN, Program, rng_for
from ..una import header, join_decls, package

MOD = "app.main"
RULE = "spec/07 §7 rule 2 (ADR 0046)"

SINK = "kind Sink[T]\n  eat: fn(T) -> Int"
BOX = "kind Box[T]\n  value: T"
# A recursive kind: the parameter reaches a function parameter one level down.
CHAIN = "kind Chain[T]\n  next: Option[Chain[T]]\n  eat: fn(T) -> Int"
# Mutually recursive kinds, the parameter reaching the function in the second one.
PAIR = "kind Outer[T]\n  inner: Option[Inner[T]]\n\nkind Inner[T]\n  eat: fn(T) -> Int\n  back: Option[Outer[T]]"


def _make(op, edition, seed, cls, text, expect=(), variant="", note=""):
    files = package("hunt", edition, {MOD: text})
    return Program(op=op, rule=RULE, edition=edition, seed=seed, cls=cls, files=files,
                   expect=list(expect), note=note, variant=variant)


def _wrap(w, t):
    """The type `t` inside wrapper `w`."""
    return {"none": t, "box": f"Box[{t}]", "option": f"Option[{t}]", "list": f"List[{t}]"}[w]


def _wrap_val(w, v):
    return {"none": v, "box": f"Box(value: {v})", "option": f"Some({v})", "list": f"[{v}]"}[w]


def _unwrap(w, v):
    """Statements binding `s`, the sink inside `v` of wrapper `w`, then using it on `pin`."""
    if w == "none":
        return f"  {v}.eat(pin)"
    if w == "box":
        return f"  {v}.value.eat(pin)"
    if w == "option":
        return f"  when {v}\n    is Some(k) -> k.eat(pin)\n    is None -> 0"
    return f"  when {v}.first()\n    is Some(k) -> k.eat(pin)\n    is None -> 0"


# G1. A sink built at `public` reaches a position typed at `secret`, inside a wrapper and through
# one of rule 2's flow sites: the secret handed to it comes back as a public `Int`.
def g1(seed, edition):
    rng = rng_for(seed)
    w = rng.choice(["none", "box", "option", "list"])
    site = rng.choice(["let", "return", "argument", "field"])
    low, high = _wrap(w, "Sink[Int]"), _wrap(w, "Sink[secret Int]")
    made = f"made() -> {low} = {_wrap_val(w, 'Sink(eat: fn(x: Int) -> x)')}"
    decls = [header(MOD), SINK, BOX, made]
    if site == "let":
        fn = f"pub leak(pin: secret Int) -> Int\n=\n  let s: {high} = made()\n" + _unwrap(w, "s")
    elif site == "return":
        decls.append(f"lifted() -> {high} = made()")
        fn = "pub leak(pin: secret Int) -> Int\n=\n  let s = lifted()\n" + _unwrap(w, "s")
    elif site == "argument":
        body = _unwrap(w, "s")
        sig = f"feed_sink(s: {high}, pin: secret Int) -> Int"
        decls.append(f"{sig} = {body.strip()}" if "\n" not in body else f"{sig}\n=\n{body}")
        fn = "pub leak(pin: secret Int) -> Int = feed_sink(made(), pin: pin)"
    else:
        decls.append(f"kind Holder\n  s: {high}")
        fn = ("pub leak(pin: secret Int) -> Int\n=\n  let h = Holder(s: made())\n"
              + _unwrap(w, "h.s"))
    decls.append(fn)
    return _make("G1", edition, seed, MUST_REJECT, join_decls(*decls), ["U4001"],
                 variant=f"{w}-{site}")


# G2. The allowed directions: a covariant kind raised (`Box[Int]` into `Box[secret Int]`), and a
# sink that takes secrets used where public data is handed to it. Open, with the secret-taking
# sink's result kept `public` by not reading its argument.
def g2(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["box-up", "sink-down", "box-up-nested"])
    decls = [header(MOD), SINK, BOX]
    if form == "box-up":
        fn = ("pub raise(n: Int) -> secret Int\n=\n  let b: Box[secret Int] = Box(value: n)\n"
              "  b.value")
    elif form == "box-up-nested":
        fn = ("pub raise(n: Int) -> secret Int\n=\n"
              "  let b: Option[Box[secret Int]] = Some(Box(value: n))\n"
              "  when b\n    is Some(x) -> x.value\n    is None -> 0")
    else:
        decls.append("strong() -> Sink[secret Int] = Sink(eat: fn(x: secret Int) -> 0)")
        fn = ("pub feed(n: Int) -> Int\n=\n  let s = strong()\n  s.eat(n)")
    decls.append(fn)
    return _make("G2", edition, seed, OPEN, join_decls(*decls), variant=form,
                 note="expected accepted: rule 2 allows this direction")


# G3. Recursive kinds: the parameter reaches a function parameter at the second level, so rule 2's
# least solution makes the kind invariant, and a public instance does not become a secret one.
def g3(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["chain", "pair"])
    twin = rng.random() < 0.25
    if form == "chain":
        kinds = CHAIN
        mk = "Chain(next: None, eat: fn(x: Int) -> x)"
        low, high = "Chain[Int]", "Chain[secret Int]"
        use = "  s.eat(pin)"
    else:
        kinds = PAIR
        mk = "Outer(inner: Some(Inner(eat: fn(x: Int) -> x, back: None)))"
        low, high = "Outer[Int]", "Outer[secret Int]"
        use = "  when s.inner\n    is Some(i) -> i.eat(pin)\n    is None -> 0"
    if twin:
        fn = (f"pub same(pin: Int) -> Int\n=\n  let s: {low} = made()\n" + use)
        cls, expect = OPEN, []
    else:
        fn = (f"pub leak(pin: secret Int) -> Int\n=\n  let s: {high} = made()\n" + use)
        cls, expect = MUST_REJECT, ["U4001"]
    text = join_decls(header(MOD), kinds, f"made() -> {low} = {mk}", fn)
    return _make("G3", edition, seed, cls, text, expect,
                 variant=form + ("-twin" if twin else ""))


OPERATORS = {"G1": g1, "G2": g2, "G3": g3}
