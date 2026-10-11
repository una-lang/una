"""Family F: the covered fragment, for part (b) (docs/plan/security-hunt.md §2, generators §4 F).

Programs stay inside the importer's vocabulary (one `Console.print`, `Files`, lists with `map`,
`filter`, `fold` and `first_where`, lambdas, generic functions, records, `Option`, `Result`), so the
formal re-check reads them. A secret value travels through a carrier and reaches an ending. Every
program is open: the judge is the comparison of `una check` with the re-check (`differ` driver),
and a declaration `una check` accepts that the re-check rejects is a candidate for the
coordinator, never interpreted before that.
"""

from ..program import MUST_ACCEPT, MUST_REJECT, OPEN, RESIDUAL, Program, rng_for
from ..una import header, join_decls, package

MOD = "app.main"
RULE = "part (b): una check and the F1 re-check agree"

# Carriers: statements binding `t`, an `Int` holding the secret `s`, through one construct.
CARRIERS = {
    "direct": "  let t = s + 1\n",
    "map": ("  let ts = [s].map(each: fn(x) -> x + 1)\n"
            "  let t = when ts.first()\n    is Some(x) -> x\n    is None -> 0\n"),
    "filter": ("  let ts = [s, 1].filter(keep: fn(x) -> x > 0)\n"
               "  let t = when ts.first()\n    is Some(x) -> x\n    is None -> 0\n"),
    # The accumulator takes `initial`'s type: a public `0` would make `acc + x` U4001 (seed 5).
    "fold": ("  let z: secret Int = 0\n"
             "  let t = [s, 2].fold(initial: z, step: fn(acc, x) -> acc + x)\n"),
    "first-where": ("  let o = [s, 3].first_where(keep: fn(x) -> x > 1)\n"
                    "  let t = when o\n    is Some(x) -> x\n    is None -> 0\n"),
    "lambda": "  let f = fn(x: secret Int) -> x * 2\n  let t = f(s)\n",
    "generic": "  let t = same(s)\n",
    "record": "  let b = Holder(v: s)\n  let t = b.v\n",
    "option": "  let o = Some(s)\n  let t = when o\n    is Some(x) -> x\n    is None -> 0\n",
    "result": ("  let r: Result[secret Int, Int] = Ok(s)\n"
               "  let t = when r\n    is Ok(x) -> x\n    is Err(e) -> e\n"),
}

SUPPORT = {
    "generic": "same[T](x: T) -> T = x",
    "record": "kind Holder\n  v: secret Int",
}

# Endings: what is done with `t`; `una check` accepts the `secret` ones and rejects the others.
ENDINGS = {
    "secret-result": ("secret Int", "", "  t"),
    "secret-branch": ("secret Int", "", "  if t > 3 then 1 else 0"),
    "public-result": ("Int", "", "  t"),
    "print": ("Unit", "  uses @console: Console[print]\n", '  @console.print("{t}")'),
    "print-under": ("Unit", "  uses @console: Console[print]\n",
                    '  if t > 3\n    @console.print("big")\n  else\n    unit'),
    "write-internal": ("Unit", "  uses @files: Files[write]\n  fails FileError\n",
                       '  @files.write("out.txt", content: "{t}")\n    if error pass'),
}


# The endings that reveal `t` at `public` (Una-2's review of #547): a program whose `t` carries
# the secret must be rejected there, by `una check` alone, whatever the re-check says.
PUBLIC_ENDINGS = ("public-result", "print", "print-under")
LEAK_CODES = ["U4001", "U4003"]


def classify(carrier, ending, structural=()):
    """The class of a carrier and an ending: a public ending is must-reject, unless the carrier
    is structural, whose `t` is public, and then must-accept; any other ending is open. The
    programs' text does not depend on it, so earlier seeds rebuild unchanged."""
    if ending not in PUBLIC_ENDINGS:
        return OPEN, []
    if carrier in structural:
        return MUST_ACCEPT, []
    return MUST_REJECT, list(LEAK_CODES)


# F1 to F4: a secret literal, a carrier and an ending.
def f1(seed, edition):
    rng = rng_for(seed)
    carrier = rng.choice(sorted(CARRIERS))
    ending = rng.choice(sorted(ENDINGS))
    result, uses, tail = ENDINGS[ending]
    level = "internal" if ending == "write-internal" else "secret"
    body = CARRIERS[carrier].replace("secret Int", f"{level} Int")
    fn = (f"pub run(s: {level} Int) -> {result}\n" + uses + "=\n" + body + tail)
    decls = [header(MOD)]
    if carrier in SUPPORT:
        decls.append(SUPPORT[carrier].replace("secret Int", f"{level} Int"))
    decls.append(fn)
    files = package("hunt", edition, {MOD: join_decls(*decls)})
    cls, expect = classify(carrier, ending)
    return Program(op="F1", rule=RULE, edition=edition, seed=seed, cls=cls, files=files,
                   expect=expect, variant=f"{carrier}-{ending}",
                   note="the judge is the comparison of una check and the re-check")


# F7: carriers through the prelude functions F1 reads since #532, #542 and #545. A table of its
# own, so that F1's carriers, and the programs old seeds rebuild to, stay unchanged.
PRELUDE_CARRIERS = {
    "at": ("  let ts = [s, 1]\n"
           "  let t = when ts.at(index: 0)\n    is Some(x) -> x\n    is None -> 0\n"),
    # The index decides whether there is an element (`spec/14 §1`).
    "at-index": ("  let ts = [1, 2]\n"
                 "  let t = when ts.at(index: s)\n    is Some(x) -> x\n    is None -> 0\n"),
    "or-else": "  let o = Some(s)\n  let t = o.or_else(default: 0)\n",
    "or-else-default": "  let o: Option[Int] = None\n  let t = o.or_else(default: s)\n",
    # A call with two arguments labels the second (U2004, `spec/04 §3`).
    "min": "  let t = min(s, b: 5)\n",
    "max": "  let t = max(s, b: 5)\n",
    "abs": "  let t = abs(s)\n",
    # Structural: `is_ok` carries the result's top label only, so `t` is public here.
    "is-ok": ("  let r: Result[secret Int, Int] = Ok(s)\n"
              "  let t = if r.is_ok() then 1 else 0\n"),
    # The coordinator's counterpart: a result chosen under a secret condition, so its top label,
    # and `is_ok`'s `Bool`, are secret.
    "is-ok-top": ("  let r: secret Result[Int, Int] = if s > 0 then Ok(1) else Err(0)\n"
                  "  let t = if r.is_ok() then 1 else 0\n"),
    "slice": '  let tx = "{s}"\n  let t = tx.slice(start: 0, end: 0).len\n',
}

# Edition 0.1 has no `slice` (U1001): its text functions are edition 0.2's (`spec/22`).
EDITION_02_ONLY = {"slice"}


def f7(seed, edition):
    rng = rng_for(seed)
    carriers = [c for c in sorted(PRELUDE_CARRIERS) if edition != "0.1" or c not in EDITION_02_ONLY]
    carrier = rng.choice(carriers)
    ending = rng.choice(sorted(ENDINGS))
    result, uses, tail = ENDINGS[ending]
    level = "internal" if ending == "write-internal" else "secret"
    body = (PRELUDE_CARRIERS[carrier].replace("secret Int", f"{level} Int")
            .replace("secret Result", f"{level} Result"))
    fn = (f"pub run(s: {level} Int) -> {result}\n" + uses + "=\n" + body + tail)
    files = package("hunt", edition, {MOD: join_decls(header(MOD), fn)})
    cls, expect = classify(carrier, ending, structural=("is-ok",))
    return Program(op="F7", rule=RULE, edition=edition, seed=seed, cls=cls, files=files,
                   expect=expect, variant=f"{carrier}-{ending}",
                   note="the judge is the comparison of una check and the re-check")


# F8: carriers through fixed-width integers, which F1 reads since #548
# (`docs/plan/formal-track-fixed.md`); the coordinator's conditions are N-138's. Tables of their
# own, so that F1's and F7's programs, and the ones old seeds rebuild to, stay unchanged.
# Each carrier: the parameter's type, its `requires` (empty for none), the statements binding
# `t`, and `t`'s type. A `requires` bounds the secret so that no step can abort.
FIXED_CARRIERS = {
    "add": ("U8", "s < 100", "  let t: U8 = s + 1\n", "U8"),
    "sub": ("U8", "s < 100", "  let t: U8 = 200 - s\n", "U8"),
    "mul": ("U8", "s < 100", "  let t: U8 = s * 2\n", "U8"),
    "div": ("U8", "", "  let t: U8 = s / 3\n", "U8"),
    "mod": ("U8", "", "  let t: U8 = s mod 7\n", "U8"),
    "neg": ("I32", "s > -100 and s < 100", "  let t: I32 = -s\n", "I32"),
    "nested": ("U8", "s < 100", "  let t: U8 = (s + 1) * 2\n", "U8"),
    "narrow": ("Int", "0 <= s and s <= 255", "  let t = s.to_u8()\n", "U8"),
    "widen": ("U8", "", "  let t = s.to_i32()\n", "I32"),
    # `Int` arithmetic on a converted value, in a function with fixed-width values: read.
    "to-int": ("U8", "", "  let t = s.to_int() + 1\n", "Int"),
    # Structural: `t` does not depend on `s`, so a public ending is must-accept.
    "public": ("U8", "", "  let k: U8 = 40\n  let t: U8 = k + 2\n", "U8"),
}
# Arithmetic on a pattern's binder, whose type Core does not write: the re-check must say
# "not re-checked" (the fail-closed path of #548, condition 1), never "accepted".
FIXED_UNKNOWN = {
    "binder-option": ("U8", "", ("  let o = Some(s)\n"
                                  "  let t: U8 = when o\n    is Some(x) -> x / 2\n    is None -> 0\n"),
                      "U8"),
    "binder-result": ("U8", "", ("  let r: Result[secret U8, U8] = Ok(s)\n"
                                  "  let t: U8 = when r\n    is Ok(x) -> x / 2\n    is Err(e) -> e\n"),
                      "U8"),
}
# The wrapping, saturating and bit methods (Core's `fixed` primitive): outside F1.
FIXED_OUTSIDE = {
    "wrapping": ("U8", "", "  let t = s.wrapping_add(by: 1)\n", "U8"),
    "saturating": ("U8", "", "  let t = s.saturating_mul(by: 3)\n", "U8"),
    "bit": ("U8", "", "  let t = s.bit_and(mask: 15)\n", "U8"),
    "shift": ("U8", "", "  let t = s.shift_right(by: 1)\n", "U8"),
}
# The residual (N-132, N-138 condition 1): the secret decides whether a step aborts, a termination
# channel T6 does not claim. No `requires`: the obligation is where Una guards it.
FIXED_RESIDUAL = {
    "overflow": ("U8", "", "  let t: U8 = s + 200\n", "U8"),
    "narrow-abort": ("Int", "", "  let t = s.to_u8()\n", "U8"),
    "zero-div": ("U8", "", "  let t: U8 = 100 / s\n", "U8"),
    "neg-abort": ("U8", "", "  let t: U8 = -s\n", "U8"),
}
FIXED_TABLES = (("read", FIXED_CARRIERS), ("not-rechecked", FIXED_UNKNOWN),
                ("outside", FIXED_OUTSIDE), ("residual", FIXED_RESIDUAL))

CONSOLE = "  uses @console: Console[print]\n"


def fixed_ending(name, ty):
    """F8's endings, at `t`'s type `ty`: the result type, the `uses` line and the tail."""
    return {
        "secret-result": (f"secret {ty}", "", "  t"),
        "secret-branch": ("secret Int", "", "  if t > 3 then 1 else 0"),
        "public-result": (ty, "", "  t"),
        "print": ("Unit", CONSOLE, '  @console.print("{t}")'),
        "print-under": ("Unit", CONSOLE, '  if t > 3\n    @console.print("big")\n  else\n    unit'),
        # The residual's own: a public print that runs only if the step before it did not abort.
        "print-after": ("Unit", CONSOLE, '  @console.print("done")'),
    }[name]


FIXED_ENDINGS = ("secret-result", "secret-branch", "public-result", "print", "print-under")
RESIDUAL_ENDINGS = ("secret-result", "print-after")


def f8(seed, edition):
    rng = rng_for(seed)
    carriers = [(c, kind, table[c]) for kind, table in FIXED_TABLES for c in sorted(table)]
    carrier, recheck, (param, requires, body, ty) = rng.choice(carriers)
    ending = rng.choice(RESIDUAL_ENDINGS if recheck == "residual" else FIXED_ENDINGS)
    result, uses, tail = fixed_ending(ending, ty)
    clause = f"  requires {requires}\n" if requires else ""
    fn = f"pub run(s: secret {param}) -> {result}\n" + clause + uses + "=\n" + body + tail
    files = package("hunt", edition, {MOD: join_decls(header(MOD), fn)})
    note = "the judge is the comparison of una check and the re-check"
    if edition == "0.1":
        # Edition 0.1 has no fixed-width types (`spec/18`): `U8` is an ordinary, undeclared name.
        cls, expect, recheck = MUST_REJECT, ["U1001"], ""
        note = "edition 0.1 has no fixed-width types"
    elif recheck == "residual":
        cls, expect = RESIDUAL, []
        note = "a secret decides whether a step aborts: descriptive, never clean (N-138)"
    elif ending in PUBLIC_ENDINGS and carrier != "public":
        cls, expect = MUST_REJECT, list(LEAK_CODES)
    else:
        cls, expect = MUST_ACCEPT, []
    return Program(op="F8", rule=RULE, edition=edition, seed=seed, cls=cls, files=files,
                   expect=expect, variant=f"{carrier}-{ending}", note=note, recheck=recheck)


OPERATORS = {"F1": f1, "F7": f7, "F8": f8}
